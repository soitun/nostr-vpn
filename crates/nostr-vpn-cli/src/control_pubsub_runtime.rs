use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fips_core::FipsEndpoint;
use nostr_pubsub::{EventSource, MatchEventOptions, MeshPeer, MeshPeerPolicy, PolicyDecision};
use nostr_pubsub_fips::{FipsPubsubPolicy, FipsPubsubPolicyOptions};
use nostr_pubsub_social_graph::{PEER_RATING_MAX_AGE, PEER_RATING_MAX_FUTURE_SKEW};
use nostr_sdk::prelude::{Client, Event, Filter, Keys, Kind, PublicKey, RelayPoolNotification};
use nostr_social_memory::rating_from_event;
use nostr_vpn_core::config::{NostrPubsubConfig, NostrPubsubMode};
use nostr_vpn_core::control_pubsub::{
    CONTROL_PUBSUB_FIPS_SERVICE_PORT, CONTROL_PUBSUB_MAX_EVENT_BYTES,
    CONTROL_PUBSUB_MAX_WIRE_BYTES, ControlPubsubAction, ControlPubsubCodec, ControlPubsubMesh,
    ControlPubsubOptions, ControlPubsubWireMessage, FIPS_PEER_ADVERT_KIND, PAID_EXIT_OFFER_KIND,
    RATING_FACT_KIND,
};
use nostr_vpn_core::updater::{UpdateEventCache, configured_update_ref};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::fips_tcp_records::{FipsTcpRecordEvent, FipsTcpRecordSender, FipsTcpRecordTransport};

