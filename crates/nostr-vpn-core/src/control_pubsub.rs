use std::collections::BTreeSet;

use nostr_pubsub::InvWantMeshOptions;

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

impl ControlPubsubOptions {
    /// Convert VPN policy/configuration into options for the shared Inv/WANT
    /// state machine without wrapping or forking that implementation.
    #[must_use]
    pub fn into_mesh_options(self) -> InvWantMeshOptions {
        InvWantMeshOptions {
            fanout: self.fanout,
            unknown_peer_reserve: 1,
            max_hops: self.max_hops,
            max_event_bytes: self.max_event_bytes,
            max_cached_events: self.max_cached_events,
            max_cached_event_bytes: nostr_pubsub::DEFAULT_INV_WANT_MAX_CACHE_BYTES,
            max_seen_events: self.max_seen_events,
            max_pending_peers_per_event: self.max_pending_peers_per_event,
            route_ttl_ms: self.route_ttl_ms,
            event_ttl_ms: self.event_ttl_ms,
            allowed_kinds: Some(self.allowed_kinds),
        }
    }
}
