    #[test]
    fn tunnel_config_applies_live_endpoint_hints_only_for_network_signal_peers() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let admin_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let admin_pubkey = admin_keys.public_key().to_hex();
        let admin_npub = admin_keys.public_key().to_bech32().expect("admin npub");
        let network_id = "fips-live-hints-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];
        app.networks[0].admins = vec![admin_pubkey.clone()];

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[
                (
                    bob_pubkey.clone(),
                    vec![("203.0.113.22:51820".to_string(), 123_000)],
                ),
                (
                    admin_pubkey.clone(),
                    vec![("203.0.113.33:51820".to_string(), 123_000)],
                ),
            ],
        )
        .expect("fips tunnel config");

        let bob = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == bob_npub)
            .expect("bob endpoint peer");
        assert_eq!(bob.addresses.len(), 1);
        assert_eq!(bob.addresses[0].addr, "203.0.113.22:51820");
        assert_eq!(bob.addresses[0].seen_at_ms, Some(123_000));

        let admin = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == admin_npub)
            .expect("admin endpoint peer");
        assert_eq!(admin.addresses.len(), 1);
        assert_eq!(admin.addresses[0].addr, "203.0.113.33:51820");
        assert_eq!(admin.addresses[0].seen_at_ms, Some(123_000));
    }

    #[test]
    fn link_event_path_hint_refresh_does_not_require_endpoint_restart() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let network_id = "fips-link-refresh-restart-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];

        let current = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[(
                bob_pubkey.clone(),
                vec![("203.0.113.22:51820".to_string(), 123_000)],
            )],
        )
        .expect("current fips tunnel config");
        let refreshed = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[],
        )
        .expect("refreshed fips tunnel config");

        let current_bob = current
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == bob_npub)
            .expect("current bob endpoint peer");
        assert_eq!(current_bob.addresses.len(), 1);
        let refreshed_bob = refreshed
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == bob_npub)
            .expect("refreshed bob endpoint peer");
        assert!(
            refreshed_bob.addresses.is_empty(),
            "link-event refreshes must not carry stale live direct hints forward",
        );
        assert!(
            !fips_tunnel_requires_endpoint_restart(&current, &refreshed),
            "path-hint-only refreshes should be applied in place, not by restarting FIPS",
        );

        let mut changed_port = refreshed.clone();
        changed_port.listen_port = changed_port.listen_port.saturating_add(1);
        assert!(
            fips_tunnel_requires_endpoint_restart(&refreshed, &changed_port),
            "transport bind changes still require a real endpoint restart",
        );
    }

    #[test]
    fn tunnel_config_keeps_static_endpoint_hint_for_control_only_admin() {
        let alice_keys = Keys::generate();
        let admin_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let admin_pubkey = admin_keys.public_key().to_hex();
        let admin_npub = admin_keys.public_key().to_bech32().expect("admin npub");
        let network_id = "fips-admin-static-hints-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = Vec::new();
        app.networks[0].admins = vec![admin_pubkey.clone()];
        app.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
            recipient: admin_pubkey.clone(),
            requested_at: 1,
        });
        app.fips_peer_endpoints
            .insert(admin_npub.clone(), vec!["203.0.113.10:51820".to_string()]);

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[],
        )
        .expect("fips tunnel config");

        assert!(
            config.peers.iter().all(|peer| peer.allowed_ips.is_empty()),
            "join-request admin peers must not get private-network routes before roster acceptance",
        );
        let admin = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == admin_npub)
            .expect("admin endpoint peer");
        assert_eq!(admin.addresses.len(), 1);
        assert_eq!(admin.addresses[0].addr, "203.0.113.10:51820");
        assert_eq!(admin.addresses[0].seen_at_ms, None);
    }

    #[test]
    fn tunnel_config_seeds_recent_outside_roster_transit_peers() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let charlie_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let charlie_pubkey = charlie_keys.public_key().to_hex();
        let charlie_npub = charlie_keys.public_key().to_bech32().expect("charlie npub");
        let network_id = "fips-recent-transit-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.connect_to_non_roster_fips_peers = true;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];

        let mut recent = nostr_vpn_core::recent_peers::RecentPeerEndpoints::default();
        assert!(recent.note_success(&charlie_pubkey, "203.0.113.55:51820", 123));

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            Some(&recent),
            &[],
        )
        .expect("fips tunnel config");

        assert!(
            config
                .peers
                .iter()
                .all(|peer| peer.participant_pubkey != charlie_pubkey),
            "non-roster transit peers must not get private-network routes",
        );
        let charlie = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == charlie_npub)
            .expect("recent non-roster peer should seed endpoint config");
        assert_eq!(charlie.addresses.len(), 1);
        assert_eq!(charlie.addresses[0].addr, "203.0.113.55:51820");
        assert_eq!(charlie.addresses[0].seen_at_ms, Some(123_000));
        assert!(
            !charlie.auto_reconnect,
            "recent transit-only peers should not retry forever"
        );
        assert!(
            charlie.discovery_fallback_transit,
            "recent non-roster peers should receive fallback lookup fanout"
        );
    }

    #[test]
    fn tunnel_config_drops_non_roster_transit_when_discovery_not_open() {
        if std::env::var("NVPN_FIPS_NOSTR_DISCOVERY_POLICY").is_ok() {
            return;
        }

        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let charlie_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let charlie_pubkey = charlie_keys.public_key().to_hex();
        let charlie_npub = charlie_keys.public_key().to_bech32().expect("charlie npub");
        let network_id = "fips-configured-only-transit-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.connect_to_non_roster_fips_peers = false;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];
        app.fips_bootstrap_peers.clear();
        app.fips_bootstrap_peers.insert(
            charlie_npub.clone(),
            vec!["203.0.113.55:51820".to_string()],
        );

        let mut recent = nostr_vpn_core::recent_peers::RecentPeerEndpoints::default();
        assert!(recent.note_success(&bob_pubkey, "1.1.1.1:51820", 123));
        assert!(recent.note_success(&charlie_pubkey, "203.0.113.66:51820", 456));

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            Some(&recent),
            &[],
        )
        .expect("fips tunnel config");

        assert_eq!(config.nostr_discovery_policy, NostrDiscoveryPolicy::ConfiguredOnly);
        assert_eq!(config.open_discovery_max_pending, 0);
        assert!(
            config.endpoint_peers.iter().any(|peer| peer.npub == bob_npub),
            "roster recent hints should still be retained"
        );
        assert!(
            config.endpoint_peers.iter().all(|peer| peer.npub != charlie_npub),
            "configured-only discovery must not seed non-roster transit peers"
        );
    }

    #[test]
    fn tunnel_config_caps_recent_outside_roster_transit_peers() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let network_id = "fips-recent-transit-cap-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.connect_to_non_roster_fips_peers = true;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];
        app.fips_bootstrap_enabled = false;

        let mut recent = nostr_vpn_core::recent_peers::RecentPeerEndpoints::default();
        assert!(recent.note_success(&bob_pubkey, "1.1.1.1:51820", 1));

        let mut non_roster_npubs = Vec::new();
        for i in 0..(FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING + 2) {
            let keys = Keys::generate();
            let pubkey = keys.public_key().to_hex();
            let npub = keys.public_key().to_bech32().expect("transit npub");
            let addr = format!("1.1.1.{}:51820", i + 2);
            assert!(recent.note_success(&pubkey, &addr, 100 + i as u64));
            non_roster_npubs.push(npub);
        }

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            Some(&recent),
            &[],
        )
        .expect("fips tunnel config");

        let bob = config
            .endpoint_peers
            .iter()
            .find(|peer| peer.npub == bob_npub)
            .expect("roster peer recent hint should be retained");
        assert_eq!(bob.addresses.len(), 1);
        assert_eq!(bob.addresses[0].addr, "1.1.1.1:51820");

        let seeded_recent_non_roster = config
            .endpoint_peers
            .iter()
            .filter(|peer| non_roster_npubs.iter().any(|npub| npub == &peer.npub))
            .collect::<Vec<_>>();
        assert_eq!(
            seeded_recent_non_roster.len(),
            FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS,
            "recent non-roster transit cache should not consume the whole open-discovery cap"
        );
        assert_eq!(
            config.open_discovery_max_pending,
            FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING - FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS,
            "recent transit seeds should leave a fixed budget for fresh open discovery"
        );
        assert!(
            !seeded_recent_non_roster
                .iter()
                .any(|peer| peer.npub == non_roster_npubs[0]),
            "oldest non-roster transit hint should be dropped first"
        );
        assert!(
            seeded_recent_non_roster
                .iter()
                .any(|peer| peer.npub == *non_roster_npubs.last().unwrap()),
            "freshest non-roster transit hint should be retained"
        );
    }

    #[test]
    fn tunnel_config_caps_bootstrap_transit_peers_without_exhausting_open_discovery() {
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let network_id = "fips-bootstrap-transit-cap-test";

        let mut app = AppConfig::default();
        app.nostr.secret_key = alice_nsec;
        app.connect_to_non_roster_fips_peers = true;
        app.networks[0].enabled = true;
        app.networks[0].network_id = network_id.to_string();
        app.networks[0].devices = vec![alice_pubkey.clone(), bob_pubkey.clone()];
        app.fips_bootstrap_peers.clear();

        let mut bootstrap_npubs = Vec::new();
        for i in 0..(FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING + 2) {
            let keys = Keys::generate();
            let npub = keys.public_key().to_bech32().expect("bootstrap npub");
            app.fips_bootstrap_peers.insert(
                npub.clone(),
                vec![format!("203.0.113.{}:51820", i + 10)],
            );
            bootstrap_npubs.push(npub);
        }

        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            network_id,
            "utun-test",
            Some(&alice_pubkey),
            None,
            &[],
        )
        .expect("fips tunnel config");

        let seeded_bootstrap = config
            .endpoint_peers
            .iter()
            .filter(|peer| bootstrap_npubs.iter().any(|npub| npub == &peer.npub))
            .collect::<Vec<_>>();
        assert_eq!(
            seeded_bootstrap.len(),
            FIPS_STATIC_NON_ROSTER_TRANSIT_MAX_SEEDS,
            "bootstrap transit peers should not consume the whole open-discovery cap"
        );
        assert_eq!(
            config.open_discovery_max_pending,
            FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING
                - FIPS_STATIC_NON_ROSTER_TRANSIT_MAX_SEEDS
                - FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS,
            "bounded bootstrap transit should leave a nonzero fresh open-discovery budget"
        );
        assert!(config.open_discovery_max_pending > 0);
    }

    #[test]
    fn recent_transit_seed_cap_prefers_static_public_endpoints() {
        let capped = cap_recent_non_roster_transit_endpoints(
            vec![
                (
                    "fresh-ephemeral".to_string(),
                    vec![("203.0.113.10:62000".to_string(), 999_000)],
                ),
                (
                    "older-stable".to_string(),
                    vec![("203.0.113.11:51820".to_string(), 1_000)],
                ),
            ],
            &HashSet::new(),
            1,
        );

        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].0, "older-stable");
    }

    #[test]
    fn endpoint_peer_hints_make_private_static_addresses_last_resort() {
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
            FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(recent_hint.priority, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY);
        assert_eq!(
            fips_peer_address_from_hint(static_hint).priority,
            FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(
            fips_peer_address_from_hint(recent_hint).priority,
            FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY
        );
    }

    #[test]
    fn endpoint_peer_hints_make_private_recent_addresses_last_resort() {
        let endpoint_peers = fips_endpoint_peers_from_mesh(
            &[],
            Vec::new(),
            vec![(
                "peer".to_string(),
                vec![("192.168.178.91:51830".to_string(), 123_000)],
            )],
        );

        let peer = endpoint_peers
            .iter()
            .find(|peer| peer.npub == "peer")
            .expect("peer");
        let recent_hint = peer
            .addresses
            .iter()
            .find(|hint| hint.addr == "192.168.178.91:51830")
            .expect("recent hint");

        assert_eq!(recent_hint.seen_at_ms, Some(123_000));
        assert_eq!(
            recent_hint.priority,
            FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY
        );
        assert_eq!(
            fips_peer_address_from_hint(recent_hint).priority,
            FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY
        );
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

        assert_eq!(static_hint.priority, FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY);
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
            FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY
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

        let filtered = filter_static_tunnel_endpoints(
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

        let filtered = filter_static_tunnel_endpoints(
            vec![(
                "peer".to_string(),
                vec![
                    "192.168.122.103:51820".to_string(),
                    "192.168.123.103:51820".to_string(),
                ],
            )],
            &tunnel_ips,
            &local_subnets,
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
    fn stamped_endpoint_filter_drops_stale_private_hints() {
        let mut tunnel_ips = HashSet::new();
        tunnel_ips.insert(IpAddr::V4(Ipv4Addr::new(10, 44, 1, 2)));
        let local_subnets = vec![Ipv4Subnet::new(Ipv4Addr::new(192, 168, 50, 10), 24)];

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
                    ("192.168.50.57:51820".to_string(), 100),
                    ("udp:192.168.50.58:51820".to_string(), 101),
                    ("203.0.113.9:51820".to_string(), 106),
                    ("peer.example.com:443".to_string(), 107),
                ],
            )]
        );
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
        assert_eq!(addrs, vec!["203.0.113.23:51820", "203.0.113.24:51820"]);
    }

    /// Pin the open-discovery / closed-data-plane invariant.
    ///
    /// FIPS handshake is `Open` so any nvpn node we see on relays may
    /// connect to us (this is what enables transit through friend-of-a-friend
    /// peers). The data plane MUST stay closed: a packet whose FIPS source
    /// npub doesn't own its inner-source IP per the local roster is dropped
    /// before it reaches the tun. This test wires both halves together so a
    /// future "fix" that opens ambient discovery OR loosens the roster gate
    /// will fail loudly.
    ///
    /// The cross-platform integration variants (T1: live handshake, T4:
    /// transit through non-roster peer) live in the FIPS docker continuity
    /// suite — they need a real endpoint pair and can't run as unit tests.
    #[test]
    fn open_discovery_does_not_loosen_tun_roster_gate() {
        let roster_peer = Keys::generate();
        let stranger = Keys::generate();
        let roster_pubkey = roster_peer.public_key().to_hex();
        let roster_npub = roster_peer.public_key().to_bech32().expect("roster npub");
        let stranger_npub = stranger.public_key().to_bech32().expect("stranger npub");

        let mesh_peer = FipsMeshPeerConfig::from_participant_pubkey(
            &roster_pubkey,
            vec!["10.44.1.2/32".to_string()],
        )
        .expect("roster peer config");
        let endpoint_peers =
            fips_endpoint_peers_from_mesh(std::slice::from_ref(&mesh_peer), Vec::new(), Vec::new());
        let config = fips_endpoint_config(
            &endpoint_peers,
            None,
            super::resolve_private_mesh_mtu(None, None, None),
            NostrDiscoveryPolicy::Open,
        );

        assert_eq!(
            config.node.discovery.nostr.policy,
            fips_endpoint::NostrDiscoveryPolicy::Open,
            "open FIPS discovery must not loosen private roster traffic admission",
        );

        let mesh = FipsMeshRuntime::new(vec![mesh_peer.clone()]);

        // The roster peer's own packet is admitted.
        let mut packet = vec![0_u8; 28];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&28_u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&[10, 44, 1, 2]);
        packet[16..20].copy_from_slice(&[10, 44, 1, 1]);
        assert!(
            mesh.receive_endpoint_data(Some(&roster_npub), &packet)
                .is_some(),
            "roster peer's owned source IP must be admitted",
        );

        // A stranger that successfully completed the open FIPS handshake
        // still cannot inject anything onto our tun, regardless of inner
        // source IP.
        assert!(
            mesh.receive_endpoint_data(Some(&stranger_npub), &packet)
                .is_none(),
            "non-roster peer must not inject packets onto the tun",
        );

        let mut spoofed = packet.clone();
        spoofed[12..16].copy_from_slice(&[203, 0, 113, 9]);
        assert!(
            mesh.receive_endpoint_data(Some(&stranger_npub), &spoofed)
                .is_none(),
            "non-roster peer must not inject packets onto the tun (spoofed source)",
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test(flavor = "current_thread")]
    async fn mesh_recv_packet_forwarding_cooperates_under_hot_stream() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let sibling_polls = Arc::new(AtomicUsize::new(0));
        let sibling_polls_task = Arc::clone(&sibling_polls);
        let sibling = tokio::spawn(async move {
            loop {
                sibling_polls_task.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        });

        for _ in 0..512 {
            super::cooperate_after_mesh_recv_packet().await;
        }

        sibling.abort();
        assert!(
            sibling_polls.load(Ordering::Relaxed) > 0,
            "hot mesh packet forwarding must yield scheduler time to sibling tasks"
        );
    }
