use crate::*;
#[cfg(feature = "embedded-fips")]
use fips_core::discovery::nostr::{OverlayEndpointAdvert, OverlayTransportKind};
#[cfg(feature = "paid-exit")]
use futures_util::{SinkExt, StreamExt};
#[cfg(feature = "embedded-fips")]
use nostr_sdk::prelude::{Keys, ToBech32};
#[cfg(feature = "embedded-fips")]
use std::collections::HashSet;
#[cfg(feature = "embedded-fips")]
use std::net::Ipv4Addr;
use std::path::Path;
#[cfg(feature = "paid-exit")]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[cfg(feature = "paid-exit")]
use tokio::net::TcpListener;
#[cfg(feature = "paid-exit")]
use tokio::sync::oneshot;
#[cfg(feature = "paid-exit")]
use tokio_tungstenite::tungstenite::Message;

#[test]
fn daemon_vpn_requires_remote_participants_to_be_active() {
    assert!(!daemon_vpn_active(true, 0));
    assert!(daemon_vpn_active(true, 1));
    assert!(!daemon_vpn_active(false, 1));
}

#[test]
fn daemon_vpn_idle_status_distinguishes_waiting_from_paused() {
    assert_eq!(
        daemon_vpn_idle_status(true, 0, false),
        crate::WAITING_FOR_PARTICIPANTS_STATUS
    );
    assert_eq!(
        daemon_vpn_idle_status(false, 0, true),
        "Listening for join requests"
    );
    assert_eq!(daemon_vpn_idle_status(false, 0, false), "Paused");
    assert_eq!(daemon_vpn_idle_status(true, 2, false), "Paused");
}