const STORE_VERSION: u8 = 1;
const STORE_MAX_EVENTS: usize = 1_024;
const COMMAND_CAPACITY: usize = 64;
const MAINTENANCE_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const OUTBOX_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
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
        mut peer_policy: Option<Arc<dyn MeshPeerPolicy>>,
        update_events_override: Option<UpdateEventCache>,
    ) -> Result<Option<Self>> {
        if config.mode == NostrPubsubMode::Off {
            return Ok(None);
        }
        let transport = FipsTcpRecordTransport::bind(
            Arc::clone(&endpoint),
            CONTROL_PUBSUB_FIPS_SERVICE_PORT,
            CONTROL_PUBSUB_MAX_WIRE_BYTES,
        )
        .await
        .context("failed to bind reliable FIPS control pubsub transport")?;
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
        if peer_policy.is_none() {
            peer_policy = Some(pubsub_policy.peer_policy());
        }
        let events = Arc::new(Mutex::new(event_store));
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task_events = Arc::clone(&events);
        let task = tokio::spawn(async move {
            run(
                PubsubRunState {
                    endpoint,
                    config,
                    bridge,
                    events: task_events,
                    peer_policy,
                    pubsub_policy,
                    update_events,
                },
                transport,
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
    config: NostrPubsubConfig,
    bridge: Option<RelayBridge>,
    events: Arc<Mutex<ControlEventStore>>,
    peer_policy: Option<Arc<dyn MeshPeerPolicy>>,
    pubsub_policy: FipsPubsubPolicy,
    update_events: UpdateEventCache,
}

#[derive(Clone, Copy)]
struct PublishContext<'a> {
    endpoint: &'a FipsEndpoint,
    transport: &'a FipsTcpRecordSender,
    codec: &'a ControlPubsubCodec,
    bridge: Option<&'a RelayBridge>,
    events: &'a Arc<Mutex<ControlEventStore>>,
    peer_policy: Option<&'a dyn MeshPeerPolicy>,
    update_events: &'a UpdateEventCache,
}

async fn run(
    state: PubsubRunState,
    mut transport: FipsTcpRecordTransport,
    outbox_path: Option<PathBuf>,
    mut command_rx: mpsc::Receiver<PublishRequest>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let PubsubRunState {
        endpoint,
        config,
        mut bridge,
        events,
        peer_policy,
        mut pubsub_policy,
        update_events,
    } = state;
    let max_event_bytes = config.max_event_bytes.min(CONTROL_PUBSUB_MAX_EVENT_BYTES);
    let mut options = ControlPubsubOptions {
        fanout: config.fanout,
        max_hops: config.max_hops,
        max_event_bytes,
        ..ControlPubsubOptions::default()
    };
    if let Some(kinds) = update_events.filter().kinds.as_ref() {
        options
            .allowed_kinds
            .extend(kinds.iter().map(|kind| u16::from(*kind)));
    }
    let mut mesh = ControlPubsubMesh::new(options);
    let codec = ControlPubsubCodec::new(CONTROL_PUBSUB_MAX_WIRE_BYTES);
    let transport_sender = transport.sender();
    let mut replayed_update_roots = HashMap::<String, String>::new();
    let mut maintenance_tick = tokio::time::interval(MAINTENANCE_TICK_INTERVAL);
    maintenance_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outbox_tick = tokio::time::interval(OUTBOX_POLL_INTERVAL);
    outbox_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            command = command_rx.recv() => {
                let Some(PublishRequest { event, response }) = command else { break; };
                let result = publish_local(
                    PublishContext {
                        endpoint: &endpoint,
                        transport: &transport_sender,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                        update_events: &update_events,
                    },
                    &mut mesh,
                    Some(&mut pubsub_policy),
                    *event,
                )
                .await;
                let _ = response.send(result);
            }
            _ = outbox_tick.tick(), if outbox_path.is_some() => {
                publish_outbox_batch(
                    PublishContext {
                        endpoint: &endpoint,
                        transport: &transport_sender,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                        update_events: &update_events,
                    },
                    &mut mesh,
                    &mut pubsub_policy,
                    outbox_path.as_deref().expect("outbox path is present"),
                )
                .await;
            }
            received = transport.recv() => {
                let Some(received) = received else { break; };
                let (source, payload) = match received {
                    FipsTcpRecordEvent::Connected { peer } => {
                        replayed_update_roots.remove(&peer);
                        tracing::debug!(%peer, "control pubsub TCP/FIPS session established; resubscribing cached state");
                        continue;
                    }
                    FipsTcpRecordEvent::Record { source_peer, payload } => (source_peer, payload),
                };
                let peers = connected_peers(&endpoint, peer_policy.as_deref()).await;
                {
                    if !peer_is_accepted(&source, peer_policy.as_deref()) {
                        tracing::debug!(%source, "dropped control pubsub stream record by peer reputation");
                        continue;
                    }
                    let message = match codec.decode(&payload) {
                        Ok(message) => message,
                        Err(error) => {
                            mesh.record_invalid_peer_message(&source);
                            tracing::debug!(%error, %source, "ignored invalid control pubsub stream record");
                            continue;
                        }
                    };
                    tracing::debug!(%source, message = ?message, "received control pubsub message");
                    if let ControlPubsubWireMessage::Frame { event_id, event } = &message {
                        if let Err(error) = event.verify() {
                            mesh.record_invalid_peer_message(&source);
                            tracing::debug!(%error, %source, "ignored invalid signed control pubsub event");
                            continue;
                        }
                        if !is_control_event(event, &update_events) {
                            mesh.record_invalid_peer_message(&source);
                            mesh.dismiss_peer_frame(&source, event_id);
                            tracing::debug!(%source, event_id, "ignored event outside control pubsub subscriptions");
                            continue;
                        }
                        if !event_is_admitted(
                            &pubsub_policy,
                            event,
                            &EventSource::fips_endpoint(source.clone()),
                        )
                        .await
                        {
                            mesh.dismiss_peer_frame(&source, event_id);
                            continue;
                        }
                    }
                    match mesh.receive(&source, message.clone(), &peers, now_ms()) {
                        Ok(actions) => {
                            execute_actions(
                                &endpoint,
                                &transport_sender,
                                &codec,
                                bridge.as_ref(),
                                &events,
                                Some(&mut pubsub_policy),
                                actions,
                            )
                            .await;
                        }
                        Err(error) => tracing::debug!(%error, %source, "ignored invalid control pubsub message"),
                    }
                }
            }
            notification = relay_notification(&mut bridge) => {
                let Some((relay_url, event)) = notification else { continue; };
                if !is_control_event(&event, &update_events) {
                    continue;
                }
                if !event_is_admitted(
                    &pubsub_policy,
                    &event,
                    &EventSource::relay(relay_url),
                )
                .await
                {
                    continue;
                }
                let peers = connected_peers(&endpoint, peer_policy.as_deref()).await;
                match mesh.publish(event.clone(), &peers, now_ms()) {
                    Ok(actions) => {
                        observe_policy_event(&mut pubsub_policy, &event);
                        ingest_into_fips_discovery(&endpoint, &event).await;
                        if let Err(error) = events.lock().await.insert(event) {
                            tracing::warn!(%error, "failed to store control event from relay");
                        }
                        execute_actions(
                            &endpoint,
                            &transport_sender,
                            &codec,
                            None,
                            &events,
                            Some(&mut pubsub_policy),
                            actions,
                        )
                        .await;
                    }
                    Err(error) => tracing::debug!(%error, "ignored invalid control event from relay"),
                }
            }
            _ = maintenance_tick.tick() => {
                publish_policy_maintenance(
                    PublishContext {
                        endpoint: &endpoint,
                        transport: &transport_sender,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                        update_events: &update_events,
                    },
                    &mut mesh,
                    &mut pubsub_policy,
                )
                .await;
                replay_update_root_to_connected_peers(
                    PublishContext {
                        endpoint: &endpoint,
                        transport: &transport_sender,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                        update_events: &update_events,
                    },
                    &mut mesh,
                    &mut replayed_update_roots,
                )
                .await;
            }
        }
    }
    transport.stop().await;
}

