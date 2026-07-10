use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fips_core::{
    FipsEndpoint, FipsEndpointOutboundDatagram, FipsEndpointServiceReceiver, PeerIdentity,
};
use nostr_pubsub::{EventSource, MeshPeer, MeshPeerPolicy, PolicyDecision};
use nostr_pubsub_fips::{FipsPubsubPolicy, FipsPubsubPolicyOptions};
use nostr_sdk::prelude::{Client, Event, Filter, Keys, Kind, RelayPoolNotification};
use nostr_vpn_core::config::{NostrPubsubConfig, NostrPubsubMode};
use nostr_vpn_core::control_pubsub::{
    CONTROL_PUBSUB_FIPS_SERVICE_PORT, CONTROL_PUBSUB_MAX_EVENT_BYTES,
    CONTROL_PUBSUB_MAX_WIRE_BYTES, ControlPubsubAction, ControlPubsubCodec, ControlPubsubMesh,
    ControlPubsubOptions, ControlPubsubWireMessage, FIPS_PEER_ADVERT_KIND, PAID_EXIT_OFFER_KIND,
    RATING_FACT_KIND,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

const STORE_VERSION: u8 = 1;
const STORE_MAX_EVENTS: usize = 1_024;
const COMMAND_CAPACITY: usize = 64;
const RECEIVE_BATCH: usize = 64;
const RETRY_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const RETRY_BASE_INTERVAL_MS: u64 = 250;
const RETRY_MAX_ATTEMPTS: u8 = 3;
const MAX_PENDING_RETRIES: usize = 1_024;
const MAX_PENDING_RETRIES_PER_PEER: usize = 64;
const OUTBOX_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const OUTBOX_BATCH: usize = 8;

enum RuntimeCommand {
    Stop,
    Publish {
        event: Box<Event>,
        response: oneshot::Sender<Result<bool>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RetryKind {
    Inventory,
    Want,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RetryKey {
    peer_id: String,
    event_id: String,
    kind: RetryKind,
}

#[derive(Debug, Clone)]
struct PendingRetry {
    message: ControlPubsubWireMessage,
    attempts: u8,
    next_attempt_ms: u64,
}

#[derive(Debug, Default)]
struct RetryState {
    pending: HashMap<RetryKey, PendingRetry>,
}

impl RetryState {
    fn track(&mut self, peer_id: &str, message: &ControlPubsubWireMessage, now_ms: u64) {
        let Some(key) = retry_key(peer_id, message) else {
            return;
        };
        if self.pending.contains_key(&key)
            || self.pending.len() >= MAX_PENDING_RETRIES
            || self
                .pending
                .keys()
                .filter(|candidate| candidate.peer_id == peer_id)
                .count()
                >= MAX_PENDING_RETRIES_PER_PEER
        {
            return;
        }
        self.pending.insert(
            key,
            PendingRetry {
                message: message.clone(),
                attempts: 1,
                next_attempt_ms: now_ms.saturating_add(RETRY_BASE_INTERVAL_MS),
            },
        );
    }

    fn acknowledge(&mut self, source_peer: &str, message: &ControlPubsubWireMessage) {
        let acknowledged = match message {
            ControlPubsubWireMessage::Want { event_id } => Some(RetryKey {
                peer_id: source_peer.to_string(),
                event_id: event_id.clone(),
                kind: RetryKind::Inventory,
            }),
            ControlPubsubWireMessage::Frame { event_id, .. } => Some(RetryKey {
                peer_id: source_peer.to_string(),
                event_id: event_id.clone(),
                kind: RetryKind::Want,
            }),
            ControlPubsubWireMessage::Inventory { .. } => None,
        };
        if let Some(key) = acknowledged {
            self.pending.remove(&key);
        }
    }

    fn due(&mut self, now_ms: u64) -> Vec<(String, ControlPubsubWireMessage)> {
        let due = self
            .pending
            .iter()
            .filter(|(_, retry)| retry.next_attempt_ms <= now_ms)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut outbound = Vec::new();
        for key in due {
            let Some(retry) = self.pending.get_mut(&key) else {
                continue;
            };
            if retry.attempts >= RETRY_MAX_ATTEMPTS {
                self.pending.remove(&key);
                continue;
            }
            retry.attempts += 1;
            let shift = u32::from(retry.attempts.saturating_sub(2));
            let backoff = RETRY_BASE_INTERVAL_MS.saturating_mul(1_u64 << shift);
            retry.next_attempt_ms = now_ms.saturating_add(backoff);
            outbound.push((key.peer_id, retry.message.clone()));
        }
        outbound
    }
}

fn retry_key(peer_id: &str, message: &ControlPubsubWireMessage) -> Option<RetryKey> {
    let (event_id, kind) = match message {
        ControlPubsubWireMessage::Inventory { event_id, .. } => {
            (event_id.clone(), RetryKind::Inventory)
        }
        ControlPubsubWireMessage::Want { event_id } => (event_id.clone(), RetryKind::Want),
        ControlPubsubWireMessage::Frame { .. } => return None,
    };
    Some(RetryKey {
        peer_id: peer_id.to_string(),
        event_id,
        kind,
    })
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredEventsFile {
    version: u8,
    events: Vec<Event>,
}

#[derive(Debug)]
struct ControlEventStore {
    path: Option<PathBuf>,
    events: HashMap<String, Event>,
    order: VecDeque<String>,
}

impl ControlEventStore {
    fn load(path: Option<PathBuf>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self {
                path: None,
                events: HashMap::new(),
                order: VecDeque::new(),
            });
        };
        let mut store = Self {
            path: Some(path.clone()),
            events: HashMap::new(),
            order: VecDeque::new(),
        };
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let saved: StoredEventsFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        if saved.version != STORE_VERSION {
            return Err(anyhow!(
                "unsupported control pubsub store version {} in {}",
                saved.version,
                path.display()
            ));
        }
        for event in saved.events {
            if event.verify().is_ok() && is_control_kind(u16::from(event.kind)) {
                store.insert_memory(event);
            }
        }
        Ok(store)
    }

    fn insert(&mut self, event: Event) -> Result<bool> {
        if self.events.contains_key(&event.id.to_hex()) {
            return Ok(false);
        }
        self.insert_memory(event);
        self.persist()?;
        Ok(true)
    }

    fn insert_memory(&mut self, event: Event) {
        let event_id = event.id.to_hex();
        if self.events.contains_key(&event_id) {
            return;
        }
        while self.events.len() >= STORE_MAX_EVENTS {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.events.remove(&oldest);
        }
        self.order.push_back(event_id.clone());
        self.events.insert(event_id, event);
    }

    fn snapshot(&self) -> Vec<Event> {
        self.order
            .iter()
            .filter_map(|event_id| self.events.get(event_id).cloned())
            .collect()
    }

    fn persist(&self) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let saved = StoredEventsFile {
            version: STORE_VERSION,
            events: self.snapshot(),
        };
        let bytes = serde_json::to_vec(&saved).context("failed to encode control pubsub store")?;
        let temporary = temporary_store_path(path);
        fs::write(&temporary, bytes)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        fs::rename(&temporary, path).with_context(|| {
            format!(
                "failed to replace control pubsub store {} with {}",
                path.display(),
                temporary.display()
            )
        })?;
        Ok(())
    }
}

pub struct ControlPubsubFipsRuntime {
    command_tx: mpsc::Sender<RuntimeCommand>,
    events: Arc<Mutex<ControlEventStore>>,
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
        Self::start_inner(endpoint, config, relays, store_path, None).await
    }

    pub async fn start_with_peer_policy(
        endpoint: Arc<FipsEndpoint>,
        config: NostrPubsubConfig,
        relays: Vec<String>,
        store_path: Option<PathBuf>,
        peer_policy: Arc<dyn MeshPeerPolicy>,
    ) -> Result<Option<Self>> {
        Self::start_inner(endpoint, config, relays, store_path, Some(peer_policy)).await
    }

    async fn start_inner(
        endpoint: Arc<FipsEndpoint>,
        config: NostrPubsubConfig,
        relays: Vec<String>,
        store_path: Option<PathBuf>,
        mut peer_policy: Option<Arc<dyn MeshPeerPolicy>>,
    ) -> Result<Option<Self>> {
        if config.mode == NostrPubsubMode::Off {
            return Ok(None);
        }
        let receiver = endpoint
            .register_service_receiver(CONTROL_PUBSUB_FIPS_SERVICE_PORT)
            .await
            .context("failed to register the FIPS control pubsub service")?;
        let bridge = RelayBridge::start(config.mode, relays).await?;
        let outbox_path = store_path
            .as_deref()
            .map(control_pubsub_outbox_directory_from_store_path);
        let event_store = ControlEventStore::load(store_path)?;
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
                },
                receiver,
                outbox_path,
                command_rx,
            )
            .await;
        });
        Ok(Some(Self {
            command_tx,
            events,
            task,
        }))
    }

    pub async fn events(&self) -> Vec<Event> {
        self.events.lock().await.snapshot()
    }

    pub async fn publish(&self, event: Event) -> Result<bool> {
        let (response, result) = oneshot::channel();
        self.command_tx
            .send(RuntimeCommand::Publish {
                event: Box::new(event),
                response,
            })
            .await
            .context("control pubsub runtime stopped before publish")?;
        result
            .await
            .context("control pubsub runtime stopped while publishing")?
    }

    pub async fn stop(self) {
        let _ = self.command_tx.send(RuntimeCommand::Stop).await;
        let _ = self.task.await;
    }
}

