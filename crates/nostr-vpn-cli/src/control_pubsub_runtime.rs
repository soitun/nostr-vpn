use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fips_core::FipsEndpoint;
use nostr_pubsub::{
    EventBus, EventSource, MatchEventOptions, MeshPeerPolicy, PolicyDecision, QueryEvent,
    VerifiedEvent,
};
use nostr_pubsub_fips::{
    FipsPubsubClient, FipsPubsubClientOptions, FipsPubsubPolicy, FipsPubsubPolicyOptions,
    FipsPubsubSubscription,
};
use nostr_pubsub_social_graph::{PEER_RATING_MAX_AGE, PEER_RATING_MAX_FUTURE_SKEW};
use nostr_sdk::prelude::{Client, Event, Filter, Keys, Kind, PublicKey, RelayPoolNotification};
use nostr_social_memory::rating_from_event;
use nostr_vpn_core::config::{NostrPubsubConfig, NostrPubsubMode};
use nostr_vpn_core::control_pubsub::{
    CONTROL_PUBSUB_MAX_EVENT_BYTES, CONTROL_PUBSUB_MAX_WIRE_BYTES, FIPS_PEER_ADVERT_KIND,
    PAID_EXIT_OFFER_KIND, RATING_FACT_KIND,
};
use nostr_vpn_core::updater::{UpdateEventCache, configured_update_ref};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

const STORE_VERSION: u8 = 1;
const STORE_MAX_EVENTS: usize = 1_024;
const COMMAND_CAPACITY: usize = 64;
const MAX_PUBSUB_PEERS: usize = 64;
const MAINTENANCE_TICK_INTERVAL: Duration = Duration::from_secs(1);
const OUTBOX_POLL_INTERVAL: Duration = Duration::from_secs(1);
const OUTBOX_BATCH: usize = 8;

struct PublishRequest {
    event: Box<Event>,
    response: oneshot::Sender<Result<bool>>,
}

include!("control_pubsub_runtime/event_store.rs");

