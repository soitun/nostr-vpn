//! Compatibility wrapper around FIPS's shared recent-peer cache.
//!
//! The cache is routing memory only. It records reusable endpoints after a
//! FIPS-authenticated connection, but callers must merge those routes only
//! into peers that are already authorized or explicitly configured.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};

use fips_core::{
    FipsEndpointPeer, RECENT_PEERS_MAX_ENDPOINTS_PER_PEER, RECENT_PEERS_MAX_PEERS, RecentPeer,
    RecentPeerEndpoint, RecentPeerTransport, RecentPeers, RecentPeersError,
};

use crate::config::{
    normalize_nostr_pubkey, normalize_runtime_network_id, npub_for_pubkey_hex,
    split_peer_transport_addr,
};

const RECENT_PEERS_SCOPE_PREFIX: &str = "nostr-vpn:";

/// Scope shared by one nostr-vpn network's recent-peer cache.
pub fn recent_peers_scope(network_id: &str) -> String {
    format!(
        "{RECENT_PEERS_SCOPE_PREFIX}{}",
        normalize_runtime_network_id(network_id)
    )
}

/// Seconds-facing compatibility API backed by FIPS's millisecond v1 model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentPeerEndpoints {
    inner: RecentPeers,
}

impl RecentPeerEndpoints {
    /// Create an empty cache bound to a canonical local npub and app scope.
    pub fn new(
        local_npub: impl Into<String>,
        scope: impl Into<String>,
    ) -> Result<Self, RecentPeersError> {
        RecentPeers::new(local_npub, scope).map(Self::from_recent_peers)
    }

    /// Wrap an already validated shared recent-peer document.
    pub fn from_recent_peers(inner: RecentPeers) -> Self {
        Self { inner }
    }

    /// Shared recent-peer document used by native persistence adapters.
    pub fn as_recent_peers(&self) -> &RecentPeers {
        &self.inner
    }

    pub fn local_npub(&self) -> &str {
        self.inner.local_npub()
    }

    pub fn scope(&self) -> &str {
        self.inner.scope()
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .peers
            .values()
            .all(|peer| peer.endpoints.is_empty())
    }

    /// Record a reusable authenticated UDP endpoint using the legacy seconds
    /// timestamp API. Hex public keys are canonicalized to npubs before being
    /// placed in the shared document. TCP and unusable socket addresses are
    /// ignored.
    pub fn note_success(&mut self, participant: &str, addr: &str, success_at: u64) -> bool {
        let Some(participant) = canonical_npub(participant) else {
            return false;
        };
        if participant == self.inner.local_npub {
            return false;
        }
        let Some(addr) = normalize_reusable_udp_endpoint(addr) else {
            return false;
        };
        let authenticated_at_ms = success_at.saturating_mul(1000);
        let previous = self.inner.peers.get(&participant).cloned();
        let peer = self
            .inner
            .peers
            .entry(participant.clone())
            .or_insert(RecentPeer {
                last_authenticated_at_ms: authenticated_at_ms,
                endpoints: Vec::new(),
            });
        peer.last_authenticated_at_ms = peer.last_authenticated_at_ms.max(authenticated_at_ms);

        if let Some(endpoint) = peer.endpoints.iter_mut().find(|endpoint| {
            endpoint
                .addr
                .parse::<SocketAddr>()
                .is_ok_and(|stored| stored == addr)
        }) {
            endpoint.addr = addr.to_string();
            endpoint.last_authenticated_at_ms =
                endpoint.last_authenticated_at_ms.max(authenticated_at_ms);
        } else {
            peer.endpoints.push(RecentPeerEndpoint {
                transport: RecentPeerTransport::Udp,
                addr: addr.to_string(),
                last_authenticated_at_ms: authenticated_at_ms,
            });
        }
        peer.endpoints.sort_by(|left, right| {
            right
                .last_authenticated_at_ms
                .cmp(&left.last_authenticated_at_ms)
                .then_with(|| left.addr.cmp(&right.addr))
        });
        peer.endpoints.truncate(RECENT_PEERS_MAX_ENDPOINTS_PER_PEER);
        self.retain_newest_peers();

        self.inner.peers.get(&participant) != previous.as_ref()
    }

    /// Record one runtime FIPS snapshot only when its authenticated transport
    /// is a reusable UDP restart candidate.
    pub fn observe_authenticated_peer(
        &mut self,
        peer: &FipsEndpointPeer,
        authenticated_at_secs: u64,
    ) -> Result<bool, RecentPeersError> {
        if peer.authenticated_udp_restart_addr().is_none() {
            return Ok(false);
        }
        self.inner
            .observe_authenticated_peer(peer, authenticated_at_secs.saturating_mul(1000))
    }

    /// Drop entries older than `now - ttl_secs` while preserving the legacy
    /// convention that a zero TTL disables pruning.
    pub fn prune_stale(&mut self, now: u64, ttl_secs: u64) -> bool {
        if ttl_secs == 0 {
            return false;
        }
        let peer_count = self.inner.peers.len();
        let endpoint_count = self
            .inner
            .peers
            .values()
            .map(|peer| peer.endpoints.len())
            .sum::<usize>();
        self.inner
            .prune(now.saturating_mul(1000), ttl_secs.saturating_mul(1000));
        self.inner.peers.len() != peer_count
            || self
                .inner
                .peers
                .values()
                .map(|peer| peer.endpoints.len())
                .sum::<usize>()
                != endpoint_count
    }

