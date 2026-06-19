use crate::*;
#[cfg(feature = "embedded-fips")]
use nostr_sdk::prelude::{Keys, ToBech32};
#[cfg(feature = "embedded-fips")]
use std::collections::HashSet;
#[cfg(feature = "embedded-fips")]
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, Instant};

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

    app.networks[0].listen_for_join_requests = true;
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
fn local_fips_endpoint_hints_include_configured_and_lan_candidates() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "89.27.103.157:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);
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
fn local_fips_endpoint_hints_do_not_share_lan_when_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

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

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

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

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(100, 120, 94, 10)]);

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

    let hints = local_fips_endpoint_hints(&app, Vec::new());

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

    let hints = local_fips_endpoint_hints(&app, Vec::new());

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

    let hints = local_fips_endpoint_hints(&app, Vec::new());

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
fn fips_link_events_restart_endpoint_for_major_link_changes() {
    assert_eq!(
        fips_link_event_refresh(true, false, false, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, true, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, true),
        FipsLinkEventRefresh::RestartEndpoint
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_restart_endpoint_for_endpoint_only_changes() {
    assert_eq!(
        fips_link_event_refresh(false, true, false, false),
        FipsLinkEventRefresh::RestartEndpoint
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
