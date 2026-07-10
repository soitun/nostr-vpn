use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use nostr_sdk::prelude::Event;
use serde::{Deserialize, Serialize};

pub const CONTROL_PUBSUB_PROTOCOL: &str = "nvpn.control.pubsub";
pub const CONTROL_PUBSUB_VERSION: u8 = 1;
pub const CONTROL_PUBSUB_FIPS_SERVICE_PORT: u16 = 7_369;
pub const CONTROL_PUBSUB_MAX_WIRE_BYTES: usize = 60 * 1024;
pub const CONTROL_PUBSUB_MAX_EVENT_BYTES: usize = 56 * 1024;
pub const FIPS_PEER_ADVERT_KIND: u16 = 37_195;
pub const PAID_EXIT_OFFER_KIND: u16 = 37_196;
pub const RATING_FACT_KIND: u16 = 7_368;

const DEFAULT_ROUTE_TTL_MS: u64 = 2 * 60 * 1_000;
const DEFAULT_EVENT_TTL_MS: u64 = 10 * 60 * 1_000;

pub type Result<T> = std::result::Result<T, ControlPubsubError>;

#[derive(Debug, thiserror::Error)]
pub enum ControlPubsubError {
    #[error("unsupported control pubsub protocol {0}")]
    UnsupportedProtocol(String),
    #[error("unsupported control pubsub version {0}")]
    UnsupportedVersion(u8),
    #[error("control pubsub wire payload is {len} bytes, maximum is {max}")]
    WirePayloadTooLarge { len: usize, max: usize },
    #[error("control pubsub event is {len} bytes, maximum is {max}")]
    EventTooLarge { len: usize, max: usize },
    #[error("unsupported Nostr event kind {0}")]
    UnsupportedEventKind(u16),
    #[error("invalid control pubsub event id {0}")]
    InvalidEventId(String),
    #[error("control pubsub frame id does not match signed event id")]
    EventIdMismatch,
    #[error("invalid signed Nostr event: {0}")]
    InvalidEvent(String),
    #[error("invalid control pubsub JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlPubsubWireMessage {
    Inventory {
        event_id: String,
        event_kind: u16,
        payload_bytes: u32,
        hop_limit: u8,
    },
    Want {
        event_id: String,
    },
    Frame {
        event_id: String,
        event: Box<Event>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ControlPubsubEnvelope {
    protocol: String,
    version: u8,
    message: ControlPubsubWireMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPubsubCodec {
    max_wire_bytes: usize,
}

impl Default for ControlPubsubCodec {
    fn default() -> Self {
        Self {
            max_wire_bytes: CONTROL_PUBSUB_MAX_WIRE_BYTES,
        }
    }
}

impl ControlPubsubCodec {
    pub fn new(max_wire_bytes: usize) -> Self {
        Self {
            max_wire_bytes: max_wire_bytes.max(1),
        }
    }

    pub fn encode(&self, message: &ControlPubsubWireMessage) -> Result<Vec<u8>> {
        let encoded = serde_json::to_vec(&ControlPubsubEnvelope {
            protocol: CONTROL_PUBSUB_PROTOCOL.to_string(),
            version: CONTROL_PUBSUB_VERSION,
            message: message.clone(),
        })?;
        self.check_wire_len(encoded.len())?;
        Ok(encoded)
    }

    pub fn decode(&self, payload: &[u8]) -> Result<ControlPubsubWireMessage> {
        self.check_wire_len(payload.len())?;
        let envelope: ControlPubsubEnvelope = serde_json::from_slice(payload)?;
        if envelope.protocol != CONTROL_PUBSUB_PROTOCOL {
            return Err(ControlPubsubError::UnsupportedProtocol(envelope.protocol));
        }
        if envelope.version != CONTROL_PUBSUB_VERSION {
            return Err(ControlPubsubError::UnsupportedVersion(envelope.version));
        }
        Ok(envelope.message)
    }

    fn check_wire_len(&self, len: usize) -> Result<()> {
        if len > self.max_wire_bytes {
            return Err(ControlPubsubError::WirePayloadTooLarge {
                len,
                max: self.max_wire_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPubsubOptions {
    pub fanout: usize,
    pub max_hops: u8,
    pub max_event_bytes: usize,
    pub max_cached_events: usize,
    pub max_seen_events: usize,
    pub max_pending_peers_per_event: usize,
    pub route_ttl_ms: u64,
    pub event_ttl_ms: u64,
    pub allowed_kinds: BTreeSet<u16>,
}

impl Default for ControlPubsubOptions {
    fn default() -> Self {
        Self {
            fanout: 8,
            max_hops: 4,
            max_event_bytes: CONTROL_PUBSUB_MAX_EVENT_BYTES,
            max_cached_events: 1_024,
            max_seen_events: 4_096,
            max_pending_peers_per_event: 64,
            route_ttl_ms: DEFAULT_ROUTE_TTL_MS,
            event_ttl_ms: DEFAULT_EVENT_TTL_MS,
            allowed_kinds: BTreeSet::from([
                FIPS_PEER_ADVERT_KIND,
                PAID_EXIT_OFFER_KIND,
                RATING_FACT_KIND,
            ]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlPubsubAction {
    Send {
        peer_id: String,
        message: ControlPubsubWireMessage,
    },
    Deliver {
        source_peer: String,
        event: Event,
    },
}

#[derive(Debug, Clone)]
struct CachedEvent {
    event: Event,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct UpstreamRoute {
    peer_id: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingPeers {
    peers: BTreeSet<String>,
    expires_at_ms: u64,
}

struct ReceivedInventory {
    event_id: String,
    event_kind: u16,
    payload_bytes: u32,
    hop_limit: u8,
}

pub struct ControlPubsubMesh {
    options: ControlPubsubOptions,
    cached_events: HashMap<String, CachedEvent>,
    cache_order: VecDeque<String>,
    seen_inventories: HashMap<String, u64>,
    seen_order: VecDeque<String>,
    delivered_events: HashSet<String>,
    delivered_order: VecDeque<String>,
    upstream_routes: HashMap<String, UpstreamRoute>,
    pending_downstream: HashMap<String, PendingPeers>,
    want_forwarded: HashMap<String, u64>,
}

impl ControlPubsubMesh {
    pub fn new(mut options: ControlPubsubOptions) -> Self {
        options.fanout = options.fanout.max(1);
        options.max_hops = options.max_hops.max(1);
        options.max_event_bytes = options.max_event_bytes.max(1);
        options.max_cached_events = options.max_cached_events.max(1);
        options.max_seen_events = options.max_seen_events.max(1);
        options.max_pending_peers_per_event = options.max_pending_peers_per_event.max(1);
        options.route_ttl_ms = options.route_ttl_ms.max(1);
        options.event_ttl_ms = options.event_ttl_ms.max(options.route_ttl_ms);
        Self {
            options,
            cached_events: HashMap::new(),
            cache_order: VecDeque::new(),
            seen_inventories: HashMap::new(),
            seen_order: VecDeque::new(),
            delivered_events: HashSet::new(),
            delivered_order: VecDeque::new(),
            upstream_routes: HashMap::new(),
            pending_downstream: HashMap::new(),
            want_forwarded: HashMap::new(),
        }
    }

    pub fn publish(
        &mut self,
        event: Event,
        peers: &[String],
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        self.prune(now_ms);
        let (event_id, payload_bytes) = self.validate_event(&event)?;
        let event_kind = u16::from(event.kind);
        self.store_event(event, now_ms);
        if !self.remember_inventory(&event_id, now_ms) {
            return Ok(Vec::new());
        }
        let inventory = ControlPubsubWireMessage::Inventory {
            event_id,
            event_kind,
            payload_bytes,
            hop_limit: self.options.max_hops,
        };
        Ok(self.send_to_selected_peers(peers, None, inventory))
    }

    pub fn receive(
        &mut self,
        source_peer: &str,
        message: ControlPubsubWireMessage,
        peers: &[String],
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        self.prune(now_ms);
        match message {
            ControlPubsubWireMessage::Inventory {
                event_id,
                event_kind,
                payload_bytes,
                hop_limit,
            } => self.receive_inventory(
                source_peer,
                peers,
                ReceivedInventory {
                    event_id,
                    event_kind,
                    payload_bytes,
                    hop_limit,
                },
                now_ms,
            ),
            ControlPubsubWireMessage::Want { event_id } => {
                self.receive_want(source_peer, event_id, now_ms)
            }
            ControlPubsubWireMessage::Frame { event_id, event } => {
                self.receive_frame(source_peer, event_id, *event, now_ms)
            }
        }
    }

    fn receive_inventory(
        &mut self,
        source_peer: &str,
        peers: &[String],
        inventory: ReceivedInventory,
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        let ReceivedInventory {
            event_id,
            event_kind,
            payload_bytes,
            hop_limit,
        } = inventory;
        validate_event_id(&event_id)?;
        self.validate_kind(event_kind)?;
        self.validate_event_len(payload_bytes as usize)?;
        if hop_limit == 0 || !self.remember_inventory(&event_id, now_ms) {
            return Ok(Vec::new());
        }

        let route_expiry = now_ms.saturating_add(self.options.route_ttl_ms);
        self.upstream_routes
            .entry(event_id.clone())
            .or_insert_with(|| UpstreamRoute {
                peer_id: source_peer.to_string(),
                expires_at_ms: route_expiry,
            });
        self.want_forwarded.insert(event_id.clone(), route_expiry);

        let mut actions = vec![ControlPubsubAction::Send {
            peer_id: source_peer.to_string(),
            message: ControlPubsubWireMessage::Want {
                event_id: event_id.clone(),
            },
        }];
        if hop_limit > 1 {
            actions.extend(self.send_to_selected_peers(
                peers,
                Some(source_peer),
                ControlPubsubWireMessage::Inventory {
                    event_id,
                    event_kind,
                    payload_bytes,
                    hop_limit: hop_limit - 1,
                },
            ));
        }
        Ok(actions)
    }

    fn receive_want(
        &mut self,
        source_peer: &str,
        event_id: String,
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        validate_event_id(&event_id)?;
        if let Some(cached) = self.cached_events.get(&event_id) {
            return Ok(vec![ControlPubsubAction::Send {
                peer_id: source_peer.to_string(),
                message: ControlPubsubWireMessage::Frame {
                    event_id,
                    event: Box::new(cached.event.clone()),
                },
            }]);
        }

        let pending = self
            .pending_downstream
            .entry(event_id.clone())
            .or_insert_with(|| PendingPeers {
                peers: BTreeSet::new(),
                expires_at_ms: now_ms.saturating_add(self.options.route_ttl_ms),
            });
        if pending.peers.len() < self.options.max_pending_peers_per_event {
            pending.peers.insert(source_peer.to_string());
        }

        let Some(route) = self.upstream_routes.get(&event_id) else {
            return Ok(Vec::new());
        };
        let already_forwarded = self
            .want_forwarded
            .get(&event_id)
            .is_some_and(|expiry| *expiry > now_ms);
        if already_forwarded {
            return Ok(Vec::new());
        }
        self.want_forwarded
            .insert(event_id.clone(), route.expires_at_ms);
        Ok(vec![ControlPubsubAction::Send {
            peer_id: route.peer_id.clone(),
            message: ControlPubsubWireMessage::Want { event_id },
        }])
    }

    fn receive_frame(
        &mut self,
        source_peer: &str,
        event_id: String,
        event: Event,
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        validate_event_id(&event_id)?;
        let (verified_id, _) = self.validate_event(&event)?;
        if verified_id != event_id {
            return Err(ControlPubsubError::EventIdMismatch);
        }
        self.store_event(event.clone(), now_ms);

        let mut actions = Vec::new();
        if self.remember_delivered(&event_id) {
            actions.push(ControlPubsubAction::Deliver {
                source_peer: source_peer.to_string(),
                event: event.clone(),
            });
        }
        if let Some(pending) = self.pending_downstream.remove(&event_id) {
            actions.extend(
                pending
                    .peers
                    .into_iter()
                    .map(|peer_id| ControlPubsubAction::Send {
                        peer_id,
                        message: ControlPubsubWireMessage::Frame {
                            event_id: event_id.clone(),
                            event: Box::new(event.clone()),
                        },
                    }),
            );
        }
        Ok(actions)
    }

    fn validate_event(&self, event: &Event) -> Result<(String, u32)> {
        event
            .verify()
            .map_err(|error| ControlPubsubError::InvalidEvent(error.to_string()))?;
        self.validate_kind(u16::from(event.kind))?;
        let payload = serde_json::to_vec(event)?;
        self.validate_event_len(payload.len())?;
        let payload_bytes =
            u32::try_from(payload.len()).map_err(|_| ControlPubsubError::EventTooLarge {
                len: payload.len(),
                max: self.options.max_event_bytes,
            })?;
        Ok((event.id.to_hex(), payload_bytes))
    }

    fn validate_kind(&self, event_kind: u16) -> Result<()> {
        if !self.options.allowed_kinds.contains(&event_kind) {
            return Err(ControlPubsubError::UnsupportedEventKind(event_kind));
        }
        Ok(())
    }

    fn validate_event_len(&self, len: usize) -> Result<()> {
        if len > self.options.max_event_bytes {
            return Err(ControlPubsubError::EventTooLarge {
                len,
                max: self.options.max_event_bytes,
            });
        }
        Ok(())
    }

    fn store_event(&mut self, event: Event, now_ms: u64) {
        let event_id = event.id.to_hex();
        if !self.cached_events.contains_key(&event_id) {
            while self.cached_events.len() >= self.options.max_cached_events {
                let Some(oldest) = self.cache_order.pop_front() else {
                    break;
                };
                self.cached_events.remove(&oldest);
            }
            self.cache_order.push_back(event_id.clone());
        }
        self.cached_events.insert(
            event_id,
            CachedEvent {
                event,
                expires_at_ms: now_ms.saturating_add(self.options.event_ttl_ms),
            },
        );
    }

    fn remember_inventory(&mut self, event_id: &str, now_ms: u64) -> bool {
        if self
            .seen_inventories
            .get(event_id)
            .is_some_and(|expiry| *expiry > now_ms)
        {
            return false;
        }
        if !self.seen_inventories.contains_key(event_id) {
            while self.seen_inventories.len() >= self.options.max_seen_events {
                let Some(oldest) = self.seen_order.pop_front() else {
                    break;
                };
                self.seen_inventories.remove(&oldest);
            }
            self.seen_order.push_back(event_id.to_string());
        }
        self.seen_inventories.insert(
            event_id.to_string(),
            now_ms.saturating_add(self.options.route_ttl_ms),
        );
        true
    }

    fn remember_delivered(&mut self, event_id: &str) -> bool {
        if !self.delivered_events.insert(event_id.to_string()) {
            return false;
        }
        self.delivered_order.push_back(event_id.to_string());
        while self.delivered_events.len() > self.options.max_seen_events {
            let Some(oldest) = self.delivered_order.pop_front() else {
                break;
            };
            self.delivered_events.remove(&oldest);
        }
        true
    }

    fn send_to_selected_peers(
        &self,
        peers: &[String],
        excluded_peer: Option<&str>,
        message: ControlPubsubWireMessage,
    ) -> Vec<ControlPubsubAction> {
        let mut selected = peers
            .iter()
            .filter(|peer| excluded_peer != Some(peer.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(self.options.fanout)
            .collect::<Vec<_>>();
        selected
            .drain(..)
            .map(|peer_id| ControlPubsubAction::Send {
                peer_id,
                message: message.clone(),
            })
            .collect()
    }

    fn prune(&mut self, now_ms: u64) {
        self.cached_events
            .retain(|_, cached| cached.expires_at_ms > now_ms);
        self.cache_order
            .retain(|event_id| self.cached_events.contains_key(event_id));
        self.seen_inventories
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
        self.seen_order
            .retain(|event_id| self.seen_inventories.contains_key(event_id));
        self.upstream_routes
            .retain(|_, route| route.expires_at_ms > now_ms);
        self.pending_downstream
            .retain(|_, pending| pending.expires_at_ms > now_ms);
        self.want_forwarded
            .retain(|_, expires_at_ms| *expires_at_ms > now_ms);
    }
}

fn validate_event_id(event_id: &str) -> Result<()> {
    if event_id.len() == 64 && event_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(ControlPubsubError::InvalidEventId(event_id.to_string()))
}
