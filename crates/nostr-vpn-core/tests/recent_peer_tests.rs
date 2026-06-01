use std::collections::HashSet;

use nostr_vpn_core::recent_peers::RecentPeerEndpoints;

#[test]
fn note_success_records_public_endpoint() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "a".repeat(64);

    let changed = state.note_success(&participant, "203.0.113.20:51820", 1_000);
    assert!(
        changed,
        "first observation of a public endpoint must be persisted"
    );
    assert!(
        !state.note_success(&participant, "203.0.113.20:51820", 1_000),
        "re-recording the same observation at the same timestamp is a no-op"
    );

    let endpoints = state.endpoints_for(&participant);
    assert_eq!(endpoints, vec!["203.0.113.20:51820".to_string()]);
}

#[test]
fn note_success_preserves_tcp_and_normalizes_udp_transport_tags() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "a".repeat(64);

    assert!(state.note_success(&participant, "tcp:203.0.113.20:443", 1_000));
    assert!(state.note_success(&participant, "udp:203.0.113.20:51820", 1_000));

    let endpoints = state.endpoints_for(&participant);
    assert_eq!(
        endpoints,
        vec![
            "203.0.113.20:51820".to_string(),
            "tcp:203.0.113.20:443".to_string()
        ]
    );
}

#[test]
fn note_success_ignores_private_endpoints() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "b".repeat(64);

    for addr in [
        "192.168.1.10:51820",
        "10.0.0.5:51820",
        "172.16.5.1:51820",
        "100.64.10.1:51820", // CGNAT
        "127.0.0.1:51820",
        "169.254.10.10:51820",
        "[fd00::1]:51820",
        "[fe80::1]:51820",
        "[::1]:51820",
        "tcp:192.168.1.10:443",
    ] {
        let changed = state.note_success(&participant, addr, 1_000);
        assert!(!changed, "LAN endpoint {addr} must not be persisted");
    }

    assert!(state.endpoints_for(&participant).is_empty());
}

#[test]
fn note_success_ignores_malformed_endpoints() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "c".repeat(64);

    for addr in [
        "",
        "not-an-address",
        "203.0.113.20",
        "host:notaport",
        "tor:203.0.113.20:9001",
    ] {
        assert!(!state.note_success(&participant, addr, 1_000));
    }
    assert!(state.endpoints_for(&participant).is_empty());
}

#[test]
fn note_success_updates_timestamp_for_existing_entry() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "d".repeat(64);

    state.note_success(&participant, "203.0.113.20:51820", 1_000);
    let changed = state.note_success(&participant, "203.0.113.20:51820", 2_000);
    assert!(changed, "later success timestamp must update the entry");

    // Stale-prune to confirm timestamp moved forward.
    assert!(
        !state.prune_stale(2_500, 1_000),
        "entry should remain because last_success_at was bumped to 2_000"
    );
    assert_eq!(
        state.endpoints_for(&participant),
        vec!["203.0.113.20:51820".to_string()]
    );
}

#[test]
fn prune_stale_drops_old_entries() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "e".repeat(64);

    state.note_success(&participant, "203.0.113.20:51820", 1_000);
    state.note_success(&participant, "198.51.100.42:51820", 5_000);

    let changed = state.prune_stale(6_000, 2_000);
    assert!(changed, "stale entry should have been pruned");

    let endpoints = state.endpoints_for(&participant);
    assert_eq!(endpoints, vec!["198.51.100.42:51820".to_string()]);
}

#[test]
fn prune_stale_drops_participant_when_no_endpoints_remain() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "f".repeat(64);

    state.note_success(&participant, "203.0.113.20:51820", 1_000);
    let changed = state.prune_stale(10_000, 2_000);
    assert!(changed);
    assert!(state.endpoints_for(&participant).is_empty());
    assert!(state.is_empty());
}

