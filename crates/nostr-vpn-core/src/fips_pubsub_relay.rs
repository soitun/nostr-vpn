use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_endpoint::FipsEndpoint;
use nostr_pubsub::{EventBus, EventSource, QueryEvent, VerifiedEvent};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions, FipsPubsubSubscription};
use nostr_pubsub_relay::RelayEventBus;
use nostr_sdk::prelude::{Event, Filter, Kind, PublicKey, RelayPoolNotification};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

const RELAY_DRAIN_BATCH: usize = 64;
const RELAY_PENDING_EVENTS: usize = 1_024;
const RELAY_ACTIVE_PUMP_INTERVAL: Duration = Duration::from_millis(50);
const RELAY_IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(250);
const RELAY_PROVIDER_TIMEOUT: Duration = Duration::from_secs(2);
const RELAY_DIRECT_PUBLISH_CONCURRENCY: usize = 32;
const RELAY_SUBSCRIPTION_LIMIT: usize = 8;
const NOSTR_RELAY_TRANSPORT: &str = "nostr_relay";

/// Pumps FIPS's signed Nostr relay transport events through the standard
/// pubsub providers. Authenticated FIPS peers and configured direct relays are
/// simultaneous carriers; neither is selected as a fallback for the other.
/// Peers reached through the Nostr relay transport are excluded from the FIPS
/// pubsub side so the carrier cannot recursively wrap itself.
pub struct FipsPubsubNostrRelayAdapter {
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl FipsPubsubNostrRelayAdapter {
    pub async fn start(endpoint: Arc<FipsEndpoint>, relays: &[String]) -> Result<Self> {
        let local_pubkey =
            PublicKey::parse(endpoint.npub()).context("invalid local FIPS endpoint identity")?;
        let client = FipsPubsubClient::start_excluding_peer_transports(
            Arc::clone(&endpoint),
            FipsPubsubClientOptions::default(),
            [NOSTR_RELAY_TRANSPORT],
        )
        .await
        .context("failed to start standard FIPS Nostr pubsub provider")?;
        let filter = Filter::new()
            .kind(Kind::Custom(
                fips_core::transport::nostr_relay::NOSTR_RELAY_DATAGRAM_KIND,
            ))
            .pubkey(local_pubkey)
            .limit(RELAY_SUBSCRIPTION_LIMIT);
        let relay_provider = start_relay_provider(relays, filter.clone()).await?;
        let relay_notifications = relay_provider
            .as_ref()
            .map(|provider| provider.client().notifications());
        let source = EventSource::fips_endpoint(endpoint.npub().to_string());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_fips_pubsub_relay_adapter(
            endpoint,
            client,
            relay_provider,
            relay_notifications,
            filter,
            source,
            shutdown_rx,
        ));
        Ok(Self {
            shutdown: Some(shutdown_tx),
            task,
        })
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for FipsPubsubNostrRelayAdapter {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

async fn run_fips_pubsub_relay_adapter(
    endpoint: Arc<FipsEndpoint>,
    client: FipsPubsubClient,
    relay_provider: Option<RelayEventBus>,
    mut relay_notifications: Option<broadcast::Receiver<RelayPoolNotification>>,
    filter: Filter,
    source: EventSource,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut subscription = None;
    let mut pending = VecDeque::<VerifiedEvent>::new();
    let (relay_publish_tx, relay_publish_task) = relay_provider
        .as_ref()
        .map(|provider| {
            let (tx, rx) = tokio::sync::mpsc::channel(RELAY_PENDING_EVENTS);
            let provider = provider.clone();
            (
                Some(tx),
                Some(tokio::spawn(run_direct_relay_publisher(
                    provider,
                    source.clone(),
                    rx,
                ))),
            )
        })
        .unwrap_or((None, None));
    let pump = tokio::time::sleep(Duration::ZERO);
    tokio::pin!(pump);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            incoming = next_pubsub_event(&mut subscription) => {
                match incoming {
                    Some(incoming) => {
                        if let Err(error) = endpoint
                            .ingest_nostr_event(incoming.event.into_event())
                            .await
                        {
                            tracing::debug!(%error, "failed to ingest FIPS Nostr relay event from pubsub");
                        }
                    }
                    None => subscription = None,
                }
            }
            incoming = next_relay_event(&mut relay_notifications) => {
                if let Err(error) = endpoint.ingest_nostr_event(incoming).await {
                    tracing::debug!(%error, "failed to ingest FIPS Nostr relay event from direct relay pubsub");
                }
            }
            _ = &mut pump => {
                let mut did_work = !pending.is_empty();
                if subscription.is_none() {
                    match client.subscribe(vec![filter.clone()]).await {
                        Ok(active) => {
                            let peers = client.connected_peer_count().await.unwrap_or_default();
                            tracing::debug!(
                                peers,
                                "FIPS Nostr relay pubsub carrier connected"
                            );
                            subscription = Some(active);
                        }
                        Err(error) => {
                            tracing::trace!(%error, "FIPS Nostr relay pubsub carrier is waiting for an authenticated peer");
                        }
                    }
                }

                match endpoint.drain_nostr_relay_events(RELAY_DRAIN_BATCH).await {
                    Ok(events) => {
                        did_work |= !events.is_empty();
                        for event in events {
                            let event = match VerifiedEvent::try_from(event) {
                                Ok(event) => event,
                                Err(error) => {
                                    tracing::debug!(%error, "discarded invalid local FIPS Nostr relay event");
                                    continue;
                                }
                            };
                            if let Some(tx) = relay_publish_tx.as_ref()
                                && let Err(error) = tx.try_send(event.clone())
                            {
                                tracing::debug!(%error, "direct Nostr pubsub provider queue is saturated");
                            }
                            if pending.len() == RELAY_PENDING_EVENTS {
                                pending.pop_front();
                                tracing::debug!("authenticated FIPS pubsub provider queue is saturated");
                            }
                            pending.push_back(event);
                        }
                    }
                    Err(error) => tracing::debug!(%error, "failed to drain FIPS Nostr relay events"),
                }

                publish_pending_fips_events(&client, &source, &mut pending).await;
                pump.as_mut().reset(
                    tokio::time::Instant::now()
                        + relay_pump_interval(did_work, !pending.is_empty()),
                );
            }
        }
    }

    client.shutdown().await;
    drop(relay_publish_tx);
    if let Some(task) = relay_publish_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(provider) = relay_provider {
        provider.client().shutdown().await;
    }
}

async fn publish_pending_fips_events<B: EventBus>(
    provider: &B,
    source: &EventSource,
    pending: &mut VecDeque<VerifiedEvent>,
) {
    let attempts = pending.len().min(RELAY_DRAIN_BATCH);
    for _ in 0..attempts {
        let Some(event) = pending.pop_front() else {
            break;
        };
        let accepted = match provider.publish(event.clone(), source.clone()).await {
            Ok(report) => report.accepted,
            Err(error) => {
                tracing::trace!(%error, "FIPS Nostr relay publication is waiting for an authenticated pubsub peer");
                false
            }
        };
        if !accepted {
            pending.push_back(event);
        }
    }
}

async fn start_relay_provider(relays: &[String], filter: Filter) -> Result<Option<RelayEventBus>> {
    if relays.is_empty() {
        return Ok(None);
    }
    let provider = RelayEventBus::new(relays.iter().cloned(), RELAY_PROVIDER_TIMEOUT)
        .await
        .map_err(|error| anyhow!("failed to start direct Nostr pubsub provider: {error}"))?;
    provider
        .client()
        .subscribe(filter, None)
        .await
        .context("failed to subscribe direct Nostr pubsub provider to FIPS relay events")?;
    Ok(Some(provider))
}

async fn run_direct_relay_publisher<B>(
    provider: B,
    source: EventSource,
    mut events: tokio::sync::mpsc::Receiver<VerifiedEvent>,
) where
    B: EventBus + Clone + Send + Sync + 'static,
{
    let mut input_closed = false;
    let mut in_flight = tokio::task::JoinSet::new();

    while !input_closed || !in_flight.is_empty() {
        tokio::select! {
            event = events.recv(), if !input_closed && in_flight.len() < RELAY_DIRECT_PUBLISH_CONCURRENCY => {
                match event {
                    Some(event) => {
                        let provider = provider.clone();
                        let source = source.clone();
                        in_flight.spawn(async move {
                            publish_direct_relay_event(provider, source, event).await;
                        });
                    }
                    None => input_closed = true,
                }
            }
            completed = in_flight.join_next(), if !in_flight.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::debug!(%error, "direct Nostr pubsub publisher task failed");
                }
            }
        }
    }
}

