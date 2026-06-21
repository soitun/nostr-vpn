#[test]
fn daemon_peer_age_secs_rejects_far_future_timestamp() {
    assert_eq!(daemon_peer_age_secs(120, 119), Some(1));
    assert_eq!(daemon_peer_age_secs(120, 122), Some(0));
    assert_eq!(daemon_peer_age_secs(120, 180), None);
}

#[test]
fn fips_runtime_state_counts_direct_roster_and_other_peers() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let roster_peer = Keys::generate().public_key().to_hex();
    let routed_roster_peer = Keys::generate().public_key().to_hex();
    let other_peer = Keys::generate().public_key().to_hex();
    config.networks[0].devices = vec![roster_peer.clone(), routed_roster_peer.clone()];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        2,
        &tunnel_runtime,
        &[
            MeshPeerStatus {
                pubkey: roster_peer,
                connected: true,
                endpoint_npub: "npub1roster".to_string(),
                transport_addr: Some("203.0.113.8:9000".to_string()),
                transport_type: Some("udp".to_string()),
                srtt_ms: Some(5),
                srtt_age_ms: Some(50),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                rekey_in_progress: true,
                rekey_draining: true,
                current_k_bit: Some(true),
                last_outbound_route: Some("fallback-route".to_string()),
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                direct_probe_retry_count: 0,
                direct_probe_auto_reconnect: false,
                direct_probe_expires_at_ms: None,
                nostr_traversal_consecutive_failures: 4,
                nostr_traversal_in_cooldown: true,
                nostr_traversal_cooldown_until_ms: Some(123_456),
                nostr_traversal_last_observed_skew_ms: Some(-125),
                last_seen_at: Some(100),
                last_control_seen_at: Some(90),
                last_data_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
            MeshPeerStatus {
                pubkey: routed_roster_peer,
                connected: true,
                endpoint_npub: "npub1routed".to_string(),
                transport_addr: None,
                transport_type: None,
                srtt_ms: Some(8),
                srtt_age_ms: Some(80),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
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
                last_seen_at: Some(100),
                last_control_seen_at: Some(90),
                last_data_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
            MeshPeerStatus {
                pubkey: other_peer,
                connected: true,
                endpoint_npub: "npub1other".to_string(),
                transport_addr: Some("203.0.113.9:9000".to_string()),
                transport_type: Some("udp".to_string()),
                srtt_ms: Some(13),
                srtt_age_ms: Some(130),
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
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
                last_seen_at: Some(100),
                last_control_seen_at: Some(90),
                last_data_seen_at: Some(100),
                tx_bytes: 0,
                rx_bytes: 0,
                error: None,
            },
        ],
        &[],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert_eq!(state.connected_peer_count, 2);
    assert_eq!(state.fips_direct_roster_peer_count, 1);
    assert_eq!(state.fips_other_peer_count, 1);
    let rekey_peer = state
        .peers
        .iter()
        .find(|peer| peer.fips_endpoint_npub == "npub1roster")
        .expect("roster peer status");
    assert!(rekey_peer.fips_rekey_in_progress);
    assert!(rekey_peer.fips_rekey_draining);
    assert_eq!(rekey_peer.fips_current_k_bit, Some(true));
    assert_eq!(rekey_peer.fips_last_outbound_route, "fallback-route");
    assert_eq!(rekey_peer.fips_nostr_traversal_failures, 4);
    assert!(rekey_peer.fips_nostr_traversal_in_cooldown);
    assert_eq!(
        rekey_peer.fips_nostr_traversal_cooldown_until_ms,
        Some(123_456)
    );
    assert_eq!(
        rekey_peer.fips_nostr_traversal_last_observed_skew_ms,
        Some(-125)
    );
    assert_eq!(rekey_peer.last_fips_control_seen_at, Some(90));
    assert_eq!(rekey_peer.last_fips_data_seen_at, Some(100));
}

#[test]
fn daemon_runtime_state_marks_peers_unreachable_when_vpn_is_off() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let peer_pubkey = Keys::generate().public_key().to_hex();
    config.networks[0].devices = vec![peer_pubkey.clone()];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");
    let peer_status = MeshPeerStatus {
        pubkey: peer_pubkey,
        connected: true,
        endpoint_npub: "npub1endpoint".to_string(),
        transport_addr: Some("127.0.0.1:9000".to_string()),
        transport_type: Some("loopback".to_string()),
        srtt_ms: Some(3),
        srtt_age_ms: Some(30),
        link_packets_sent: 10,
        link_packets_recv: 11,
        link_bytes_sent: 120,
        link_bytes_recv: 130,
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
        last_seen_at: Some(100),
        last_control_seen_at: Some(90),
        last_data_seen_at: Some(100),
        tx_bytes: 200,
        rx_bytes: 300,
        error: None,
    };

    let state = crate::build_daemon_runtime_state(
        &config,
        false,
        false,
        1,
        &tunnel_runtime,
        &[peer_status],
        &[],
        &std::collections::HashMap::new(),
        "Paused",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert!(!state.vpn_active);
    assert!(!state.vpn_enabled);
    assert_eq!(state.connected_peer_count, 0);
    assert!(!state.mesh_ready);
    assert_eq!(state.peers.len(), 1);
    assert!(!state.peers[0].reachable);
    assert!(state.peers[0].runtime_endpoint.is_none());
}

#[test]
fn macos_route_delete_error_is_absent_matches_missing_route_output() {
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -ifscope utun100 default\nstdout: not in table\nstderr:"
    ));
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -ifscope utun100 -net 0.0.0.0/1\nstdout: route: bad interface name\nstderr:"
    ));
    assert!(macos_route_delete_error_is_absent(
        "command failed: route -n delete -host 203.0.113.8\nstdout:\nstderr: route: writing to routing socket: No such process"
    ));
    assert!(!macos_route_delete_error_is_absent(
        "command failed: route -n delete -host 203.0.113.8\nstdout:\nstderr: permission denied"
    ));
}