struct RelayBridge {
    client: Client,
    notifications: tokio::sync::broadcast::Receiver<RelayPoolNotification>,
}

impl RelayBridge {
    async fn start(mode: NostrPubsubMode, relays: Vec<String>) -> Result<Option<Self>> {
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

    async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

struct PubsubRunState {
    endpoint: Arc<FipsEndpoint>,
    config: NostrPubsubConfig,
    bridge: Option<RelayBridge>,
    events: Arc<Mutex<ControlEventStore>>,
    peer_policy: Option<Arc<dyn MeshPeerPolicy>>,
    pubsub_policy: FipsPubsubPolicy,
}

#[derive(Clone, Copy)]
struct PublishContext<'a> {
    endpoint: &'a FipsEndpoint,
    codec: &'a ControlPubsubCodec,
    bridge: Option<&'a RelayBridge>,
    events: &'a Arc<Mutex<ControlEventStore>>,
    peer_policy: Option<&'a dyn MeshPeerPolicy>,
}

async fn run(
    state: PubsubRunState,
    receiver: FipsEndpointServiceReceiver,
    outbox_path: Option<PathBuf>,
    mut command_rx: mpsc::Receiver<RuntimeCommand>,
) {
    let PubsubRunState {
        endpoint,
        config,
        mut bridge,
        events,
        peer_policy,
        mut pubsub_policy,
    } = state;
    let max_event_bytes = config.max_event_bytes.min(CONTROL_PUBSUB_MAX_EVENT_BYTES);
    let options = ControlPubsubOptions {
        fanout: config.fanout,
        max_hops: config.max_hops,
        max_event_bytes,
        ..ControlPubsubOptions::default()
    };
    let mut mesh = ControlPubsubMesh::new(options);
    let codec = ControlPubsubCodec::new(CONTROL_PUBSUB_MAX_WIRE_BYTES);
    let mut datagrams = Vec::with_capacity(RECEIVE_BATCH);
    let mut retries = RetryState::default();
    let mut retry_tick = tokio::time::interval(RETRY_TICK_INTERVAL);
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outbox_tick = tokio::time::interval(OUTBOX_POLL_INTERVAL);
    outbox_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                match command {
                    Some(RuntimeCommand::Stop) | None => break,
                    Some(RuntimeCommand::Publish { event, response }) => {
                        let result = publish_local(
                            PublishContext {
                                endpoint: &endpoint,
                                codec: &codec,
                                bridge: bridge.as_ref(),
                                events: &events,
                                peer_policy: peer_policy.as_deref(),
                            },
                            &mut mesh,
                            &mut retries,
                            Some(&mut pubsub_policy),
                            *event,
                        )
                        .await;
                        let _ = response.send(result);
                    }
                }
            }
            _ = outbox_tick.tick(), if outbox_path.is_some() => {
                publish_outbox_batch(
                    PublishContext {
                        endpoint: &endpoint,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                    },
                    &mut mesh,
                    &mut retries,
                    &mut pubsub_policy,
                    outbox_path.as_deref().expect("outbox path is present"),
                )
                .await;
            }
            count = receiver.recv_batch_into(&mut datagrams, RECEIVE_BATCH) => {
                let Some(count) = count else { break; };
                let peers = connected_peers(&endpoint, peer_policy.as_deref()).await;
                for datagram in datagrams.iter().take(count) {
                    let source = datagram.source_peer.npub().to_string();
                    if !peer_is_accepted(&source, peer_policy.as_deref()) {
                        tracing::debug!(%source, "dropped control pubsub datagram by peer reputation");
                        continue;
                    }
                    let message = match codec.decode(datagram.data.as_ref()) {
                        Ok(message) => message,
                        Err(error) => {
                            mesh.record_invalid_peer_message(&source);
                            tracing::debug!(%error, %source, "ignored invalid control pubsub datagram");
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
                            retries.acknowledge(&source, &message);
                            execute_actions(
                                &endpoint,
                                &codec,
                                bridge.as_ref(),
                                &events,
                                &mut retries,
                                Some(&mut pubsub_policy),
                                actions,
                            )
                            .await;
                        }
                        Err(error) => tracing::debug!(%error, %source, "ignored invalid control pubsub message"),
                    }
                }
                datagrams.clear();
            }
            notification = relay_notification(&mut bridge) => {
                let Some((relay_url, event)) = notification else { continue; };
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
                            &codec,
                            None,
                            &events,
                            &mut retries,
                            Some(&mut pubsub_policy),
                            actions,
                        )
                        .await;
                    }
                    Err(error) => tracing::debug!(%error, "ignored invalid control event from relay"),
                }
            }
            _ = retry_tick.tick() => {
                send_control_messages(&endpoint, &codec, retries.due(now_ms())).await;
                publish_policy_maintenance(
                    PublishContext {
                        endpoint: &endpoint,
                        codec: &codec,
                        bridge: bridge.as_ref(),
                        events: &events,
                        peer_policy: peer_policy.as_deref(),
                    },
                    &mut mesh,
                    &mut retries,
                    &mut pubsub_policy,
                )
                .await;
            }
        }
    }
    if let Some(bridge) = bridge.as_ref() {
        bridge.shutdown().await;
    }
}

