use std::collections::HashSet;

use fips_core::{FipsEndpointPeer, Identity, NodeAddr};
use nostr_vpn_core::recent_peers::{RecentPeerEndpoints, recent_peers_scope};

const NETWORK_ID: &str = "recent-peer-tests";

fn npub() -> String {
    Identity::generate().npub()
}

fn cache(local_npub: &str) -> RecentPeerEndpoints {
    RecentPeerEndpoints::new(local_npub, recent_peers_scope(NETWORK_ID)).unwrap()
}

fn endpoint_peer(npub: String, transport: &str, addr: &str) -> FipsEndpointPeer {
    FipsEndpointPeer {
        npub,
        node_addr: NodeAddr::from_bytes([7; 16]),
        connected: true,
        transport_addr: Some(addr.to_string()),
        transport_type: Some(transport.to_string()),
        link_id: 1,
        srtt_ms: None,
        srtt_age_ms: None,
        packets_sent: 0,
        packets_recv: 0,
        bytes_sent: 0,
        bytes_recv: 0,
        rekey_in_progress: false,
        rekey_draining: false,
        current_k_bit: None,
        last_outbound_route: None,
        direct_probe_pending: false,
        direct_probe_after_ms: None,
        direct_probe_retry_count: 0,
        direct_probe_auto_reconnect: false,
        direct_probe_expires_at_ms: None,
        nostr_traversal_consecutive_failures: 0,
        nostr_traversal_in_cooldown: false,
        nostr_traversal_cooldown_until_ms: None,
        nostr_traversal_last_observed_skew_ms: None,
    }
}

#[test]
fn seconds_api_emits_shared_v1_json_with_millisecond_timestamps() {
    let local = npub();
    let remote = npub();
    let scope = recent_peers_scope(NETWORK_ID);
    let mut state = cache(&local);

    assert!(state.note_success(&remote, "udp:203.0.113.20:51820", 123));
    let serialized = state.to_json_pretty().unwrap();
    let json: serde_json::Value = serde_json::from_str(&serialized).unwrap();

    assert_eq!(json["version"], 1);
    assert_eq!(json["local_npub"], local);
    assert_eq!(json["scope"], scope);
    assert_eq!(json["peers"][&remote]["endpoints"][0]["transport"], "udp");
    assert_eq!(
        json["peers"][&remote]["endpoints"][0]["last_authenticated_at_ms"],
        123_000
    );

    let restored = RecentPeerEndpoints::from_json(&serialized, &local, &scope).unwrap();
    assert_eq!(
        restored.endpoints_for(&remote),
        vec!["203.0.113.20:51820".to_string()]
    );
}

#[test]
fn runtime_observation_keeps_only_reusable_authenticated_udp() {
    let local = npub();
    let udp_remote = npub();
    let tcp_remote = npub();
    let mut state = cache(&local);

    assert!(
        state
            .observe_authenticated_peer(
                &endpoint_peer(udp_remote.clone(), "udp", "10.0.0.8:51820"),
                456,
            )
            .unwrap()
    );
    assert!(
        !state
            .observe_authenticated_peer(
                &endpoint_peer(tcp_remote.clone(), "tcp", "203.0.113.20:443"),
                456,
            )
            .unwrap()
    );

    assert_eq!(
        state.endpoints_for(&udp_remote),
        vec!["10.0.0.8:51820".to_string()]
    );
    assert!(state.endpoints_for(&tcp_remote).is_empty());
    assert!(!state.as_recent_peers().peers.contains_key(&tcp_remote));
}

#[test]
fn legacy_note_success_drops_tcp_and_unusable_udp() {
    let local = npub();
    let remote = npub();
    let mut state = cache(&local);

    for addr in [
        "tcp:203.0.113.20:443",
        "203.0.113.20:0",
        "0.0.0.0:51820",
        "[ff02::1]:51820",
        "not-an-address",
    ] {
        assert!(!state.note_success(&remote, addr, 1));
    }
    assert!(state.is_empty());
}

#[test]
fn cache_caps_endpoints_and_prunes_using_seconds_helpers() {
    let local = npub();
    let remote = npub();
    let mut state = cache(&local);
    for index in 0..6 {
        assert!(state.note_success(
            &remote,
            &format!("203.0.113.20:{}", 50_000 + index),
            1_000 + index,
        ));
    }

    assert_eq!(state.endpoints_for(&remote).len(), 4);
    assert!(!state.prune_stale(6_000, 5_000));
    assert!(state.prune_stale(6_006, 5_000));
    assert!(state.is_empty());
}

#[test]
fn retain_participants_accepts_hex_or_npub_but_persists_canonical_npubs() {
    let local = Identity::generate();
    let alice = Identity::generate();
    let bob = Identity::generate();
    let mut state = cache(&local.npub());
    state.note_success(&alice.pubkey().to_string(), "203.0.113.1:51820", 1);
    state.note_success(&bob.npub(), "203.0.113.2:51820", 1);

    let keep = HashSet::from([alice.pubkey().to_string()]);
    assert!(state.retain_participants(&keep));

    assert!(state.as_recent_peers().peers.contains_key(&alice.npub()));
    assert!(!state.as_recent_peers().peers.contains_key(&bob.npub()));
}

#[test]
fn strict_decode_rejects_another_network_scope() {
    let local = npub();
    let state = cache(&local);
    let json = state.to_json_pretty().unwrap();

    assert!(
        RecentPeerEndpoints::from_json(&json, &local, &recent_peers_scope("other-network"))
            .is_err()
    );
}