pub struct ControlPubsubFipsRuntime {
    command_tx: mpsc::Sender<PublishRequest>,
    events: Arc<Mutex<ControlEventStore>>,
    relay_client: Option<Client>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ControlPubsubFipsRuntime {
    pub async fn start(
        endpoint: Arc<FipsEndpoint>,
        config: NostrPubsubConfig,
        relays: Vec<String>,
        store_path: Option<PathBuf>,
    ) -> Result<Option<Self>> {
        if config.mode == NostrPubsubMode::Off {
            return Ok(None);
        }
        Self::start_inner(endpoint, config, relays, store_path, None, None).await
    }

    pub async fn start_with_peer_policy(
        endpoint: Arc<FipsEndpoint>,
        config: NostrPubsubConfig,
        relays: Vec<String>,
        store_path: Option<PathBuf>,
        peer_policy: Arc<dyn MeshPeerPolicy>,
    ) -> Result<Option<Self>> {
        Self::start_inner(
            endpoint,
            config,
            relays,
            store_path,
            Some(peer_policy),
            None,
        )
        .await
    }

    async fn start_inner(
        endpoint: Arc<FipsEndpoint>,
        config: NostrPubsubConfig,
        relays: Vec<String>,
        store_path: Option<PathBuf>,
        peer_policy: Option<Arc<dyn MeshPeerPolicy>>,
        update_events_override: Option<UpdateEventCache>,
    ) -> Result<Option<Self>> {
        if config.mode == NostrPubsubMode::Off {
            return Ok(None);
        }
        let update_events = match update_events_override {
            Some(update_events) => update_events,
            None => configured_update_events()?,
        };
        let bridge = RelayBridge::start(config.mode, relays, &update_events).await?;
        let relay_client = bridge.as_ref().map(|bridge| bridge.client.clone());
        let outbox_path = store_path
            .as_deref()
            .map(control_pubsub_outbox_directory_from_store_path);
        let event_store = ControlEventStore::load(store_path, update_events.clone())?;
        let stored_events = event_store.snapshot();
        let pubsub_policy = FipsPubsubPolicy::new(
            Arc::clone(&endpoint),
            stored_events.iter(),
            FipsPubsubPolicyOptions::default(),
        )?;
        let peer_policy = peer_policy.unwrap_or_else(|| pubsub_policy.peer_policy());
        let pubsub_policy = Arc::new(Mutex::new(pubsub_policy));

        let max_event_bytes = config.max_event_bytes.min(CONTROL_PUBSUB_MAX_EVENT_BYTES);
        let fips_pubsub = FipsPubsubClient::start(
            Arc::clone(&endpoint),
            FipsPubsubClientOptions {
                max_frame_bytes: max_event_bytes
                    .saturating_add(4 * 1_024)
                    .min(CONTROL_PUBSUB_MAX_WIRE_BYTES),
                max_connected_peers: MAX_PUBSUB_PEERS,
                max_replay_events: STORE_MAX_EVENTS,
                receive_batch_size: 64,
                ..FipsPubsubClientOptions::default()
            },
        )
        .await
        .context("failed to bind standard FIPS Nostr pubsub service")?;

        let events = Arc::new(Mutex::new(event_store));
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task_events = Arc::clone(&events);
        let task = tokio::spawn(async move {
            run(
                PubsubRunState {
                    endpoint,
                    fips_pubsub,
                    bridge,
                    events: task_events,
                    peer_policy,
                    pubsub_policy,
                    update_events,
                },
                outbox_path,
                command_rx,
                shutdown_rx,
            )
            .await;
        });
        Ok(Some(Self {
            command_tx,
            events,
            relay_client,
            shutdown: Some(shutdown_tx),
            task,
        }))
    }

    pub async fn events(&self) -> Vec<Event> {
        self.events.lock().await.snapshot()
    }

    pub async fn publish(&self, event: Event) -> Result<bool> {
        let (response, result) = oneshot::channel();
        self.command_tx
            .send(PublishRequest {
                event: Box::new(event),
                response,
            })
            .await
            .context("control pubsub runtime stopped before publish")?;
        result
            .await
            .context("control pubsub runtime stopped while publishing")?
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = (&mut self.task).await;
        if let Some(client) = self.relay_client.take() {
            client.shutdown().await;
        }
    }
}

impl Drop for ControlPubsubFipsRuntime {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

struct RelayBridge {
    client: Client,
    notifications: tokio::sync::broadcast::Receiver<RelayPoolNotification>,
}

impl RelayBridge {
    async fn start(
        mode: NostrPubsubMode,
        relays: Vec<String>,
        update_events: &UpdateEventCache,
    ) -> Result<Option<Self>> {
        if mode != NostrPubsubMode::Relay || relays.is_empty() {
            return Ok(None);
        }
        let client = Client::new(Keys::generate());
        let notifications = client.notifications();
        for relay in &relays {
            client
                .add_relay(relay)
                .await
                .with_context(|| format!("failed to add control pubsub relay {relay}"))?;
        }
        client.connect().await;
        client
            .subscribe(Filter::new().kinds(control_kinds()), None)
            .await
            .context("failed to subscribe to control pubsub relays")?;
        client
            .subscribe(update_events.filter().clone(), None)
            .await
            .context("failed to subscribe to Hashtree update roots")?;
        Ok(Some(Self {
            client,
            notifications,
        }))
    }