#[test]
fn macos_default_routes_from_netstat_finds_underlay_and_utun_routes() {
    let routes = macos_default_routes_from_netstat(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
default            link#13            UCSIg               utun5\n\
default            link#26            UCSIg           bridge100      !\n",
    );

    assert_eq!(
        routes,
        vec![
            crate::MacosRouteSpec {
                gateway: Some("192.168.64.1".to_string()),
                interface: "en0".to_string(),
            },
            crate::MacosRouteSpec {
                gateway: None,
                interface: "utun5".to_string(),
            },
            crate::MacosRouteSpec {
                gateway: None,
                interface: "bridge100".to_string(),
            },
        ]
    );

    assert_eq!(
        macos_underlay_default_route_from_routes(&routes),
        Some(crate::MacosRouteSpec {
            gateway: Some("192.168.64.1".to_string()),
            interface: "en0".to_string(),
        })
    );
}

#[test]
fn macos_split_defaults_are_detected_from_netstat() {
    assert!(crate::macos_network::macos_has_tunnel_split_default_routes(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
    assert!(
        !crate::macos_network::macos_has_tunnel_split_default_routes(
            "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
"
        )
    );
}

#[test]
fn macos_underlay_default_route_detection_requires_real_underlay_route() {
    assert!(crate::macos_network::macos_has_underlay_default_route(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
default            192.168.64.1       UGScg                 en0\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
    assert!(!crate::macos_network::macos_has_underlay_default_route(
        "Routing tables\n\
Internet:\n\
Destination        Gateway            Flags               Netif Expire\n\
0/1                link#13            UCS                 utun5\n\
128/1              link#13            UCS                 utun5\n\
"
    ));
}

#[test]
fn macos_tunnel_default_route_targets_use_split_defaults() {
    assert_eq!(
        crate::macos_network::macos_tunnel_default_route_targets(),
        &["0.0.0.0/1", "128.0.0.0/1"]
    );
}

#[test]
fn macos_gateway_route_args_install_global_host_routes() {
    assert_eq!(
        crate::macos_network::macos_gateway_route_args_for_test(
            "add",
            "65.109.48.91/32",
            "192.168.64.1",
        ),
        vec![
            "-n".to_string(),
            "add".to_string(),
            "-host".to_string(),
            "65.109.48.91".to_string(),
            "192.168.64.1".to_string(),
        ]
    );
    assert_eq!(
        crate::macos_network::macos_gateway_route_args_for_test(
            "change",
            "0.0.0.0/0",
            "192.168.64.1",
        ),
        vec![
            "-n".to_string(),
            "change".to_string(),
            "default".to_string(),
            "192.168.64.1".to_string(),
        ]
    );
}

#[test]
fn macos_interface_names_from_ifconfig_list_parses_interfaces() {
    assert_eq!(
        crate::macos_network::macos_interface_names_from_ifconfig_list(
            "lo0 gif0 stf0 anpi0 en0 en1 utun0 utun100\n"
        ),
        vec![
            "lo0", "gif0", "stf0", "anpi0", "en0", "en1", "utun0", "utun100"
        ]
    );
}

#[test]
fn macos_ipconfig_router_from_output_parses_ip_and_ip_mult_formats() {
    assert_eq!(
        crate::macos_network::macos_ipconfig_router_from_output("router (ip): 192.168.64.1\n"),
        Some("192.168.64.1".parse().unwrap())
    );
    assert_eq!(
        crate::macos_network::macos_ipconfig_router_from_output(
            "router (ip_mult): {192.168.64.1}\n"
        ),
        Some("192.168.64.1".parse().unwrap())
    );
}

#[test]
fn macos_ifconfig_has_ipv4_matches_exact_interface_address() {
    let output = "utun5: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1380\n\
\tinet 10.44.10.23 --> 10.44.10.23 netmask 0xffffffff\n\
\tinet6 fe80::1%utun5 prefixlen 64 scopeid 0x8\n";

    assert!(macos_ifconfig_has_ipv4(
        output,
        Ipv4Addr::new(10, 44, 10, 23)
    ));
    assert!(!macos_ifconfig_has_ipv4(
        output,
        Ipv4Addr::new(10, 44, 10, 24)
    ));
}

#[test]
fn macos_tunnel_ipv4_netmask_uses_host_route() {
    assert_eq!(crate::macos_tunnel_ipv4_netmask(), "255.255.255.255");
}