async fn replay_update_root_to_connected_peers(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    replayed: &mut HashMap<String, String>,
) {
    let PublishContext {
        endpoint,
        transport,
        codec,
        events,
        peer_policy,
        ..
    } = context;
    let peers = connected_peers(endpoint, peer_policy).await;
    let connected = peers
        .iter()
        .map(|peer| peer.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    replayed.retain(|peer_id, _| connected.contains(peer_id.as_str()));

    let event = events.lock().await.latest_update_event();
    let Some(event) = event else {
        return;
    };
    let event_id = event.id.to_hex();
    let mut actions = Vec::new();
    for peer in peers {
        if replayed.get(&peer.id) == Some(&event_id) {
            continue;
        }
        match mesh.replay_to_peer(event.clone(), &peer.id, now_ms()) {
            Ok(mut replay) => {
                actions.append(&mut replay);
                replayed.insert(peer.id, event_id.clone());
            }
            Err(error) => {
                tracing::warn!(%error, event_id, "failed to prepare cached update-root replay");
            }
        }
    }
    execute_actions(endpoint, transport, codec, None, events, None, actions).await;
}

async fn publish_local(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    mut pubsub_policy: Option<&mut FipsPubsubPolicy>,
    event: Event,
) -> Result<bool> {
    let PublishContext {
        endpoint,
        transport,
        codec,
        bridge,
        events,
        peer_policy,
        update_events,
    } = context;
    if !is_control_event(&event, update_events) {
        anyhow::bail!("event is outside control pubsub subscriptions");
    }
    let peers = connected_peers(endpoint, peer_policy).await;
    if peers.is_empty() && bridge.is_none() {
        return Ok(false);
    }
    let actions = mesh.publish(event.clone(), &peers, now_ms())?;
    tracing::debug!(event_id = %event.id, peers = peers.len(), actions = actions.len(), "publishing local control event");
    if let Some(policy) = pubsub_policy.as_deref_mut() {
        observe_policy_event(policy, &event);
    }
    events.lock().await.insert(event.clone())?;
    ingest_into_fips_discovery(endpoint, &event).await;
    if let Some(bridge) = bridge {
        bridge.publish(&event).await;
    }
    execute_actions(
        endpoint,
        transport,
        codec,
        None,
        events,
        pubsub_policy,
        actions,
    )
    .await;
    Ok(true)
}

async fn publish_policy_maintenance(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    pubsub_policy: &mut FipsPubsubPolicy,
) {
    let now = now_ms();
    if let Err(error) = context
        .events
        .lock()
        .await
        .prune_expired_ratings(now / 1_000)
    {
        tracing::warn!(%error, "failed to prune expired control pubsub ratings");
    }
    let events = match pubsub_policy.maintenance_events(now).await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "failed to evaluate pubsub policy maintenance");
            return;
        }
    };
    for event in events {
        let published = match publish_local(context, mesh, None, event.clone()).await {
            Ok(published) => published,
            Err(error) => {
                tracing::warn!(%error, "failed to publish pubsub policy event");
                false
            }
        };
        if let Err(error) = pubsub_policy.complete_maintenance_event(&event, published, now) {
            tracing::warn!(%error, "failed to complete pubsub policy maintenance");
        }
    }
}