async fn publish_local(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    retries: &mut RetryState,
    mut pubsub_policy: Option<&mut FipsPubsubPolicy>,
    event: Event,
) -> Result<bool> {
    let PublishContext {
        endpoint,
        codec,
        bridge,
        events,
        peer_policy,
    } = context;
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
        codec,
        None,
        events,
        retries,
        pubsub_policy,
        actions,
    )
    .await;
    Ok(true)
}

async fn publish_policy_maintenance(
    context: PublishContext<'_>,
    mesh: &mut ControlPubsubMesh,
    retries: &mut RetryState,
    pubsub_policy: &mut FipsPubsubPolicy,
) {
    let now = now_ms();
    let events = match pubsub_policy.maintenance_events(now).await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "failed to evaluate pubsub policy maintenance");
            return;
        }
    };
    for event in events {
        let published = match publish_local(context, mesh, retries, None, event.clone()).await {
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
    retries: &mut RetryState,
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
        match publish_local(context, mesh, retries, Some(&mut *pubsub_policy), event).await {
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
    codec: &ControlPubsubCodec,
    bridge: Option<&RelayBridge>,
    events: &Arc<Mutex<ControlEventStore>>,
    retries: &mut RetryState,
    mut pubsub_policy: Option<&mut FipsPubsubPolicy>,
    actions: Vec<ControlPubsubAction>,
) {
    let mut outbound = Vec::new();
    for action in actions {
        match action {
            ControlPubsubAction::Send { peer_id, message } => {
                retries.track(&peer_id, &message, now_ms());
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

    send_control_messages(endpoint, codec, outbound).await;
}

fn observe_policy_event(policy: &mut FipsPubsubPolicy, event: &Event) {
    if let Err(error) = policy.observe_event(event) {
        tracing::warn!(%error, event_id = %event.id, "failed to observe pubsub policy event");
    }
}

async fn send_control_messages(
    endpoint: &FipsEndpoint,
    codec: &ControlPubsubCodec,
    messages: Vec<(String, ControlPubsubWireMessage)>,
) {
    let mut outbound = HashMap::<String, Vec<FipsEndpointOutboundDatagram>>::new();
    for (peer_id, message) in messages {
        let payload = match codec.encode(&message) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, %peer_id, "failed to encode control pubsub message");
                continue;
            }
        };
        outbound
            .entry(peer_id)
            .or_default()
            .push(FipsEndpointOutboundDatagram::new(
                CONTROL_PUBSUB_FIPS_SERVICE_PORT,
                CONTROL_PUBSUB_FIPS_SERVICE_PORT,
                payload,
            ));
    }

    for (peer_id, datagrams) in outbound {
        let remote = match PeerIdentity::from_npub(&peer_id) {
            Ok(remote) => remote,
            Err(error) => {
                tracing::debug!(%error, %peer_id, "ignored invalid control pubsub peer");
                continue;
            }
        };
        let count = datagrams.len();
        if let Err(error) = endpoint
            .send_datagram_batch_to_peer(remote, datagrams)
            .await
        {
            tracing::debug!(%error, %peer_id, count, "failed to send control pubsub datagram batch");
        } else {
            tracing::debug!(%peer_id, count, "sent control pubsub datagram batch");
        }
    }
}

async fn ingest_into_fips_discovery(endpoint: &FipsEndpoint, event: &Event) {
    if let Err(error) = endpoint.ingest_nostr_pubsub_event(event.clone()).await {
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

fn is_control_kind(kind: u16) -> bool {
    [
        FIPS_PEER_ADVERT_KIND,
        PAID_EXIT_OFFER_KIND,
        RATING_FACT_KIND,
    ]
    .contains(&kind)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn temporary_store_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "control-pubsub-events.json".into());
    name.push(".tmp");
    path.with_file_name(name)
}

pub fn control_pubsub_store_file_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("control-pubsub-events.json")
}

fn control_pubsub_outbox_directory_from_store_path(store_path: &Path) -> PathBuf {
    store_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("control-pubsub-outbox")
}

pub fn control_pubsub_outbox_directory(config_path: &Path) -> PathBuf {
    control_pubsub_outbox_directory_from_store_path(&control_pubsub_store_file_path(config_path))
}

pub fn queue_control_pubsub_event(config_path: &Path, event: &Event) -> Result<bool> {
    validate_control_pubsub_event(event)?;
    let bytes = serde_json::to_vec(event).context("failed to encode control pubsub event")?;

    let directory = control_pubsub_outbox_directory(config_path);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let destination = directory.join(format!("{}.json", event.id.to_hex()));
    if destination.exists() {
        return Ok(false);
    }
    let temporary = directory.join(format!(
        ".{}.{}-{}.tmp",
        event.id.to_hex(),
        std::process::id(),
        now_ms()
    ));
    fs::write(&temporary, bytes)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to queue {}", destination.display()));
    }
    Ok(true)
}

fn validate_control_pubsub_event(event: &Event) -> Result<()> {
    event
        .verify()
        .map_err(|error| anyhow!("invalid signed control pubsub event: {error}"))?;
    let kind = u16::from(event.kind);
    if !is_control_kind(kind) {
        anyhow::bail!("unsupported control pubsub event kind {kind}");
    }
    let bytes = serde_json::to_vec(event).context("failed to encode control pubsub event")?;
    if bytes.len() > CONTROL_PUBSUB_MAX_EVENT_BYTES {
        anyhow::bail!(
            "control pubsub event is {} bytes, maximum is {}",
            bytes.len(),
            CONTROL_PUBSUB_MAX_EVENT_BYTES
        );
    }
    Ok(())
}

fn control_pubsub_outbox_event_paths(directory: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(%error, path = %directory.display(), "failed to scan control pubsub outbox");
            return Vec::new();
        }
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.truncate(OUTBOX_BATCH);
    paths
}