    /// Keep only entries for the supplied public keys.
    pub fn retain_participants(&mut self, participants: &HashSet<String>) -> bool {
        let canonical = participants
            .iter()
            .filter_map(|participant| canonical_npub(participant))
            .collect::<HashSet<_>>();
        let before = self.inner.peers.len();
        self.inner
            .peers
            .retain(|participant, _| canonical.contains(participant));
        self.inner.peers.len() != before
    }

    /// Endpoint strings recorded for a single participant.
    pub fn endpoints_for(&self, participant: &str) -> Vec<String> {
        let Some(participant) = canonical_npub(participant) else {
            return Vec::new();
        };
        let mut endpoints = self
            .inner
            .peers
            .get(&participant)
            .map(|peer| {
                peer.endpoints
                    .iter()
                    .map(|endpoint| endpoint.addr.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        endpoints.sort();
        endpoints.dedup();
        endpoints
    }

    /// Sorted compatibility snapshot without timestamps.
    pub fn as_static_peer_endpoints(&self) -> Vec<(String, Vec<String>)> {
        self.inner
            .peers
            .iter()
            .filter_map(|(participant, peer)| {
                let mut endpoints = peer
                    .endpoints
                    .iter()
                    .map(|endpoint| endpoint.addr.clone())
                    .collect::<Vec<_>>();
                endpoints.sort();
                endpoints.dedup();
                (!endpoints.is_empty()).then(|| (participant.clone(), endpoints))
            })
            .collect()
    }

    /// Sorted compatibility snapshot preserving FIPS's millisecond freshness.
    pub fn as_static_peer_endpoints_with_seen_at(&self) -> Vec<(String, Vec<(String, u64)>)> {
        self.inner
            .peers
            .iter()
            .filter_map(|(participant, peer)| {
                let mut endpoints = peer
                    .endpoints
                    .iter()
                    .map(|endpoint| (endpoint.addr.clone(), endpoint.last_authenticated_at_ms))
                    .collect::<Vec<_>>();
                endpoints.sort_by(|left, right| left.0.cmp(&right.0));
                endpoints.dedup_by(|left, right| left.0 == right.0);
                (!endpoints.is_empty()).then(|| (participant.clone(), endpoints))
            })
            .collect()
    }

    pub fn to_json_pretty(&self) -> Result<String, RecentPeersError> {
        self.inner.to_json_pretty()
    }

    pub fn from_json(
        raw: &str,
        expected_local_npub: &str,
        expected_scope: &str,
    ) -> Result<Self, RecentPeersError> {
        RecentPeers::from_json(raw, expected_local_npub, expected_scope)
            .map(Self::from_recent_peers)
    }

    fn retain_newest_peers(&mut self) {
        while self.inner.peers.len() > RECENT_PEERS_MAX_PEERS {
            let Some(oldest) = self
                .inner
                .peers
                .iter()
                .min_by(|left, right| {
                    left.1
                        .last_authenticated_at_ms
                        .cmp(&right.1.last_authenticated_at_ms)
                        .then_with(|| left.0.cmp(right.0))
                })
                .map(|(participant, _)| participant.clone())
            else {
                break;
            };
            self.inner.peers.remove(&oldest);
        }
    }
}

fn canonical_npub(value: &str) -> Option<String> {
    normalize_nostr_pubkey(value.trim())
        .ok()
        .map(|pubkey| npub_for_pubkey_hex(&pubkey))
}

fn normalize_reusable_udp_endpoint(addr: &str) -> Option<SocketAddr> {
    let (transport, host_port) = split_peer_transport_addr(addr.trim());
    if !transport.eq_ignore_ascii_case("udp") {
        return None;
    }
    let addr = host_port.parse::<SocketAddr>().ok()?;
    let unusable_ip = match addr.ip() {
        IpAddr::V4(ip) => ip.is_unspecified() || ip.is_multicast(),
        IpAddr::V6(ip) => ip.is_unspecified() || ip.is_multicast(),
    };
    (addr.port() != 0 && !unusable_ip).then_some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_namespaced_and_network_bound() {
        assert_eq!(recent_peers_scope("  My Network  "), "nostr-vpn:My Network");
    }

    #[test]
    fn reusable_udp_normalizes_bare_and_tagged_addresses() {
        assert_eq!(
            normalize_reusable_udp_endpoint("203.0.113.5:51820"),
            Some("203.0.113.5:51820".parse().unwrap())
        );
        assert_eq!(
            normalize_reusable_udp_endpoint("udp:10.0.0.5:51820"),
            Some("10.0.0.5:51820".parse().unwrap())
        );
    }

    #[test]
    fn tcp_and_unusable_udp_are_rejected() {
        assert_eq!(normalize_reusable_udp_endpoint("tcp:203.0.113.5:443"), None);
        assert_eq!(normalize_reusable_udp_endpoint("203.0.113.5:0"), None);
        assert_eq!(normalize_reusable_udp_endpoint("0.0.0.0:51820"), None);
        assert_eq!(normalize_reusable_udp_endpoint("[ff02::1]:51820"), None);
    }
}