async fn publish_outbox_batch(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    pubsub_policy: &mut FipsPubsubPolicy,
    outbox_path: &Path,
) {
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
        match publish_local(context, mesh, Some(&mut *pubsub_policy), event).await {
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

async fn execute_actions(
    endpoint: &FipsEndpoint,
    transport: &FipsTcpRecordSender,
    codec: &ControlPubsubCodec,
    bridge: Option<&RelayBridge>,
    events: &Arc<Mutex<ControlEventStore>>,
    mut pubsub_policy: Option<&mut FipsPubsubPolicy>,
    actions: Vec<ControlPubsubAction>,
) {
    let mut outbound = Vec::new();
    for action in actions {
        match action {
            ControlPubsubAction::Send { peer_id, message } => {
                outbound.push((peer_id, message));
            }
            ControlPubsubAction::Deliver { event, .. } => {
                if let Some(policy) = pubsub_policy.as_deref_mut() {
                    observe_policy_event(policy, &event);
                }
                ingest_into_fips_discovery(endpoint, &event).await;
                let inserted = match events.lock().await.insert(event.clone()) {
                    Ok(inserted) => inserted,
                    Err(error) => {
                        tracing::warn!(%error, event_id = %event.id, "failed to store control pubsub event");
                        false
                    }
                };
                if inserted {
                    tracing::debug!(event_id = %event.id, "delivered new control pubsub event");
                    if let Some(bridge) = bridge {
                        bridge.publish(&event).await;
                    }
                }
            }
        }
    }

    send_control_messages(transport, codec, outbound).await;
}

fn observe_policy_event(policy: &mut FipsPubsubPolicy, event: &Event) {
    if let Err(error) = policy.observe_event(event) {
        tracing::warn!(%error, event_id = %event.id, "failed to observe pubsub policy event");
    }
}

async fn send_control_messages(
    transport: &FipsTcpRecordSender,
    codec: &ControlPubsubCodec,
    messages: Vec<(String, ControlPubsubWireMessage)>,
) {
    for (peer_id, message) in messages {
        let payload = match codec.encode(&message) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, %peer_id, "failed to encode control pubsub message");
                continue;
            }
        };
        if let Err(error) = transport.send(&peer_id, payload).await {
            tracing::debug!(%error, %peer_id, "failed to queue control pubsub TCP/FIPS record");
        } else {
            tracing::debug!(%peer_id, "queued control pubsub TCP/FIPS record");
        }
    }
}

async fn ingest_into_fips_discovery(endpoint: &FipsEndpoint, event: &Event) {
    if let Err(error) = endpoint.ingest_nostr_discovery_event(event.clone()).await {
        tracing::debug!(%error, event_id = %event.id, "failed to ingest pubsub event into FIPS discovery");
    }
}

async fn connected_peers(
    endpoint: &FipsEndpoint,
    peer_policy: Option<&dyn MeshPeerPolicy>,
) -> Vec<MeshPeer> {
    let mut peers = endpoint
        .peers()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|peer| peer.connected)
        .filter_map(|peer| select_mesh_peer(&peer.npub, peer_policy))
        .collect::<Vec<_>>();
    peers.sort_by(|left, right| left.id.cmp(&right.id));
    peers.dedup_by(|left, right| left.id == right.id);
    peers
}

fn peer_is_accepted(peer_id: &str, peer_policy: Option<&dyn MeshPeerPolicy>) -> bool {
    select_mesh_peer(peer_id, peer_policy).is_some()
}

fn select_mesh_peer(peer_id: &str, peer_policy: Option<&dyn MeshPeerPolicy>) -> Option<MeshPeer> {
    let Some(peer_policy) = peer_policy else {
        return Some(MeshPeer::new(peer_id));
    };
    match peer_policy.select_mesh_peer(peer_id) {
        Ok(peer) => peer,
        Err(error) => {
            tracing::warn!(%error, %peer_id, "peer reputation failed; treating peer as unknown");
            Some(MeshPeer::new(peer_id))
        }
    }
}

async fn event_is_admitted(policy: &FipsPubsubPolicy, event: &Event, source: &EventSource) -> bool {
    match policy.check_event(event, source).await {
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
