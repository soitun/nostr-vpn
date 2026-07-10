use std::collections::BTreeSet;

use nostr_pubsub::{
    InvWantAction, InvWantCodec, InvWantMesh, InvWantMeshOptions, InvWantWireMessage, MeshPeer,
    PubsubError,
};
use nostr_sdk::prelude::Event;

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

pub type Result<T> = std::result::Result<T, PubsubError>;
pub type ControlPubsubError = PubsubError;
pub type ControlPubsubWireMessage = InvWantWireMessage;
pub type ControlPubsubAction = InvWantAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPubsubCodec {
    inner: InvWantCodec,
}

impl Default for ControlPubsubCodec {
    fn default() -> Self {
        Self::new(CONTROL_PUBSUB_MAX_WIRE_BYTES)
    }
}

impl ControlPubsubCodec {
    #[must_use]
    pub fn new(max_wire_bytes: usize) -> Self {
        Self {
            inner: InvWantCodec::new(
                CONTROL_PUBSUB_PROTOCOL,
                CONTROL_PUBSUB_VERSION,
                max_wire_bytes,
            ),
        }
    }

    pub fn encode(&self, message: &ControlPubsubWireMessage) -> Result<Vec<u8>> {
        self.inner.encode(message)
    }

    pub fn decode(&self, payload: &[u8]) -> Result<ControlPubsubWireMessage> {
        self.inner.decode(payload)
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

pub struct ControlPubsubMesh {
    inner: InvWantMesh,
}

impl ControlPubsubMesh {
    #[must_use]
    pub fn new(options: ControlPubsubOptions) -> Self {
        Self {
            inner: InvWantMesh::new(InvWantMeshOptions {
                fanout: options.fanout,
                unknown_peer_reserve: 1,
                max_hops: options.max_hops,
                max_event_bytes: options.max_event_bytes,
                max_cached_events: options.max_cached_events,
                max_seen_events: options.max_seen_events,
                max_pending_peers_per_event: options.max_pending_peers_per_event,
                route_ttl_ms: options.route_ttl_ms,
                event_ttl_ms: options.event_ttl_ms,
                allowed_kinds: Some(options.allowed_kinds),
            }),
        }
    }

    pub fn publish(
        &mut self,
        event: Event,
        peers: &[String],
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        self.inner.publish(event, &mesh_peers(peers), now_ms)
    }

    pub fn receive(
        &mut self,
        source_peer: &str,
        message: ControlPubsubWireMessage,
        peers: &[String],
        now_ms: u64,
    ) -> Result<Vec<ControlPubsubAction>> {
        self.inner
            .receive(source_peer, message, &mesh_peers(peers), now_ms)
    }
}

fn mesh_peers(peers: &[String]) -> Vec<MeshPeer> {
    peers.iter().cloned().map(MeshPeer::new).collect()
}