async fn publish_direct_relay_event<B: EventBus>(
    provider: B,
    source: EventSource,
    event: VerifiedEvent,
) {
    match tokio::time::timeout(RELAY_PROVIDER_TIMEOUT, provider.publish(event, source)).await {
        Ok(Ok(report)) if report.accepted => {}
        Ok(Ok(report)) => {
            tracing::trace!(
                reason = report.reason.as_deref().unwrap_or("unspecified"),
                "direct Nostr pubsub provider rejected FIPS relay event"
            );
        }
        Ok(Err(error)) => {
            tracing::trace!(%error, "direct Nostr pubsub provider did not publish FIPS relay event");
        }
        Err(_) => {
            tracing::trace!("direct Nostr pubsub provider timed out publishing FIPS relay event");
        }
    }
}

fn relay_pump_interval(did_work: bool, has_pending: bool) -> Duration {
    if did_work || has_pending {
        RELAY_ACTIVE_PUMP_INTERVAL
    } else {
        RELAY_IDLE_PUMP_INTERVAL
    }
}

async fn next_pubsub_event(
    subscription: &mut Option<FipsPubsubSubscription>,
) -> Option<QueryEvent> {
    match subscription {
        Some(subscription) => subscription.recv().await,
        None => std::future::pending().await,
    }
}