    async fn publish(&self, event: &Event) {
        if let Err(error) = self.client.send_event(event).await {
            tracing::warn!(%error, event_id = %event.id, "failed to bridge control event to Nostr relays");
        }
    }
}

struct PubsubRunState {
    endpoint: Arc<FipsEndpoint>,
    fips_pubsub: FipsPubsubClient,
    bridge: Option<RelayBridge>,
    events: Arc<Mutex<ControlEventStore>>,
    peer_policy: Arc<dyn MeshPeerPolicy>,
    pubsub_policy: Arc<Mutex<FipsPubsubPolicy>>,
    update_events: UpdateEventCache,
}

#[derive(Clone, Copy)]
struct PublishContext<'a> {
    endpoint: &'a FipsEndpoint,
    fips_pubsub: &'a FipsPubsubClient,
    bridge: Option<&'a RelayBridge>,
    events: &'a Arc<Mutex<ControlEventStore>>,
    pubsub_policy: &'a Arc<Mutex<FipsPubsubPolicy>>,
    update_events: &'a UpdateEventCache,
}

async fn run(
    state: PubsubRunState,
    outbox_path: Option<PathBuf>,
    mut command_rx: mpsc::Receiver<PublishRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let PubsubRunState {
        endpoint,
        fips_pubsub,
        mut bridge,
        events,
        peer_policy,
        pubsub_policy,
        update_events,
    } = state;
    let mut maintenance_tick = tokio::time::interval(MAINTENANCE_TICK_INTERVAL);
    maintenance_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outbox_tick = tokio::time::interval(OUTBOX_POLL_INTERVAL);
    outbox_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut subscription = None;
    let mut subscribed_peer_links = Vec::new();
    sync_fips_subscription(
        &endpoint,
        &fips_pubsub,
        peer_policy.as_ref(),
        &events,
        &update_events,
        &mut subscription,
        &mut subscribed_peer_links,
    )
    .await;

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            command = command_rx.recv() => {
                let Some(PublishRequest { event, response }) = command else { break; };
                let result = publish_local(
                    PublishContext {
                        endpoint: &endpoint,
                        fips_pubsub: &fips_pubsub,
                        bridge: bridge.as_ref(),
                        events: &events,
                        pubsub_policy: &pubsub_policy,
                        update_events: &update_events,
                    },
                    *event,
                )
                .await;
                let _ = response.send(result);
            }
            _ = outbox_tick.tick(), if outbox_path.is_some() => {
                publish_outbox_batch(
                    PublishContext {
                        endpoint: &endpoint,
                        fips_pubsub: &fips_pubsub,
                        bridge: bridge.as_ref(),
                        events: &events,
                        pubsub_policy: &pubsub_policy,
                        update_events: &update_events,
                    },
                    outbox_path.as_deref().expect("outbox path is present"),
                )
                .await;
            }
            delivery = fips_notification(&mut subscription) => {
                let Some(delivery) = delivery else { continue; };
                process_fips_delivery(
                    &endpoint,
                    bridge.as_ref(),
                    &events,
                    peer_policy.as_ref(),
                    &pubsub_policy,
                    &update_events,
                    delivery,
                )
                .await;
            }
            notification = relay_notification(&mut bridge) => {
                let Some((relay_url, event)) = notification else { continue; };
                if !is_control_event(&event, &update_events)
                    || !event_is_admitted(
                        &pubsub_policy,
                        &event,
                        &EventSource::relay(relay_url),
                    )
                    .await
                {
                    continue;
                }
                if let Err(error) = publish_local(
                    PublishContext {
                        endpoint: &endpoint,
                        fips_pubsub: &fips_pubsub,
                        bridge: None,
                        events: &events,
                        pubsub_policy: &pubsub_policy,
                        update_events: &update_events,
                    },
                    event,
                )
                .await
                {
                    tracing::debug!(%error, "ignored invalid control event from relay");
                }
            }
            _ = maintenance_tick.tick() => {
                sync_fips_subscription(
                    &endpoint,
                    &fips_pubsub,
                    peer_policy.as_ref(),
                    &events,
                    &update_events,
                    &mut subscription,
                    &mut subscribed_peer_links,
                )
                .await;
                publish_policy_maintenance(
                    PublishContext {
                        endpoint: &endpoint,
                        fips_pubsub: &fips_pubsub,
                        bridge: bridge.as_ref(),
                        events: &events,
                        pubsub_policy: &pubsub_policy,
                        update_events: &update_events,
                    },
                )
                .await;
            }
        }
    }
}

async fn publish_local(context: PublishContext<'_>, event: Event) -> Result<bool> {
    if !is_control_event(&event, context.update_events) {
        anyhow::bail!("event is outside control pubsub subscriptions");
    }
    let event_bytes = serde_json::to_vec(&event)?;
    if event_bytes.len() > CONTROL_PUBSUB_MAX_EVENT_BYTES {
        anyhow::bail!(
            "control pubsub event is {} bytes, maximum is {}",
            event_bytes.len(),
            CONTROL_PUBSUB_MAX_EVENT_BYTES
        );
    }
    let verified = VerifiedEvent::try_from(event.clone())?;
    let fips_published = match context
        .fips_pubsub
        .publish(
            verified,
            EventSource::local_index(context.endpoint.npub().to_string()),
        )
        .await
    {
        Ok(report) => report.accepted,
        Err(error) => {
            tracing::debug!(%error, event_id = %event.id, "standard FIPS pubsub publication deferred");
            false
        }
    };
    tracing::debug!(event_id = %event.id, fips_published, "publishing local control event");
    observe_policy_event(context.pubsub_policy, &event).await;
    context.events.lock().await.insert(event.clone())?;
    ingest_into_fips_discovery(context.endpoint, &event).await;
    if let Some(bridge) = context.bridge {
        bridge.publish(&event).await;
    }
    Ok(fips_published || context.bridge.is_some())
}

