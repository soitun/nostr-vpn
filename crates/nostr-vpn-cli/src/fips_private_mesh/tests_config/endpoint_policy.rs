
    #[test]
    fn endpoint_peer_hints_prefer_configured_private_addresses() {
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            &[],
            vec![("peer".to_string(), vec!["192.168.178.91:51830".to_string()])],
            vec![(
                "peer".to_string(),
                vec![("89.27.103.157:33838".to_string(), 123_000)],
            )],
        );

        let peer = endpoint_peers
            .iter()
            .find(|peer| peer.npub == "peer")
            .expect("peer");
        let static_hint = peer
            .addresses
            .iter()
            .find(|hint| hint.addr == "192.168.178.91:51830")
            .expect("static hint");
        let recent_hint = peer
            .addresses
            .iter()
            .find(|hint| hint.addr == "89.27.103.157:33838")
            .expect("recent hint");

        assert_eq!(
            static_hint.priority,
            FIPS_CONFIGURED_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(recent_hint.priority, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY);
        assert_eq!(
            fips_peer_address_from_hint(static_hint).priority,
            FIPS_CONFIGURED_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(
            fips_peer_address_from_hint(recent_hint).priority,
            FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY
        );
    }

    #[test]
    fn recent_cache_does_not_create_configured_peer_membership() {
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            &[],
            Vec::new(),
            vec![(
                "peer".to_string(),
                vec![("192.168.178.91:51830".to_string(), 123_000)],
            )],
        );

        assert!(endpoint_peers.is_empty());
    }

    #[test]
    fn endpoint_peer_hints_treat_public_static_addresses_as_hints() {
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            &[],
            vec![("peer".to_string(), vec!["198.51.100.91:51830".to_string()])],
            vec![(
                "peer".to_string(),
                vec![("89.27.103.157:33838".to_string(), 123_000)],
            )],
        );

        let peer = endpoint_peers
            .iter()
            .find(|peer| peer.npub == "peer")
            .expect("peer");
        let static_hint = peer
            .addresses
            .iter()
            .find(|hint| hint.addr == "198.51.100.91:51830")
            .expect("static hint");
        let recent_hint = peer
            .addresses
            .iter()
            .find(|hint| hint.addr == "89.27.103.157:33838")
            .expect("recent hint");

        assert_eq!(static_hint.priority, FIPS_CONFIGURED_PEER_ENDPOINT_PRIORITY);
        assert_eq!(recent_hint.priority, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY);
    }

    #[test]
    fn endpoint_peer_hints_keep_operator_static_duplicate_unstamped() {
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            &[],
            vec![("peer".to_string(), vec!["198.51.100.91:51830".to_string()])],
            vec![(
                "peer".to_string(),
                vec![
                    ("198.51.100.91:51830".to_string(), 123_000),
                    ("198.51.100.91:51830".to_string(), 456_000),
                ],
            )],
        );

        let peer = endpoint_peers
            .iter()
            .find(|peer| peer.npub == "peer")
            .expect("peer");
        let matching_hints = peer
            .addresses
            .iter()
            .filter(|hint| hint.addr == "198.51.100.91:51830")
            .collect::<Vec<_>>();

        assert_eq!(matching_hints.len(), 1);
        assert_eq!(matching_hints[0].seen_at_ms, None);
        assert_eq!(
            matching_hints[0].priority,
            FIPS_CONFIGURED_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(
            fips_peer_address_from_hint(matching_hints[0]).seen_at_ms,
            None
        );
    }

    #[test]
    fn static_endpoint_filter_drops_stale_private_hints() {
        let mut tunnel_ips = HashSet::new();
        tunnel_ips.insert(IpAddr::V4(Ipv4Addr::new(10, 44, 1, 2)));
        let local_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(192, 168, 50, 10), 24)];

        let filtered = filter_static_tunnel_endpoints_with_policy_and_route_check(
            vec![(
                "peer".to_string(),
                vec![
                    "192.168.50.57:51820".to_string(),
                    "udp:192.168.50.58:51820".to_string(),
                    "192.168.51.57:51820".to_string(),
                    "100.120.94.10:51820".to_string(),
                    "10.44.1.2:51820".to_string(),
                    "203.0.113.9:51820".to_string(),
                    "peer.example.com:443".to_string(),
                ],
            )],
            &tunnel_ips,
            &local_subnets,
            false,
            |_| true,
        );

        assert_eq!(
            filtered,
            vec![(
                "peer".to_string(),
                vec![
                    "192.168.50.57:51820".to_string(),
                    "udp:192.168.50.58:51820".to_string(),
                    "203.0.113.9:51820".to_string(),
                    "peer.example.com:443".to_string(),
                ],
            )]
        );
    }

    #[test]
    fn static_endpoint_filter_keeps_routed_private_hint_in_static_mode() {
        let mut tunnel_ips = HashSet::new();
        tunnel_ips.insert(IpAddr::V4(Ipv4Addr::new(10, 44, 1, 2)));
        let local_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(172, 17, 0, 2), 16)];

        let filtered = filter_static_tunnel_endpoints_with_policy(
            vec![(
                "peer".to_string(),
                vec![
                    "192.168.64.5:51874".to_string(),
                    "10.44.1.2:51820".to_string(),
                ],
            )],
            &tunnel_ips,
            &local_subnets,
            true,
        );

        assert_eq!(
            filtered,
            vec![("peer".to_string(), vec!["192.168.64.5:51874".to_string()],)]
        );
    }

    #[test]
    fn static_endpoint_filter_keeps_private_hints_on_explicit_local_routes() {
        let tunnel_ips = HashSet::new();
        let local_subnets = vec![
            Ipv4Subnet::new(Ipv4Addr::new(192, 168, 178, 57), 24),
            Ipv4Subnet::new(Ipv4Addr::new(192, 168, 122, 0), 24),
        ];

        let filtered = filter_static_tunnel_endpoints_with_policy_and_route_check(
            vec![(
                "peer".to_string(),
                vec![
                    "192.168.122.103:51820".to_string(),
                    "192.168.123.103:51820".to_string(),
                ],
            )],
            &tunnel_ips,
            &local_subnets,
            false,
            |_| true,
        );

        assert_eq!(
            filtered,
            vec![(
                "peer".to_string(),
                vec!["192.168.122.103:51820".to_string()],
            )]
        );
    }

    #[test]
    fn linux_route_parser_keeps_private_routes_via_local_underlay_gateway() {
        let interface_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(192, 168, 178, 57), 24)];
        let routes = linux_private_ipv4_route_subnets_from_ip_route(
            "\
default via 192.168.178.1 dev eno1
192.168.122.0/24 via 192.168.178.91 dev eno1
192.168.123.0/24 via 10.0.0.1 dev eno1
172.19.0.0/16 dev virbr0 proto kernel scope link src 172.19.0.1
10.44.4.97 dev utun100 scope link src 10.44.67.51
",
            &interface_subnets,
        );

        assert!(routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(192, 168, 122, 0), 24)));
        assert!(routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(172, 19, 0, 0), 16)));
        assert!(!routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(192, 168, 123, 0), 24)));
        assert!(!routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(10, 44, 4, 97), 32)));
    }

    #[test]
    fn macos_route_parser_keeps_private_routes_via_local_underlay_gateway() {
        let interface_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(192, 168, 178, 57), 24)];
        let routes = macos_private_ipv4_route_subnets_from_netstat(
            "\
Destination        Gateway            Flags               Netif Expire
default            192.168.178.1      UGScg                 en0
10.44.4.97         utun5              UHS                 utun5
192.168.122        192.168.178.91     UGSc                  en0
192.168.123        10.0.0.1           UGSc                  en0
192.168.178        link#7             UCS                   en0      !
",
            &interface_subnets,
        );

        assert!(routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(192, 168, 122, 0), 24)));
        assert!(routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(192, 168, 178, 0), 24)));
        assert!(!routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(192, 168, 123, 0), 24)));
        assert!(!routes.contains(&Ipv4Subnet::new(Ipv4Addr::new(10, 44, 4, 97), 32)));
    }

    #[test]
    fn macos_route_get_rejects_private_endpoint_via_hotspot_default_gateway() {
        let route_get = "\
   route to: 192.168.178.57
destination: 192.168.178.57
    gateway: 172.20.10.1
  interface: en0
";

        assert!(!macos_route_get_has_direct_private_endpoint_route(
            route_get,
            Ipv4Addr::new(192, 168, 178, 57),
        ));
    }

    #[test]
    fn macos_route_get_keeps_private_endpoint_on_direct_lan() {
        let route_get = "\
   route to: 192.168.178.57
destination: 192.168.178.57
    gateway: 192.168.178.57
  interface: en0
";

        assert!(macos_route_get_has_direct_private_endpoint_route(
            route_get,
            Ipv4Addr::new(192, 168, 178, 57),
        ));
    }

    #[test]
    fn linux_route_get_rejects_private_endpoint_via_default_gateway() {
        assert!(!linux_route_get_has_direct_private_endpoint_route(
            "192.168.178.57 via 172.20.10.1 dev wlan0 src 172.20.10.2 uid 501"
        ));
    }

    #[test]
    fn linux_route_get_keeps_private_endpoint_on_direct_lan() {
        assert!(linux_route_get_has_direct_private_endpoint_route(
            "192.168.178.57 dev eth0 src 192.168.178.55 uid 1000"
        ));
    }

    #[test]
    fn stamped_endpoint_filter_drops_stale_private_hints() {
        let mut tunnel_ips = HashSet::new();
        tunnel_ips.insert(IpAddr::V4(Ipv4Addr::new(10, 44, 1, 2)));
        let local_subnets = Vec::new();

        let filtered = filter_stamped_tunnel_endpoints(
            vec![(
                "peer".to_string(),
                vec![
                    ("192.168.50.57:51820".to_string(), 100),
                    ("udp:192.168.50.58:51820".to_string(), 101),
                    ("192.168.51.57:51820".to_string(), 102),
                    ("172.19.0.1:51820".to_string(), 103),
                    ("100.120.94.10:51820".to_string(), 104),
                    ("10.44.1.2:51820".to_string(), 105),
                    ("203.0.113.9:51820".to_string(), 106),
                    ("peer.example.com:443".to_string(), 107),
                ],
            )],
            &tunnel_ips,
            &local_subnets,
        );

        assert_eq!(
            filtered,
            vec![(
                "peer".to_string(),
                vec![
                    ("203.0.113.9:51820".to_string(), 106),
                    ("peer.example.com:443".to_string(), 107),
                ],
            )]
        );
    }

    #[test]
    fn static_endpoint_filter_requires_subnet_and_direct_route_for_private_hints() {
        let local_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(192, 168, 50, 10), 24)];

        assert!(static_endpoint_allowed_on_current_underlay_with_route_check(
            "192.168.50.57:51820",
            &local_subnets,
            |_| true,
        ));
        assert!(!static_endpoint_allowed_on_current_underlay_with_route_check(
            "192.168.50.57:51820",
            &local_subnets,
            |_| false,
        ));
        assert!(!static_endpoint_allowed_on_current_underlay_with_route_check(
            "192.168.51.57:51820",
            &local_subnets,
            |_| true,
        ));
        assert!(static_endpoint_allowed_on_current_underlay_with_route_check(
            "203.0.113.9:51820",
            &local_subnets,
            |_| false,
        ));
    }

    #[test]
    fn tunnel_config_drops_overlay_tunnel_endpoint_hints() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let network_id = "fips-tunnel-hints-test";
        let bob_tunnel_ip = derive_mesh_tunnel_ip(network_id, &bob_pubkey).expect("bob tunnel ip");
        let bob_tunnel_endpoint = format!("{}:51820", strip_cidr(&bob_tunnel_ip));

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];
        app.fips_peer_endpoints.insert(
            bob_npub.clone(),
            vec![
                bob_tunnel_endpoint.clone(),
                "203.0.113.23:51820".to_string(),
            ],
        );

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[(
                bob_pubkey.clone(),
                vec![
                    (bob_tunnel_endpoint, 123_000),
                    ("203.0.113.24:51820".to_string(), 124_000),
                ],
            )],
        )
        .expect("fips tunnel config");

        let bob = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == bob_npub)
            .expect("bob endpoint peer");
        let addrs = bob
            .addresses
            .iter()
            .map(|hint| hint.addr.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            addrs,
            ["203.0.113.23:51820", "203.0.113.24:51820"]
        );
    }