async fn next_relay_event(
    notifications: &mut Option<broadcast::Receiver<RelayPoolNotification>>,
) -> Event {
    let Some(notifications) = notifications else {
        return std::future::pending().await;
    };
    loop {
        match notifications.recv().await {
            Ok(RelayPoolNotification::Event { event, .. })
                if event.kind
                    == Kind::Custom(
                        fips_core::transport::nostr_relay::NOSTR_RELAY_DATAGRAM_KIND,
                    ) =>
            {
                return (*event).clone();
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                return std::future::pending().await;
            }
        }
    }
}

#[cfg(test)]
mod fips_pubsub_relay_adapter_tests {
    use super::*;
    use crate::fips_control::{FipsControlFrame, JoinRosterControl, NetworkRoster, SignedRoster};
    use crate::fips_control_tcp::FipsControlTcpRuntime;
    use async_trait::async_trait;
    use fips_core::config::{IdentityConfig, NostrRelayConfig, PeerConfig, TransportInstances};
    use fips_core::{
        Config, FipsEndpoint, Identity, SimNetwork, SimTransportConfig, register_sim_network,
        unregister_sim_network,
    };
    use fips_endpoint::PeerIdentity;
    use nostr_pubsub::{
        EventBus, PublishReport, QueryOptions, QueryReport, Result as PubsubResult,
    };
    use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};
    use nostr_sdk::prelude::{EventBuilder, Filter, Keys, Kind, PublicKey};
    use std::sync::Mutex;
    use tokio::time::timeout;

    #[test]
    fn relay_pump_slows_only_when_idle() {
        assert_eq!(relay_pump_interval(false, false), RELAY_IDLE_PUMP_INTERVAL);
        assert_eq!(relay_pump_interval(true, false), RELAY_ACTIVE_PUMP_INTERVAL);
        assert_eq!(relay_pump_interval(false, true), RELAY_ACTIVE_PUMP_INTERVAL);
    }

    #[derive(Clone)]
    struct FirstPublishRejected {
        first_event_id: String,
        attempted: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EventBus for FirstPublishRejected {
        async fn publish(
            &self,
            event: VerifiedEvent,
            _source: EventSource,
        ) -> PubsubResult<PublishReport> {
            let event_id = event.as_event().id.to_hex();
            self.attempted
                .lock()
                .expect("attempted event lock")
                .push(event_id.clone());
            if event_id == self.first_event_id {
                Ok(PublishReport {
                    accepted: false,
                    priority: 0,
                    reason: Some("relay did not acknowledge the event".to_string()),
                })
            } else {
                Ok(PublishReport {
                    accepted: true,
                    priority: 0,
                    reason: None,
                })
            }
        }

        async fn query(
            &self,
            _filters: Vec<Filter>,
            _options: QueryOptions,
        ) -> PubsubResult<QueryReport> {
            Ok(QueryReport::default())
        }
    }

    #[derive(Clone)]
    struct BlockingPublishes {
        blocked_event_ids: Arc<Vec<String>>,
        attempted: Arc<Mutex<Vec<String>>>,
        completed: Arc<Mutex<Vec<String>>>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl EventBus for BlockingPublishes {
        async fn publish(
            &self,
            event: VerifiedEvent,
            _source: EventSource,
        ) -> PubsubResult<PublishReport> {
            let event_id = event.as_event().id.to_hex();
            self.attempted
                .lock()
                .expect("attempted event lock")
                .push(event_id.clone());
            if self.blocked_event_ids.contains(&event_id) {
                self.release.notified().await;
            }
            self.completed
                .lock()
                .expect("completed event lock")
                .push(event_id);
            Ok(PublishReport {
                accepted: true,
                priority: 0,
                reason: None,
            })
        }

        async fn query(
            &self,
            _filters: Vec<Filter>,
            _options: QueryOptions,
        ) -> PubsubResult<QueryReport> {
            Ok(QueryReport::default())
        }
    }

    #[tokio::test]
    async fn rejected_direct_relay_publication_does_not_block_the_next_datagram() {
        let keys = Keys::generate();
        let first = VerifiedEvent::try_from(
            EventBuilder::new(Kind::Custom(21_060), "first")
                .sign_with_keys(&keys)
                .expect("sign first event"),
        )
        .expect("verify first event");
        let second = VerifiedEvent::try_from(
            EventBuilder::new(Kind::Custom(21_060), "second")
                .sign_with_keys(&keys)
                .expect("sign second event"),
        )
        .expect("verify second event");
        let first_event_id = first.as_event().id.to_hex();
        let second_event_id = second.as_event().id.to_hex();
        let attempted = Arc::new(Mutex::new(Vec::new()));
        let provider = FirstPublishRejected {
            first_event_id: first_event_id.clone(),
            attempted: Arc::clone(&attempted),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(first).await.expect("queue first event");
        tx.send(second).await.expect("queue second event");
        drop(tx);

        timeout(
            Duration::from_millis(200),
            run_direct_relay_publisher(provider, EventSource::local_index("test"), rx),
        )
        .await
        .expect("publisher must advance after a stalled event");

        assert_eq!(
            attempted.lock().expect("attempted event lock").as_slice(),
            [first_event_id, second_event_id]
        );
    }

    #[tokio::test]
    async fn slow_direct_relay_publications_do_not_serialize_later_datagrams() {
        const BLOCKED_EVENTS: usize = 25;
        let keys = Keys::generate();
        let events = (0..=BLOCKED_EVENTS)
            .map(|index| {
                VerifiedEvent::try_from(
                    EventBuilder::new(Kind::Custom(21_060), format!("event-{index}"))
                        .sign_with_keys(&keys)
                        .expect("sign relay event"),
                )
                .expect("verify relay event")
            })
            .collect::<Vec<_>>();
        let event_ids = events
            .iter()
            .map(|event| event.as_event().id.to_hex())
            .collect::<Vec<_>>();
        let last_event_id = event_ids.last().expect("later relay event").clone();
        let attempted = Arc::new(Mutex::new(Vec::new()));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let release = Arc::new(tokio::sync::Notify::new());
        let provider = BlockingPublishes {
            blocked_event_ids: Arc::new(event_ids[..BLOCKED_EVENTS].to_vec()),
            attempted: Arc::clone(&attempted),
            completed: Arc::clone(&completed),
            release: Arc::clone(&release),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(events.len());
        for event in events {
            tx.send(event).await.expect("queue relay event");
        }
        drop(tx);
        let publisher = tokio::spawn(run_direct_relay_publisher(
            provider,
            EventSource::local_index("test"),
            rx,
        ));

        timeout(Duration::from_millis(500), async {
            loop {
                if completed
                    .lock()
                    .expect("completed event lock")
                    .contains(&last_event_id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("later relay event must not wait for 25 slow provider attempts");
        assert_eq!(
            attempted.lock().expect("attempted event lock").len(),
            BLOCKED_EVENTS + 1
        );

        release.notify_waiters();
        timeout(Duration::from_millis(500), publisher)
            .await
            .expect("publisher should drain released attempts")
            .expect("publisher task");
    }

    #[tokio::test]
    async fn rejected_fips_pubsub_publication_rotates_behind_newer_datagrams() {
        let keys = Keys::generate();
        let first = VerifiedEvent::try_from(
            EventBuilder::new(Kind::Custom(21_060), "first")
                .sign_with_keys(&keys)
                .expect("sign first event"),
        )
        .expect("verify first event");
        let second = VerifiedEvent::try_from(
            EventBuilder::new(Kind::Custom(21_060), "second")
                .sign_with_keys(&keys)
                .expect("sign second event"),
        )
        .expect("verify second event");
        let first_event_id = first.as_event().id.to_hex();
        let second_event_id = second.as_event().id.to_hex();
        let attempted = Arc::new(Mutex::new(Vec::new()));
        let provider = FirstPublishRejected {
            first_event_id: first_event_id.clone(),
            attempted: Arc::clone(&attempted),
        };
        let mut pending = VecDeque::from([first, second]);

        publish_pending_fips_events(&provider, &EventSource::local_index("test"), &mut pending)
            .await;

        assert_eq!(
            attempted.lock().expect("attempted event lock").as_slice(),
            [first_event_id.clone(), second_event_id]
        );
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending
                .front()
                .expect("rejected event remains queued")
                .as_event()
                .id
                .to_hex(),
            first_event_id
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn standard_join_and_roster_cross_the_fips_pubsub_relay_carrier() {
        let network = format!("nvpn-standard-join-pubsub-{}", std::process::id());
        register_sim_network(&network, SimNetwork::new(7_368));

        let guest_identity = Identity::from_secret_bytes(&[11; 32]).expect("guest identity");
        let provider_identity = Identity::from_secret_bytes(&[12; 32]).expect("provider identity");
        let admin_identity = Identity::from_secret_bytes(&[13; 32]).expect("admin identity");
        let guest = bind_test_endpoint(test_guest_config(&network, &provider_identity)).await;
        let provider = bind_test_endpoint(test_provider_config(
            &network,
            &provider_identity,
            &guest_identity,
        ))
        .await;
        let admin = bind_test_endpoint(test_admin_config(&admin_identity, &guest_identity)).await;
        wait_for_sim_peer(&guest, provider.npub()).await;
        wait_for_sim_peer(&provider, guest.npub()).await;

        let provider_client =
            FipsPubsubClient::start(Arc::clone(&provider), FipsPubsubClientOptions::default())
                .await
                .expect("provider pubsub service");
        let guest_adapter = FipsPubsubNostrRelayAdapter::start(Arc::clone(&guest), &[])
            .await
            .expect("guest relay carrier");
        wait_for_peer_subscription(&provider_client).await;
        let admin_pubkey = PublicKey::parse(admin.npub()).expect("admin pubkey");
        let admin_subscription = provider_client
            .subscribe(vec![
                Filter::new()
                    .kind(Kind::Custom(
                        fips_core::transport::nostr_relay::NOSTR_RELAY_DATAGRAM_KIND,
                    ))
                    .pubkey(admin_pubkey),
            ])
            .await
            .expect("subscribe for admin relay events");
        let (relay_shutdown_tx, relay_shutdown_rx) = oneshot::channel();
        let relay_task = tokio::spawn(run_test_relay(
            provider_client,
            admin_subscription,
            Arc::clone(&admin),
            relay_shutdown_rx,
        ));

        let mut guest_control = FipsControlTcpRuntime::start(Arc::clone(&guest))
            .await
            .expect("guest control");
        let admin_control = FipsControlTcpRuntime::start(Arc::clone(&admin))
            .await
            .expect("admin control");
        let admin_keys = Keys::parse(&hex::encode([13; 32])).expect("admin keys");
        let guest_keys = Keys::parse(&hex::encode([11; 32])).expect("guest keys");
        let roster = SignedRoster::sign(
            "ordinary-network",
            NetworkRoster {
                network_name: "Ordinary network".to_string(),
                devices: vec![
                    admin_keys.public_key().to_hex(),
                    guest_keys.public_key().to_hex(),
                ],
                admins: vec![admin_keys.public_key().to_hex()],
                aliases: Default::default(),
                signed_at: 1_778_998_001,
            },
            &admin_keys,
        )
        .expect("signed roster");
        let roster_frame = FipsControlFrame::JoinRoster {
            control: Box::new(
                JoinRosterControl::new(roster, "ordinary-request-secret")
                    .expect("join roster control"),
            ),
        };
        let admin_sender = admin_control.sender();
        let guest_peer = PeerIdentity::from_npub(guest.npub()).expect("guest peer");
        let roster_send =
            tokio::spawn(async move { admin_sender.send(guest_peer, &roster_frame).await });
        let received = timeout(Duration::from_secs(10), guest_control.recv())
            .await
            .expect("guest roster timeout")
            .expect("guest control remains active");
        assert_eq!(received.source_peer.npub(), admin.npub());
        assert!(matches!(
            received.frame,
            FipsControlFrame::JoinRoster { .. }
        ));
        roster_send
            .await
            .expect("roster send task")
            .expect("join roster acknowledged");

        guest_control.stop().await;
        admin_control.stop().await;
        let _ = relay_shutdown_tx.send(());
        let _ = relay_task.await;
        guest_adapter.stop().await;
        guest.shutdown().await.expect("guest shutdown");
        provider.shutdown().await.expect("provider shutdown");
        admin.shutdown().await.expect("admin shutdown");
        unregister_sim_network(&network);
    }

    async fn bind_test_endpoint(config: Config) -> Arc<FipsEndpoint> {
        Arc::new(
            Box::pin(
                FipsEndpoint::builder()
                    .config(config)
                    .without_system_tun()
                    .bind(),
            )
            .await
            .expect("bind test endpoint"),
        )
    }

    fn test_base_config(secret: [u8; 32]) -> Config {
        let mut config = Config::new();
        config.node.identity = IdentityConfig {
            nsec: Some(hex::encode(secret)),
            persistent: false,
        };
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.lan.enabled = false;
        config.node.discovery.local.enabled = false;
        config.node.retry.base_interval_secs = 1;
        config.node.retry.max_backoff_secs = 1;
        config.node.rate_limit.handshake_burst = 1_000;
        config.node.rate_limit.handshake_rate = 1_000.0;
        config
    }

    fn test_guest_config(network: &str, provider: &Identity) -> Config {
        let mut config = test_base_config([11; 32]);
        config.transports.sim = TransportInstances::Single(SimTransportConfig {
            network: Some(network.to_string()),
            addr: Some("guest".to_string()),
            mtu: Some(1_280),
            auto_connect: Some(false),
            accept_connections: Some(true),
        });
        config.transports.nostr_relay = TransportInstances::Single(NostrRelayConfig {
            auto_connect: Some(false),
            accept_connections: Some(true),
            ..NostrRelayConfig::default()
        });
        // The ordinary device-approval join request does not know which admin
        // will approve it. Exercise the fresh authenticated relay handshake
        // instead of pre-seeding the admin in the guest's endpoint peer index.
        config.peers = vec![PeerConfig::new(provider.npub(), "sim", "provider")];
        config
    }

    fn test_provider_config(network: &str, provider: &Identity, guest: &Identity) -> Config {
        let _ = provider;
        let mut config = test_base_config([12; 32]);
        config.transports.sim = TransportInstances::Single(SimTransportConfig {
            network: Some(network.to_string()),
            addr: Some("provider".to_string()),
            mtu: Some(1_280),
            auto_connect: Some(false),
            accept_connections: Some(true),
        });
        config.peers = vec![PeerConfig::new(guest.npub(), "sim", "guest")];
        config
    }

    fn test_admin_config(admin: &Identity, guest: &Identity) -> Config {
        let _ = admin;
        let mut config = test_base_config([13; 32]);
        config.transports.nostr_relay = TransportInstances::Single(NostrRelayConfig {
            auto_connect: Some(false),
            accept_connections: Some(false),
            ..NostrRelayConfig::default()
        });
        config.peers = vec![PeerConfig::new(guest.npub(), "nostr_relay", guest.npub())];
        config
    }

    async fn wait_for_sim_peer(endpoint: &FipsEndpoint, expected: &str) {
        timeout(Duration::from_secs(5), async {
            loop {
                if endpoint
                    .peers()
                    .await
                    .expect("peer snapshot")
                    .into_iter()
                    .any(|peer| {
                        peer.connected
                            && peer.npub == expected
                            && peer.transport_type.as_deref() == Some("sim")
                    })
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("sim peers connect");
    }

    async fn wait_for_peer_subscription(client: &FipsPubsubClient) {
        timeout(Duration::from_secs(5), async {
            loop {
                if client
                    .peer_subscription_count()
                    .expect("peer subscriptions")
                    > 0
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("guest subscribes to provider");
    }

    async fn run_test_relay(
        client: FipsPubsubClient,
        mut admin_subscription: FipsPubsubSubscription,
        admin: Arc<FipsEndpoint>,
        mut shutdown: oneshot::Receiver<()>,
    ) {
        // Preserve bidirectional relay progress while modeling ordinary
        // non-zero transit latency for the cold first-roster exchange.
        const FORWARD_DELAY: Duration = Duration::from_millis(250);
        let mut to_admin = VecDeque::new();
        let mut to_guest = VecDeque::new();
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                incoming = admin_subscription.recv() => {
                    let Some(incoming) = incoming else { break; };
                    to_admin.push_back((
                        tokio::time::Instant::now() + FORWARD_DELAY,
                        incoming.event.into_event(),
                    ));
                }
                _ = tick.tick() => {
                    for event in admin
                        .drain_nostr_relay_events(RELAY_DRAIN_BATCH)
                        .await
                        .expect("drain admin relay events")
                    {
                        to_guest.push_back((
                            tokio::time::Instant::now() + FORWARD_DELAY,
                            VerifiedEvent::try_from(event).expect("verified admin event"),
                        ));
                    }
                    let now = tokio::time::Instant::now();
                    while to_admin.front().is_some_and(|(ready_at, _)| *ready_at <= now) {
                        let (_, event) = to_admin.pop_front().expect("ready admin event");
                        admin
                            .ingest_nostr_event(event)
                            .await
                            .expect("ingest event for admin");
                    }
                    while to_guest.front().is_some_and(|(ready_at, _)| *ready_at <= now) {
                        let (_, event) = to_guest.pop_front().expect("ready guest event");
                        client
                            .publish(
                                event,
                                EventSource::fips_endpoint(admin.npub().to_string()),
                            )
                            .await
                            .expect("publish admin relay event");
                    }
                }
            }
        }
        client.shutdown().await;
    }
}