async fn publish_policy_maintenance(context: PublishContext<'_>) {
    let now = now_ms();
    if let Err(error) = context
        .events
        .lock()
        .await
        .prune_expired_ratings(now / 1_000)
    {
        tracing::warn!(%error, "failed to prune expired control pubsub ratings");
    }
    let policy_events = match context
        .pubsub_policy
        .lock()
        .await
        .maintenance_events(now)
        .await
    {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "failed to evaluate pubsub policy maintenance");
            return;
        }
    };
    for event in policy_events {
        let published = match publish_local(context, event.clone()).await {
            Ok(published) => published,
            Err(error) => {
                tracing::warn!(%error, "failed to publish pubsub policy event");
                false
            }
        };
        if let Err(error) = context
            .pubsub_policy
            .lock()
            .await
            .complete_maintenance_event(&event, published, now)
        {
            tracing::warn!(%error, "failed to complete pubsub policy maintenance");
        }
    }
}

async fn publish_outbox_batch(context: PublishContext<'_>, outbox_path: &Path) {
    for path in control_pubsub_outbox_event_paths(outbox_path) {
        let event = match fs::read(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .and_then(|bytes| {
                serde_json::from_slice::<Event>(&bytes)
                    .with_context(|| format!("failed to decode {}", path.display()))
            }) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "discarding invalid control pubsub outbox entry");
                let _ = fs::remove_file(&path);
                continue;
            }
        };
        if let Err(error) = validate_control_pubsub_event(&event) {
            tracing::warn!(%error, path = %path.display(), "discarding rejected control pubsub outbox entry");
            let _ = fs::remove_file(&path);
            continue;
        }
        match publish_local(context, event).await {
            Ok(true) => {
                if let Err(error) = fs::remove_file(&path) {
                    tracing::warn!(%error, path = %path.display(), "failed to remove published control pubsub outbox entry");
                }
            }
            Ok(false) => break,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "discarding rejected control pubsub outbox entry");
                let _ = fs::remove_file(&path);
            }
        }
    }
}

async fn process_fips_delivery(
    endpoint: &FipsEndpoint,
    bridge: Option<&RelayBridge>,
    events: &Arc<Mutex<ControlEventStore>>,
    peer_policy: &dyn MeshPeerPolicy,
    pubsub_policy: &Arc<Mutex<FipsPubsubPolicy>>,
    update_events: &UpdateEventCache,
    delivery: QueryEvent,
) {
    let source_peer = delivery.source.id.as_str();
    if peer_policy
        .select_mesh_peer(source_peer)
        .ok()
        .flatten()
        .is_none()
    {
        tracing::debug!(%source_peer, "ignored control event from unselected FIPS pubsub peer");
        return;
    }
    let event = delivery.event.into_event();
    if !is_control_event(&event, update_events)
        || !event_is_admitted(pubsub_policy, &event, &delivery.source).await
    {
        return;
    }
    observe_policy_event(pubsub_policy, &event).await;
    ingest_into_fips_discovery(endpoint, &event).await;
    let inserted = match events.lock().await.insert(event.clone()) {
        Ok(inserted) => inserted,
        Err(error) => {
            tracing::warn!(%error, event_id = %event.id, "failed to store control pubsub event");
            false
        }
    };
    if inserted {
        tracing::debug!(event_id = %event.id, "delivered new standard FIPS pubsub event");
        if let Some(bridge) = bridge {
            bridge.publish(&event).await;
        }
    }
}

async fn observe_policy_event(policy: &Arc<Mutex<FipsPubsubPolicy>>, event: &Event) {
    if let Err(error) = policy.lock().await.observe_event(event) {
        tracing::warn!(%error, event_id = %event.id, "failed to observe pubsub policy event");
    }
}

async fn ingest_into_fips_discovery(endpoint: &FipsEndpoint, event: &Event) {
    if let Err(error) = endpoint.ingest_nostr_discovery_event(event.clone()).await {
        tracing::debug!(%error, event_id = %event.id, "failed to ingest pubsub event into FIPS discovery");
    }
}