#[test]
fn fips_private_runtime_active_tolerates_no_active_network() {
    let mut app = AppConfig::generated();
    app.fips_host_tunnel_enabled = false;
    for network in &mut app.networks {
        network.listen_for_join_requests = false;
    }

    assert!(app.active_network_opt().is_none());
    assert!(!fips_private_runtime_active(&app, true, 0));

    let network_id = app.networks[0].id.clone();
    app.set_network_enabled(&network_id, true)
        .expect("enable network");
    app.set_network_join_requests_enabled(&network_id, true)
        .expect("enable join requests");
    assert!(fips_private_runtime_active(&app, false, 0));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_roster_publish_attempts_disconnected_recipients() {
    let recipients = vec!["alice".to_string(), "bob".to_string()];

    let (ready, pending) = split_ready_fips_roster_recipients(recipients.clone());

    assert_eq!(ready, recipients);
    assert!(pending.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_share_public_configured_endpoint_with_roster() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "89.27.103.157:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)], &[]);
    let addrs = hints.into_iter().map(|hint| hint.addr).collect::<Vec<_>>();

    assert_eq!(
        addrs,
        vec![
            "192.168.50.10:51820".to_string(),
            "89.27.103.157:51820".to_string(),
        ]
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_share_fips_advertised_udp_endpoint_with_roster() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;
    let advertised = vec![
        OverlayEndpointAdvert {
            transport: OverlayTransportKind::Udp,
            addr: "8.8.8.8:51820".to_string(),
        },
        OverlayEndpointAdvert {
            transport: OverlayTransportKind::Udp,
            addr: "nat".to_string(),
        },
        OverlayEndpointAdvert {
            transport: OverlayTransportKind::Tcp,
            addr: "8.8.4.4:443".to_string(),
        },
    ];

    let hints = local_fips_endpoint_hints(&app, Vec::new(), &advertised);
    let addrs = hints.into_iter().map(|hint| hint.addr).collect::<Vec<_>>();

    assert_eq!(addrs, vec!["8.8.8.8:51820"]);
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_lan_when_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)], &[]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_keep_configured_lan_when_lan_discovery_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "192.168.50.22:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)], &[]);

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "192.168.50.22:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_cgnat_candidates() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(100, 120, 94, 10)], &[]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_loopback_when_lan_enabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, Vec::new(), &[]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_tunnel_endpoint() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "10.44.1.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, Vec::new(), &[]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_keep_dns_endpoint_and_listen_port() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "peer.example.com:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, Vec::new(), &[]);

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "peer.example.com:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_keep_local_non_tunnel_addresses() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(192, 168, 50, 10)), "10.44.1.1/32");

    assert!(candidates.contains(&Ipv4Addr::new(192, 168, 50, 10)));
    assert!(!candidates.contains(&Ipv4Addr::new(10, 44, 1, 1)));
    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_drop_detected_cgnat_address() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(100, 120, 94, 10)), "10.44.1.1/32");

    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn endpoint_hint_recipients_are_active_participants_only() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let admin = Keys::generate();
    let own_pubkey = own.public_key().to_hex();
    let peer_pubkey = peer.public_key().to_hex();
    let admin_pubkey = admin.public_key().to_hex();
    let mut app = AppConfig::generated();
    let network_id = app.networks[0].id.clone();
    app.set_network_enabled(&network_id, true)
        .expect("activate first network");
    app.nostr.secret_key = own.secret_key().to_bech32().expect("own nsec");
    app.nostr.public_key = own_pubkey.clone();
    app.networks[0].devices = vec![own_pubkey.clone(), peer_pubkey.clone()];
    app.networks[0].admins = vec![admin_pubkey.clone()];

    let recipients = desired_fips_endpoint_hint_recipients(&app);

    assert_eq!(recipients, HashSet::from([peer_pubkey]));
    assert!(!recipients.contains(&own_pubkey));
    assert!(!recipients.contains(&admin_pubkey));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn endpoint_hint_capability_refreshes_only_matching_roster_peer() {
    use nostr_vpn_core::fips_control::{PeerCapabilities, PeerEndpointHint};

    let roster_peer = "11".repeat(32);
    let other_peer = "22".repeat(32);
    let recipients = HashSet::from([roster_peer.clone()]);
    let capabilities = PeerCapabilities {
        endpoint_hints: vec![PeerEndpointHint::udp("192.168.50.10:51820")],
        ..PeerCapabilities::default()
    };

    assert_eq!(
        endpoint_hint_refresh_participant(
            Some("network-a"),
            &recipients,
            &roster_peer,
            "network-a",
            &capabilities,
        ),
        Some(roster_peer.clone())
    );
    assert_eq!(
        endpoint_hint_refresh_participant(
            Some("network-a"),
            &recipients,
            &roster_peer,
            "network-b",
            &capabilities,
        ),
        None
    );
    assert_eq!(
        endpoint_hint_refresh_participant(
            Some("network-a"),
            &recipients,
            &other_peer,
            "network-a",
            &capabilities,
        ),
        None
    );
    assert_eq!(
        endpoint_hint_refresh_participant(
            Some("network-a"),
            &recipients,
            &roster_peer,
            "network-a",
            &PeerCapabilities::default(),
        ),
        None
    );
}

#[cfg(all(feature = "embedded-fips", feature = "paid-exit"))]
#[test]
fn fips_tunnel_config_carries_paid_route_payment_streaming_inputs() {
    let own = Keys::generate();
    let own_pubkey = own.public_key().to_hex();
    let mut app = AppConfig::generated();
    let network_id = app.networks[0].network_id.clone();
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.nostr.secret_key = own.secret_key().to_bech32().expect("own nsec");
    app.nostr.public_key = own_pubkey.clone();
    app.nostr.relays = vec![
        " wss://relay.example ".to_string(),
        "wss://disabled.example".to_string(),
    ];
    app.nostr.disabled_relays = vec!["wss://disabled.example".to_string()];
    app.paid_exit.enabled = true;
    app.paid_exit.pricing.price_msat = 123;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-fips-paid-route-streaming-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");

    let config = fips_tunnel_config_from_app(
        &app,
        &config_path,
        &network_id,
        "utun-test",
        Some(&own_pubkey),
        None,
        &[],
    )
    .expect("build fips config");

    assert_eq!(
        config.paid_route_store_path,
        paid_route_store_file_path(&config_path)
    );
    assert_eq!(
        config.paid_route_wallet_data_dir,
        paid_exit_wallet_data_dir(&config_path)
    );
    assert_eq!(
        config.paid_route_payment_relays,
        vec!["wss://relay.example".to_string()]
    );
    assert_eq!(config.paid_exit.pricing.price_msat, 123);
    assert_eq!(config.identity_nsec, app.nostr.secret_key);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_nonzero_pid_rejects_zero_and_invalid_values() {
    assert_eq!(parse_nonzero_pid("4242"), Some(4242));
    assert_eq!(parse_nonzero_pid("0"), None);
    assert_eq!(parse_nonzero_pid("not-a-number"), None);
}

#[test]
fn wall_time_jump_detection_flags_sleep_resume_after_threshold() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        0,
        1_000,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS
    ));
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
    assert!(wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[test]
fn wall_time_jump_detection_ignores_busy_loop_delays() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[test]
fn daemon_network_refresh_cadence_keeps_link_changes_low_latency() {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 15);
        const {
            assert!(DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS <= 1_000);
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 1);
}