#[test]
fn recent_cache_caps_endpoints_per_peer_to_freshest_observations() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "g".repeat(64);

    for i in 0..6 {
        state.note_success(
            &participant,
            &format!("203.0.113.20:{}", 50_000 + i),
            1_000 + i,
        );
    }

    assert_eq!(
        state.endpoints_for(&participant),
        vec![
            "203.0.113.20:50002".to_string(),
            "203.0.113.20:50003".to_string(),
            "203.0.113.20:50004".to_string(),
            "203.0.113.20:50005".to_string(),
        ]
    );
}

#[test]
fn retain_participants_filters_unknown_npubs() {
    let mut state = RecentPeerEndpoints::default();
    let alice = "a".repeat(64);
    let bob = "b".repeat(64);

    state.note_success(&alice, "203.0.113.1:51820", 1_000);
    state.note_success(&bob, "203.0.113.2:51820", 1_000);

    let keep: HashSet<String> = [alice.clone()].into_iter().collect();
    state.retain_participants(&keep);

    assert_eq!(
        state.endpoints_for(&alice),
        vec!["203.0.113.1:51820".to_string()]
    );
    assert!(state.endpoints_for(&bob).is_empty());
}

#[test]
fn round_trip_through_json_preserves_state() {
    let mut state = RecentPeerEndpoints::default();
    let participant = "1".repeat(64);
    state.note_success(&participant, "203.0.113.20:51820", 1_000);
    state.note_success(&participant, "198.51.100.5:51820", 2_500);

    let serialized = state.to_json_pretty().expect("serialize");
    let restored = RecentPeerEndpoints::from_json(&serialized).expect("deserialize");

    let mut roundtrip = restored.endpoints_for(&participant);
    roundtrip.sort();
    let mut expected = state.endpoints_for(&participant);
    expected.sort();
    assert_eq!(roundtrip, expected);
}

#[test]
fn from_json_tolerates_garbage_input() {
    assert!(RecentPeerEndpoints::from_json("not json").is_err());
    let empty = RecentPeerEndpoints::from_json("{}").expect("empty object");
    assert!(empty.is_empty());
}

#[test]
fn from_json_caps_legacy_endpoint_floods() {
    let raw = r#"{
        "version": 1,
        "entries": {
            "peer-a": [
                {"addr": "203.0.113.20:50000", "last_success_at": 1000},
                {"addr": "203.0.113.20:50001", "last_success_at": 1001},
                {"addr": "203.0.113.20:50002", "last_success_at": 1002},
                {"addr": "203.0.113.20:50003", "last_success_at": 1003},
                {"addr": "203.0.113.20:50004", "last_success_at": 1004}
            ]
        }
    }"#;

    let restored = RecentPeerEndpoints::from_json(raw).expect("deserialize");

    assert_eq!(
        restored.endpoints_for("peer-a"),
        vec![
            "203.0.113.20:50001".to_string(),
            "203.0.113.20:50002".to_string(),
            "203.0.113.20:50003".to_string(),
            "203.0.113.20:50004".to_string(),
        ]
    );
}

#[test]
fn as_static_peer_endpoints_emits_sorted_npubs() {
    let mut state = RecentPeerEndpoints::default();
    let alice = "a".repeat(64);
    let bob = "b".repeat(64);
    state.note_success(&bob, "203.0.113.2:51820", 1_000);
    state.note_success(&alice, "203.0.113.1:51820", 1_000);
    state.note_success(&alice, "198.51.100.5:51820", 2_000);

    let emitted = state.as_static_peer_endpoints();
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted[0].0, alice);
    assert_eq!(emitted[1].0, bob);
    // Endpoints inside each peer come out sorted+deduped.
    let mut alice_eps = emitted[0].1.clone();
    alice_eps.sort();
    assert_eq!(
        alice_eps,
        vec![
            "198.51.100.5:51820".to_string(),
            "203.0.113.1:51820".to_string()
        ]
    );
}