async fn sync_fips_subscription(
    endpoint: &FipsEndpoint,
    fips_pubsub: &FipsPubsubClient,
    peer_policy: &dyn MeshPeerPolicy,
    events: &Arc<Mutex<ControlEventStore>>,
    update_events: &UpdateEventCache,
    subscription: &mut Option<FipsPubsubSubscription>,
    subscribed_peer_links: &mut Vec<(String, u64)>,
) {
    let peers = connected_peers(endpoint, peer_policy).await;
    if subscription.is_some() && *subscribed_peer_links == peers {
        return;
    }
    subscription.take();
    subscribed_peer_links.clear();
    if peers.is_empty() {
        return;
    }
    let mut filters = vec![Filter::new().kinds(control_kinds())];
    if update_events.filter().kinds.is_some() {
        filters.push(update_events.filter().clone());
    }
    let next = match fips_pubsub.subscribe(filters).await {
        Ok(subscription) => subscription,
        Err(error) => {
            tracing::debug!(%error, "standard FIPS pubsub subscription deferred");
            return;
        }
    };
    *subscribed_peer_links = peers;
    *subscription = Some(next);

    for event in events.lock().await.snapshot() {
        let event_id = event.id;
        let Ok(event) = VerifiedEvent::try_from(event) else {
            continue;
        };
        if let Err(error) = fips_pubsub
            .publish(event, EventSource::local_index(endpoint.npub().to_string()))
            .await
        {
            tracing::debug!(%error, %event_id, "stored FIPS pubsub replay deferred");
        }
    }
}

async fn connected_peers(
    endpoint: &FipsEndpoint,
    peer_policy: &dyn MeshPeerPolicy,
) -> Vec<(String, u64)> {
    let mut peers = endpoint
        .peers()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|peer| peer.connected)
        .filter_map(|peer| {
            peer_policy
                .select_mesh_peer(&peer.npub)
                .ok()
                .flatten()
                .map(|selected| (selected.id, peer.link_id))
        })
        .collect::<Vec<_>>();
    peers.sort();
    peers.dedup();
    peers
}

async fn fips_notification(
    subscription: &mut Option<FipsPubsubSubscription>,
) -> Option<QueryEvent> {
    let Some(subscription) = subscription.as_mut() else {
        return std::future::pending().await;
    };
    subscription.recv().await
}

async fn event_is_admitted(
    policy: &Arc<Mutex<FipsPubsubPolicy>>,
    event: &Event,
    source: &EventSource,
) -> bool {
    match policy.lock().await.check_event(event, source).await {
        Ok(PolicyDecision::Drop { reason }) => {
            tracing::debug!(
                event_id = %event.id,
                author = %event.pubkey,
                source = %source.id.as_str(),
                %reason,
                "dropped control pubsub event by author reputation"
            );
            false
        }
        Ok(PolicyDecision::Allow { .. } | PolicyDecision::Throttle { .. }) => true,
        Err(error) => {
            tracing::debug!(
                %error,
                event_id = %event.id,
                source = %source.id.as_str(),
                "ignored control pubsub event rejected by shared policy"
            );
            false
        }
    }
}

async fn relay_notification(bridge: &mut Option<RelayBridge>) -> Option<(String, Event)> {
    let Some(bridge) = bridge.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        match bridge.notifications.recv().await {
            Ok(RelayPoolNotification::Event {
                relay_url, event, ..
            }) => return Some((relay_url.to_string(), (*event).clone())),
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return std::future::pending().await;
            }
        }
    }
}

fn control_kinds() -> [Kind; 3] {
    [
        Kind::Custom(FIPS_PEER_ADVERT_KIND),
        Kind::Custom(PAID_EXIT_OFFER_KIND),
        Kind::Custom(RATING_FACT_KIND),
    ]
}

fn is_control_event(event: &Event, update_events: &UpdateEventCache) -> bool {
    matches!(
        u16::from(event.kind),
        FIPS_PEER_ADVERT_KIND | PAID_EXIT_OFFER_KIND | RATING_FACT_KIND
    ) || update_events
        .filter()
        .match_event(event, MatchEventOptions::new())
}

include!("control_pubsub_runtime/outbox.rs");

#[cfg(test)]
#[path = "control_pubsub_runtime/tests.rs"]
mod fips_update_tests;