#[test]
fn macos_underlay_route_check_throttles_route_event_storms() {
    assert_eq!(MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS, 5);

    let start = Instant::now();
    let mut last_check_at = start;

    assert!(!macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(1),
    ));
    assert_eq!(last_check_at, start);

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(5),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(5));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        true,
        false,
        start + Duration::from_secs(6),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(6));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        true,
        start + Duration::from_secs(7),
    ));
}

#[test]
fn macos_underlay_route_repair_defers_only_for_confirmed_captive_portal() {
    assert!(!macos_underlay_route_repair_allowed(Some(true)));
    assert!(macos_underlay_route_repair_allowed(Some(false)));
    assert!(macos_underlay_route_repair_allowed(None));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_refresh_paths_for_major_link_changes() {
    assert_eq!(
        fips_link_event_refresh(true, false, false, false),
        FipsLinkEventRefresh::RefreshPaths
    );
    assert_eq!(
        fips_link_event_refresh(false, false, true, false),
        FipsLinkEventRefresh::RefreshPaths
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, true),
        FipsLinkEventRefresh::RefreshPaths
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_refresh_paths_for_endpoint_only_changes() {
    assert_eq!(
        fips_link_event_refresh(false, true, false, false),
        FipsLinkEventRefresh::RefreshPaths
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, false),
        FipsLinkEventRefresh::None
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_stale_participant_recovery_is_cooldown_gated() {
    let mut last_restart_at = None;

    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000
    ));
    assert_eq!(last_restart_at, Some(1_000));
    assert!(!fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000 + FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS - 1
    ));
    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000 + FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS
    ));
    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        900
    ));
}

#[cfg(feature = "embedded-fips")]
fn pending_fips_peer(pubkey: &str) -> MeshPeerStatus {
    MeshPeerStatus {
        pubkey: pubkey.to_string(),
        connected: false,
        endpoint_npub: format!("npub1{pubkey}"),
        transport_addr: None,
        transport_type: None,
        srtt_ms: None,
        srtt_age_ms: None,
        link_packets_sent: 0,
        link_packets_recv: 0,
        link_bytes_sent: 0,
        link_bytes_recv: 0,
        rekey_in_progress: false,
        rekey_draining: false,
        current_k_bit: None,
        last_outbound_route: None,
        direct_probe_pending: true,
        direct_probe_after_ms: Some(1_234),
        direct_probe_retry_count: 4,
        direct_probe_auto_reconnect: true,
        direct_probe_expires_at_ms: Some(5_678),
        nostr_traversal_consecutive_failures: 1,
        nostr_traversal_in_cooldown: false,
        nostr_traversal_cooldown_until_ms: None,
        nostr_traversal_last_observed_skew_ms: None,
        last_seen_at: None,
        last_control_seen_at: None,
        last_data_seen_at: None,
        tx_bytes: 1024,
        rx_bytes: 0,
        error: Some("fips link pending".to_string()),
    }
}

#[cfg(feature = "embedded-fips")]
fn connected_relay() -> DaemonRelayState {
    DaemonRelayState {
        url: "wss://relay.example".to_string(),
        status: "connected".to_string(),
    }
}