#[cfg(any(feature = "paid-exit", test))]
pub fn load_control_pubsub_events(config_path: &Path) -> Result<Vec<Event>> {
    Ok(ControlEventStore::load(Some(control_pubsub_store_file_path(config_path)))?.snapshot())
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::time::Duration;

    use super::*;
    use fips_endpoint::{
        Config, ConnectPolicy, PeerConfig, RoutingMode, TransportInstances, UdpConfig,
    };
    use nostr_sdk::prelude::{EventBuilder, ToBech32};

    #[test]
    fn retry_state_is_acknowledged_and_bounded_per_peer() {
        let event_id = "11".repeat(32);
        let inventory = ControlPubsubWireMessage::Inventory {
            event_id: event_id.clone(),
            event_kind: FIPS_PEER_ADVERT_KIND,
            payload_bytes: 512,
            hop_limit: 4,
        };
        let mut retries = RetryState::default();
        retries.track("peer-a", &inventory, 1_000);

        assert!(retries.due(1_249).is_empty());
        assert_eq!(retries.due(1_250).len(), 1);
        retries.acknowledge(
            "peer-a",
            &ControlPubsubWireMessage::Want {
                event_id: event_id.clone(),
            },
        );
        assert!(retries.due(10_000).is_empty());

        for index in 0..(MAX_PENDING_RETRIES_PER_PEER + 16) {
            retries.track(
                "peer-a",
                &ControlPubsubWireMessage::Want {
                    event_id: format!("{index:064x}"),
                },
                20_000,
            );
        }
        assert_eq!(
            retries
                .pending
                .keys()
                .filter(|key| key.peer_id == "peer-a")
                .count(),
            MAX_PENDING_RETRIES_PER_PEER
        );

        let mut unanswered = RetryState::default();
        unanswered.track("blackhole", &inventory, 30_000);
        assert_eq!(unanswered.due(30_250).len(), 1);
        assert_eq!(unanswered.due(30_500).len(), 1);
        assert!(unanswered.due(31_000).is_empty());
    }

    fn available_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("bind ephemeral UDP port")
            .local_addr()
            .expect("ephemeral UDP address")
            .port()
    }

    fn endpoint_config(local_port: u16, peers: &[(&str, u16, bool)]) -> Config {
        let mut config = Config::new();
        config.node.routing.mode = RoutingMode::ReplyLearned;
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some(format!("127.0.0.1:{local_port}")),
            accept_connections: Some(true),
            ..UdpConfig::default()
        });
        for (npub, port, auto_connect) in peers {
            let mut peer = PeerConfig::new(*npub, "udp", format!("127.0.0.1:{port}"));
            if !auto_connect {
                peer.connect_policy = ConnectPolicy::Manual;
            }
            config.peers.push(peer);
        }
        config
    }

    async fn endpoint(keys: &Keys, config: Config) -> Arc<FipsEndpoint> {
        Arc::new(
            FipsEndpoint::builder()
                .config(config)
                .identity_nsec(keys.secret_key().to_bech32().expect("nsec"))
                .without_system_tun()
                .bind()
                .await
                .expect("bind FIPS endpoint"),
        )
    }

    async fn wait_connected(endpoint: &FipsEndpoint, peer_npub: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if endpoint
                    .peers()
                    .await
                    .unwrap_or_default()
                    .iter()
                    .any(|peer| peer.connected && peer.npub == peer_npub)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("FIPS peer connected");
    }

    #[test]
    fn control_event_store_persists_one_copy_per_event_id() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("nvpn-control-pubsub-{nonce}"));
        let config_path = directory.join("config.toml");
        let store_path = control_pubsub_store_file_path(&config_path);
        let event = EventBuilder::new(Kind::Custom(FIPS_PEER_ADVERT_KIND), "advert")
            .sign_with_keys(&Keys::generate())
            .expect("signed advert");
        let mut store = ControlEventStore::load(Some(store_path)).expect("empty store");

        assert!(store.insert(event.clone()).expect("first insert"));
        assert!(!store.insert(event.clone()).expect("duplicate insert"));
        assert!(queue_control_pubsub_event(&config_path, &event).expect("queue first publication"));
        assert!(
            !queue_control_pubsub_event(&config_path, &event)
                .expect("deduplicate queued publication")
        );
        assert_eq!(
            load_control_pubsub_events(&config_path)
                .expect("reload store")
                .iter()
                .filter(|stored| stored.id == event.id)
                .count(),
            1
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn client_outbox_waits_for_a_connected_fips_peer() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("nvpn-control-wait-{nonce}"));
        let config_path = directory.join("config.toml");
        let event = EventBuilder::new(Kind::Custom(PAID_EXIT_OFFER_KIND), "offer")
            .sign_with_keys(&Keys::generate())
            .expect("signed offer");
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind FIPS endpoint"),
        );
        let runtime = ControlPubsubFipsRuntime::start(
            Arc::clone(&endpoint),
            NostrPubsubConfig {
                mode: NostrPubsubMode::Client,
                ..NostrPubsubConfig::default()
            },
            Vec::new(),
            Some(control_pubsub_store_file_path(&config_path)),
        )
        .await
        .expect("start client pubsub")
        .expect("client pubsub enabled");

        assert!(queue_control_pubsub_event(&config_path, &event).expect("queue offer"));
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            control_pubsub_outbox_directory(&config_path)
                .join(format!("{}.json", event.id.to_hex()))
                .exists()
        );

        runtime.stop().await;
        endpoint.shutdown().await.expect("shutdown endpoint");
        let _ = fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn three_node_fips_line_delivers_control_event_without_relays() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("nvpn-control-publisher-{nonce}"));
        let config_path = directory.join("config.toml");
        let alice = Keys::generate();
        let bob = Keys::generate();
        let carol = Keys::generate();
        let alice_npub = alice.public_key().to_bech32().expect("alice npub");
        let bob_npub = bob.public_key().to_bech32().expect("bob npub");
        let carol_npub = carol.public_key().to_bech32().expect("carol npub");
        let alice_port = available_udp_port();
        let bob_port = available_udp_port();
        let carol_port = available_udp_port();

        let alice_endpoint = endpoint(
            &alice,
            endpoint_config(alice_port, &[(&bob_npub, bob_port, true)]),
        )
        .await;
        let bob_endpoint = endpoint(
            &bob,
            endpoint_config(
                bob_port,
                &[
                    (&alice_npub, alice_port, true),
                    (&carol_npub, carol_port, true),
                ],
            ),
        )
        .await;
        let carol_endpoint = endpoint(
            &carol,
            endpoint_config(carol_port, &[(&bob_npub, bob_port, true)]),
        )
        .await;

        wait_connected(&alice_endpoint, &bob_npub).await;
        wait_connected(&bob_endpoint, &alice_npub).await;
        wait_connected(&bob_endpoint, &carol_npub).await;
        wait_connected(&carol_endpoint, &bob_npub).await;
        tokio::time::sleep(Duration::from_millis(1_200)).await;

        let config = NostrPubsubConfig {
            mode: NostrPubsubMode::Client,
            fanout: 8,
            max_hops: 4,
            max_event_bytes: 60 * 1024,
        };
        let alice_pubsub = ControlPubsubFipsRuntime::start(
            Arc::clone(&alice_endpoint),
            config.clone(),
            Vec::new(),
            None,
        )
        .await
        .expect("start Alice pubsub")
        .expect("Alice pubsub enabled");
        let bob_pubsub = ControlPubsubFipsRuntime::start(
            Arc::clone(&bob_endpoint),
            config.clone(),
            Vec::new(),
            None,
        )
        .await
        .expect("start Bob pubsub")
        .expect("Bob pubsub enabled");
        let carol_pubsub = ControlPubsubFipsRuntime::start(
            Arc::clone(&carol_endpoint),
            config,
            Vec::new(),
            Some(control_pubsub_store_file_path(&config_path)),
        )
        .await
        .expect("start Carol pubsub")
        .expect("Carol pubsub enabled");

        let event = EventBuilder::new(Kind::Custom(PAID_EXIT_OFFER_KIND), "offer")
            .sign_with_keys(&carol)
            .expect("signed paid exit offer");
        let event_id = event.id;
        assert!(queue_control_pubsub_event(&config_path, &event).expect("queue over FIPS"));

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if alice_pubsub
                    .events()
                    .await
                    .iter()
                    .any(|event| event.id == event_id)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("Alice receives Carol's event through Bob");
        assert_eq!(
            alice_pubsub
                .events()
                .await
                .iter()
                .filter(|event| event.id == event_id)
                .count(),
            1
        );

        alice_pubsub.stop().await;
        bob_pubsub.stop().await;
        carol_pubsub.stop().await;
        alice_endpoint.shutdown().await.expect("shutdown Alice");
        bob_endpoint.shutdown().await.expect("shutdown Bob");
        carol_endpoint.shutdown().await.expect("shutdown Carol");
        let _ = fs::remove_dir_all(directory);
    }
}