#[cfg(feature = "embedded-fips")]
fn roster_pubkeys(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_waits_for_grace_and_cooldown() {
    let peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let relays = vec![connected_relay()];
    let roster = roster_pubkeys(&["a", "b"]);
    let mut state = FipsPendingRosterRestartState::default();
    let start = 10_000;

    assert!(!fips_pending_roster_restart_due(
        &peers, &relays, &roster, 2, &mut state, start
    ));
    assert!(!fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS - 1
    ));
    assert!(fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
    assert!(!fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS + 1
    ));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_requires_connected_relay_and_all_pending() {
    let mut state = FipsPendingRosterRestartState::default();
    let disconnected_relay = DaemonRelayState {
        url: "wss://relay.example".to_string(),
        status: "disconnected".to_string(),
    };
    let peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let roster = roster_pubkeys(&["a", "b"]);

    assert!(!fips_pending_roster_restart_due(
        &peers,
        &[disconnected_relay],
        &roster,
        2,
        &mut state,
        10_000 + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));

    let mut partly_connected = peers.clone();
    partly_connected[0].connected = true;
    partly_connected[0].error = None;
    assert!(!fips_pending_roster_restart_due(
        &partly_connected,
        &[connected_relay()],
        &roster,
        2,
        &mut state,
        20_000
    ));

    let one_peer_missing_from_snapshot = vec![pending_fips_peer("a")];
    assert!(!fips_pending_roster_restart_due(
        &one_peer_missing_from_snapshot,
        &[connected_relay()],
        &roster,
        2,
        &mut state,
        30_000 + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_ignores_connected_non_roster_transit() {
    let mut peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let mut transit = pending_fips_peer("transit");
    transit.connected = true;
    transit.error = None;
    transit.last_seen_at = Some(10_000);
    peers.push(transit);

    let relays = vec![connected_relay()];
    let roster = roster_pubkeys(&["a", "b"]);
    let mut state = FipsPendingRosterRestartState::default();
    let start = 40_000;

    assert!(!fips_pending_roster_restart_due(
        &peers, &relays, &roster, 2, &mut state, start
    ));
    assert!(fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn runtime_exit_node_routes_do_not_advertise_ipv6_default() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    assert_eq!(runtime_exit_node_default_routes(), vec!["0.0.0.0/0"]);
    assert_eq!(runtime_effective_advertised_routes(&app), vec!["0.0.0.0/0"]);
}

#[test]
fn legacy_macos_exit_cleanup_leaves_global_ipv4_forwarding_alone() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    let plan = legacy_macos_exit_cleanup_plan(&runtime_effective_advertised_routes(&app));

    assert!(plan.cleanup_pf_nat);
    assert!(!plan.restore_ipv4_forwarding);
}

#[test]
fn macos_exit_node_pf_rules_are_scoped_to_tunnel_source_and_outbound_iface() {
    let rules = crate::macos_network::macos_exit_node_pf_rules("utun42", "en0", "10.44.0.0/16");

    assert_eq!(
        rules,
        concat!(
            "nat on en0 inet from 10.44.0.0/16 to any -> (en0)\n",
            "pass in quick on utun42 inet from 10.44.0.0/16 to any keep state\n",
            "pass out quick on en0 inet from 10.44.0.0/16 to any keep state\n",
        )
    );
    assert!(!rules.contains("net.inet.ip.forwarding"));
    assert!(!rules.contains("pass in quick on en0"));
}

#[test]
fn macos_exit_node_cleanup_flushes_only_nvpn_anchor() {
    assert_eq!(
        crate::macos_network::macos_pf_anchor_flush_args(),
        vec!["-a", "com.apple/to.nostrvpn/exit", "-F", "all"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_underlay_repair_resets_tunnel_runtime() {
    let mut runtime = CliTunnelRuntime::new("utun4");
    runtime.active_listen_port = Some(51820);

    crate::session_runtime::reset_tunnel_runtime_after_macos_underlay_repair(&mut runtime);

    assert!(runtime.active_listen_port.is_none());
}

#[test]
fn macos_connect_privilege_preflight_requires_admin_when_euid_is_not_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(501));

    let error = crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect_err("non-root macOS preflight should fail");
    let message = error.to_string();
    assert!(message.contains("admin privileges"));
    assert!(message.contains("did you run with sudo?"));
    assert!(message.contains("sudo nvpn start --connect"));
    assert!(message.contains("sudo nvpn service install"));

    crate::set_macos_euid_override_for_test(None);
}

#[test]
fn macos_connect_privilege_preflight_allows_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(0));

    crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect("root macOS preflight should pass");

    crate::set_macos_euid_override_for_test(None);
}
