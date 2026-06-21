
    #[test]
    fn mobile_endpoint_send_run_batches_consecutive_resolved_peer() {
        let participant = Keys::generate().public_key().to_hex();
        let participant_key = mobile_participant_pubkey_bytes(&participant).expect("participant");
        let endpoint_npub = Keys::generate().public_key().to_bech32().expect("npub");
        let identity = PeerIdentity::from_npub(&endpoint_npub).expect("peer identity");
        let endpoint_node_addr = *identity.node_addr().as_bytes();
        let mut identity_map = MobilePeerIdentityMap::default();
        identity_map
            .by_endpoint_node_addr
            .insert(endpoint_node_addr, identity);
        let identities = Arc::new(RwLock::new(identity_map));
        let mut run = None;

        assert!(
            push_mobile_endpoint_send_run(
                &mut run,
                &identities,
                None,
                Some(participant_key),
                endpoint_node_addr,
                vec![1],
            )
            .is_none()
        );
        assert!(
            push_mobile_endpoint_send_run(
                &mut run,
                &identities,
                None,
                Some(participant_key),
                endpoint_node_addr,
                vec![2],
            )
            .is_none()
        );

        let Some(MobileEndpointSendRun::Identity {
            participant_fallback: run_participant_fallback,
            participant_key: run_participant_key,
            identity: run_identity,
            payloads,
        }) = run.as_ref()
        else {
            panic!("resolved peer should own an identity send run");
        };
        assert!(run_participant_fallback.is_none());
        assert_eq!(*run_participant_key, Some(participant_key));
        assert_eq!(*run_identity, identity);
        assert_eq!(payloads, &vec![vec![1], vec![2]]);

        let previous = push_mobile_endpoint_send_run(
            &mut run,
            &identities,
            Some("other".to_string()),
            None,
            [9; 16],
            vec![3],
        )
        .expect("peer change should flush previous run");
        let MobileEndpointSendRun::Identity { payloads, .. } = previous;
        assert_eq!(payloads, vec![vec![1], vec![2]]);
        assert!(run.is_none());
    }

    #[test]
    fn mobile_peer_ping_due_recovers_from_future_timestamps() {
        assert!(!mobile_peer_ping_due(Some(122), Some(115), 120));
        assert!(!mobile_peer_ping_due(Some(180), Some(1), 120));
        assert!(!mobile_peer_ping_due(None, Some(122), 120));
        assert!(mobile_peer_ping_due(None, Some(180), 120));
    }

    #[test]
    fn mobile_connected_roster_peers_rejects_far_future_presence() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        let peer = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let mesh = Arc::new(RwLock::new(FipsMeshRuntime::with_local_routes(
            config.peers.clone(),
            vec![],
        )));
        let now = unix_timestamp();
        let presence = Arc::new(RwLock::new(HashMap::from([(
            peer.to_string(),
            MobilePeerPresence {
                last_seen_at: Some(now + 60),
                ..MobilePeerPresence::default()
            },
        )])));

        let connected = mobile_connected_roster_peers(&mesh, &presence).expect("connected peers");

        assert!(connected.is_empty());
    }

    #[test]
    fn mobile_runtime_state_keeps_retry_only_probe_separate_from_link() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        let peer = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let mesh = FipsMeshRuntime::with_local_routes(config.peers.clone(), vec![]);
        let now = 1_778_998_000;
        let mut presence = HashMap::new();
        presence.insert(
            peer.to_string(),
            MobilePeerPresence {
                last_seen_at: Some(now - 4),
                ..MobilePeerPresence::default()
            },
        );
        let endpoint_node_addr = *PeerIdentity::from_npub(&config.peers[0].endpoint_npub)
            .expect("endpoint identity")
            .node_addr();
        let endpoint_peer = FipsEndpointPeer {
            npub: config.peers[0].endpoint_npub.clone(),
            node_addr: endpoint_node_addr,
            connected: false,
            transport_addr: None,
            transport_type: None,
            link_id: 0,
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
            direct_probe_pending: true,
            direct_probe_after_ms: Some(98_765),
            direct_probe_retry_count: 2,
            direct_probe_auto_reconnect: false,
            direct_probe_expires_at_ms: Some(123_456),
            nostr_traversal_consecutive_failures: 2,
            nostr_traversal_in_cooldown: true,
            nostr_traversal_cooldown_until_ms: Some(99_000),
            nostr_traversal_last_observed_skew_ms: Some(200),
        };

        let state = mobile_runtime_state(
            &config,
            &mesh,
            &presence,
            vec![endpoint_peer],
            Vec::new(),
            now,
        );

        assert_eq!(state.connected_peer_count, 1);
        assert!(state.peers[0].reachable);
        assert!(state.peers[0].direct_probe_pending);
        assert_eq!(state.peers[0].direct_probe_after_ms, Some(98_765));
        assert_eq!(state.peers[0].direct_probe_retry_count, 2);
        assert!(!state.peers[0].direct_probe_auto_reconnect);
        assert_eq!(state.peers[0].direct_probe_expires_at_ms, Some(123_456));
        assert_eq!(state.peers[0].fips_nostr_traversal_failures, 2);
        assert!(state.peers[0].fips_nostr_traversal_in_cooldown);
        assert_eq!(
            state.peers[0].fips_nostr_traversal_cooldown_until_ms,
            Some(99_000)
        );
        assert_eq!(
            state.peers[0].fips_nostr_traversal_last_observed_skew_ms,
            Some(200)
        );
        assert_eq!(state.peers[0].fips_transport_addr, "");
        assert_eq!(state.peers[0].last_fips_seen_at, Some(now - 4));
    }

    #[test]
    fn mobile_endpoint_hints_include_current_lan_candidates() {
        let mobile = MobileTunnelConfig {
            advertised_endpoint: "192.168.50.22:51820".to_string(),
            listen_port: 51820,
            local_address: "10.44.1.2/32".to_string(),
            share_local_candidates: true,
            ..empty_config()
        };

        let hints = mobile_endpoint_hints_with_candidates(
            &mobile,
            vec![
                Ipv4Addr::new(192, 168, 50, 33),
                Ipv4Addr::new(10, 44, 1, 2),
                Ipv4Addr::new(100, 100, 50, 1),
            ],
        );
        let addrs = hints.into_iter().map(|hint| hint.addr).collect::<Vec<_>>();

        assert_eq!(
            addrs,
            vec![
                "192.168.50.22:51820".to_string(),
                "192.168.50.33:51820".to_string(),
            ]
        );
    }

    #[test]
    fn mobile_config_wireguard_exit_keeps_mesh_peer_routes_narrow() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        let peer = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.wireguard_exit = WireGuardExitConfig {
            enabled: true,
            address: "10.99.99.2/32".to_string(),
            private_key: "client-private-key".to_string(),
            peer_public_key: "server-public-key".to_string(),
            endpoint: "198.51.100.20:51820".to_string(),
            allowed_ips: vec!["0.0.0.0/0".to_string()],
            ..WireGuardExitConfig::default()
        };

        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");

        assert_eq!(config.peers.len(), 1);
        assert!(
            config
                .route_targets
                .iter()
                .any(|route| route == MESH_TUNNEL_IPV4_CIDR)
        );
        assert!(
            config
                .route_targets
                .iter()
                .any(|route| route == "0.0.0.0/0")
        );

        let peer_routes = config
            .route_targets
            .iter()
            .filter(|route| route.as_str() != "0.0.0.0/0")
            .filter(|route| route.as_str() != MESH_TUNNEL_IPV4_CIDR)
            .collect::<Vec<_>>();
        assert_eq!(peer_routes.len(), 1);
        assert!(peer_routes[0].starts_with("10."));
        assert!(peer_routes[0].ends_with("/32"));
        assert_eq!(config.peers[0].allowed_ips, vec![peer_routes[0].clone()]);

        let wg_config = config.wireguard_exit.as_ref().expect("wg config");
        assert_eq!(wg_config.allowed_ips, vec!["0.0.0.0/0"]);
        assert_eq!(wg_config.persistent_keepalive_secs, 25);
        assert_eq!(config.excluded_routes, vec!["198.51.100.20/32"]);
        assert_eq!(
            config.dns_servers,
            vec![nostr_vpn_core::MESH_MAGIC_DNS_SERVER, "10.64.0.1"]
        );
        assert_eq!(
            config.magic_dns_server,
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER
        );
    }

    #[test]
    fn mobile_config_wireguard_exit_preserves_custom_dns_with_magic_dns() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![own.clone()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.wireguard_exit = WireGuardExitConfig {
            enabled: true,
            address: "10.99.99.2/32".to_string(),
            private_key: "client-private-key".to_string(),
            peer_public_key: "server-public-key".to_string(),
            endpoint: "198.51.100.20:51820".to_string(),
            allowed_ips: vec!["0.0.0.0/0".to_string()],
            dns: vec!["94.140.14.14".to_string()],
            ..WireGuardExitConfig::default()
        };

        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");

        assert_eq!(
            config.dns_servers,
            vec![nostr_vpn_core::MESH_MAGIC_DNS_SERVER, "94.140.14.14"]
        );
        assert_eq!(
            config.magic_dns_server,
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER
        );
    }

    #[test]
    fn mobile_wireguard_exit_dns_forwarders_prefer_configured_tunnel_dns() {
        let platform_dns = vec!["1.1.1.1".to_string()];
        let tunnel_dns = vec![
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER.to_string(),
            "94.140.14.14".to_string(),
        ];

        let forwarders = mobile_magic_dns_forwarders(
            &platform_dns,
            &tunnel_dns,
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER,
        );

        assert_eq!(
            forwarders,
            vec![
                "94.140.14.14:53".parse().unwrap(),
                "1.1.1.1:53".parse().unwrap(),
                "9.9.9.9:53".parse().unwrap(),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mobile_wireguard_start_returns_before_handshake_watchdog() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().expect("test nsec");
        let mut app = AppConfig::generated();
        app.nostr.secret_key.clone_from(&nsec);
        app.ensure_defaults();

        let mobile = MobileTunnelConfig {
            identity_nsec: nsec,
            network_id: "mobile-wg-start".to_string(),
            local_address: "10.44.10.2/32".to_string(),
            listen_port: 0,
            nostr_discovery_enabled: false,
            wireguard_exit: Some(WireGuardExitConfig {
                enabled: true,
                address: "10.99.99.2/32".to_string(),
                private_key: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string(),
                peer_public_key: "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=".to_string(),
                endpoint: format!("127.0.0.1:{}", available_udp_port()),
                allowed_ips: vec!["0.0.0.0/0".to_string()],
                persistent_keepalive_secs: 25,
                ..WireGuardExitConfig::default()
            }),
            ..empty_config()
        };

        let started_at = Instant::now();
        let started = Box::pin(tokio::time::timeout(
            Duration::from_secs(2),
            MobileTunnel::start_async(mobile, app),
        ))
        .await
        .expect("mobile tunnel startup must not wait for WG handshake")
        .expect("mobile tunnel should start with a non-responding WG endpoint");
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "startup should return so Android can protect the WG socket before the watchdog expires"
        );
        assert!(
            started.wg_upstream_socket_fd >= 0,
            "Android needs the WG UDP fd immediately after startup"
        );

        shutdown_started_mobile_tunnel(started).await;
    }

    #[test]
    fn wg_upstream_excluded_route_is_ipv4_only() {
        assert_eq!(
            wg_upstream_excluded_route_for_addr("198.51.100.20:51820".parse().unwrap()),
            Some("198.51.100.20/32".to_string())
        );
        assert_eq!(
            wg_upstream_excluded_route_for_addr("[2001:db8::20]:51820".parse().unwrap()),
            None
        );
    }

    #[test]
    fn mobile_tunnel_launch_config_redacts_persisted_secrets() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-mobile-launch-redaction-{nonce}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.toml");

        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        let secret_key = app.nostr.secret_key.clone();
        let peer = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.wireguard_exit = WireGuardExitConfig {
            enabled: true,
            address: "10.99.99.2/32".to_string(),
            private_key: "client-private-key".to_string(),
            peer_public_key: "server-public-key".to_string(),
            peer_preshared_key: "client-peer-psk".to_string(),
            endpoint: "198.51.100.20:51820".to_string(),
            allowed_ips: vec!["0.0.0.0/0".to_string()],
            ..WireGuardExitConfig::default()
        };
        app.save(&path).expect("save config");

        let json = tunnel_config_json(dir.to_str().expect("utf8 temp dir"));
        assert!(!json.contains(&secret_key));
        assert!(!json.contains("join-secret"));
        assert!(!json.contains("client-private-key"));
        assert!(!json.contains("client-peer-psk"));
        assert!(json.contains("198.51.100.20:51820"));

        let launch_config: MobileTunnelConfig = serde_json::from_str(&json).expect("launch config");
        assert!(launch_config.app_config_toml.is_empty());
        assert!(launch_config.identity_nsec.is_empty());
        assert!(launch_config.invite_secret.is_empty());
        assert!(launch_config.pending_join_invite_secret.is_empty());
        assert_eq!(
            launch_config
                .wireguard_exit
                .as_ref()
                .expect("wireguard exit")
                .private_key,
            ""
        );

        let loaded = mobile_app_config(&launch_config).expect("load app config from path");
        let runtime_config =
            MobileTunnelConfig::from_app_with_config_path(&loaded, &path).expect("runtime config");
        assert_eq!(runtime_config.identity_nsec, secret_key);
        assert_eq!(
            runtime_config
                .wireguard_exit
                .as_ref()
                .expect("runtime wireguard")
                .private_key,
            "client-private-key"
        );

        let provider_json =
            tunnel_provider_options_config_json(dir.to_str().expect("utf8 temp dir"));
        assert!(provider_json.contains(&secret_key));
        assert!(provider_json.contains("join-secret"));
        assert!(provider_json.contains("client-private-key"));
        assert!(provider_json.contains("client-peer-psk"));

        let provider_config: MobileTunnelConfig =
            serde_json::from_str(&provider_json).expect("provider options config");
        assert!(
            provider_config.config_path.is_empty(),
            "packet tunnel extension must not read the containing app's private config path"
        );

        let provider_loaded =
            mobile_app_config(&provider_config).expect("load app config from embedded toml");
        let provider_runtime =
            MobileTunnelConfig::from_app_with_config_path(&provider_loaded, Path::new(""))
                .expect("provider runtime config");
        assert_eq!(provider_runtime.identity_nsec, secret_key);
        assert_eq!(
            provider_runtime
                .wireguard_exit
                .as_ref()
                .expect("provider runtime wireguard")
                .private_key,
            "client-private-key"
        );
        assert!(provider_runtime.config_path.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mobile_config_json_reports_errors_as_json() {
        let json = tunnel_config_json("\0/not-a-path");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(value["error"].as_str().is_some());
    }

    #[test]
    fn mobile_fips_config_uses_discovery_for_roster_peers() {
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer");
        let mobile = MobileTunnelConfig {
            peers: vec![peer],
            advertised_endpoint: "192.168.50.22".to_string(),
            listen_port: 51820,
            nostr_relays: vec!["wss://relay.example".to_string()],
            stun_servers: vec!["stun:stun.example:3478".to_string()],
            share_local_candidates: true,
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        config
            .validate()
            .expect("mobile FIPS config should validate");
        assert_eq!(
            config.node.discovery.backoff_base_secs,
            FIPS_DISCOVERY_BACKOFF_BASE_SECS
        );
        assert_eq!(
            config.node.discovery.backoff_max_secs,
            FIPS_DISCOVERY_BACKOFF_MAX_SECS
        );
        assert_eq!(
            config.node.discovery.forward_min_interval_secs,
            FIPS_DISCOVERY_FORWARD_MIN_INTERVAL_SECS
        );
        assert_eq!(
            config.node.rate_limit.handshake_resend_interval_ms,
            MOBILE_HANDSHAKE_RESEND_INTERVAL_MS
        );
        assert!(
            (config.node.rate_limit.handshake_resend_backoff - MOBILE_HANDSHAKE_RESEND_BACKOFF)
                .abs()
                < f64::EPSILON
        );
        assert!(config.node.discovery.nostr.enabled);
        assert!(config.node.discovery.nostr.advertise);
        assert!(config.node.discovery.nostr.share_local_candidates);
        assert!(config.node.discovery.lan.enabled);
        assert_eq!(
            config.node.discovery.nostr.policy,
            NostrDiscoveryPolicy::ConfiguredOnly
        );
        assert_eq!(
            config.node.discovery.nostr.open_discovery_max_pending,
            MOBILE_NOSTR_OPEN_DISCOVERY_MAX_PENDING
        );
        assert_eq!(
            config.node.discovery.nostr.failure_streak_threshold,
            MOBILE_NOSTR_FAILURE_STREAK_THRESHOLD
        );
        assert_eq!(
            config.node.discovery.nostr.startup_sweep_max_age_secs,
            FIPS_NOSTR_STARTUP_SWEEP_MAX_AGE_SECS
        );
        // The mesh id must NOT appear in the publicly visible relay app tag.
        assert_eq!(config.node.discovery.nostr.app, FIPS_NOSTR_DISCOVERY_APP);
        assert_eq!(
            config.node.discovery.nostr.advert_relays,
            vec!["wss://relay.example".to_string()]
        );
        assert_eq!(
            config.node.discovery.nostr.dm_relays,
            vec!["wss://relay.example".to_string()]
        );
        assert_eq!(
            config.node.discovery.nostr.stun_servers,
            vec!["stun:stun.example:3478".to_string()]
        );
        let TransportInstances::Single(udp) = &config.transports.udp else {
            panic!("expected single udp transport");
        };
        assert_eq!(udp.bind_addr(), "0.0.0.0:51820");
        assert!(!udp.outbound_only());
        assert!(udp.accept_connections());
        assert!(udp.advertise_on_nostr());
        assert!(!udp.is_public());
        assert_eq!(
            mobile_endpoint_hints_with_candidates(&mobile, Vec::new()),
            vec![PeerEndpointHint::udp("192.168.50.22:51820")]
        );
        assert_eq!(config.peers.len(), 1);
        // Mobile peer caps are clamped well below fips's defaults so Open
        // discovery doesn't burn battery on ambient connections.
        assert_eq!(config.node.limits.max_peers, MOBILE_MAX_FIPS_PEERS);
        assert_eq!(
            config.node.limits.max_connections,
            MOBILE_MAX_FIPS_CONNECTIONS
        );
        assert_eq!(config.node.limits.max_links, MOBILE_MAX_FIPS_LINKS);
        assert!(config.peers[0].discovery_fallback_transit);
    }

    #[test]
    fn mobile_fips_config_can_scope_discovery_to_roster_peers() {
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer");
        let mobile = MobileTunnelConfig {
            peers: vec![peer],
            connect_to_non_roster_fips_peers: false,
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        assert_eq!(
            config.node.discovery.nostr.policy,
            NostrDiscoveryPolicy::ConfiguredOnly
        );
    }

    #[test]
    fn mobile_fips_config_marks_default_route_peers_non_transit() {
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string(), "0.0.0.0/0".to_string()],
        )
        .expect("peer");
        let mobile = MobileTunnelConfig {
            peers: vec![peer.clone()],
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);
        let peer_config = config
            .peers
            .iter()
            .find(|candidate| candidate.npub == peer.endpoint_npub)
            .expect("exit peer");

        assert!(
            !peer_config.discovery_fallback_transit,
            "exit/default-route peers should not receive ambient lookup transit"
        );
    }

    #[test]
    fn mobile_fips_config_uses_static_peer_hints() {
        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer");
        let mut peer_hints = HashMap::new();
        peer_hints.insert(
            peer.participant_pubkey.clone(),
            vec![FipsPeerAddressHint {
                addr: "192.168.50.10:51820".to_string(),
                seen_at_ms: None,
                priority: FIPS_STATIC_PEER_ENDPOINT_PRIORITY,
            }],
        );
        let mobile = MobileTunnelConfig {
            peers: vec![peer.clone()],
            peer_hints,
            nostr_relays: vec!["wss://relay.example".to_string()],
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);
        let peer_config = config
            .peers
            .iter()
            .find(|candidate| candidate.npub == peer.endpoint_npub)
            .expect("seeded peer");

        assert_eq!(peer_config.addresses.len(), 1);
        assert_eq!(peer_config.addresses[0].transport, "udp");
        assert_eq!(peer_config.addresses[0].addr, "192.168.50.10:51820");
        assert_eq!(
            peer_config.addresses[0].priority,
            FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY
        );
    }

    #[test]
    fn mobile_fips_config_keeps_hinted_non_roster_peers() {
        let roster_peer = FipsMeshPeerConfig::from_participant_pubkey(
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc",
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("roster peer");
        let transit_peer = AppConfig::generated()
            .own_nostr_pubkey_hex()
            .expect("transit pubkey");
        let transit = FipsMeshPeerConfig::from_participant_pubkey(transit_peer, Vec::new())
            .expect("transit peer");
        let mut peer_hints = HashMap::new();
        peer_hints.insert(
            transit.participant_pubkey.clone(),
            vec![FipsPeerAddressHint {
                addr: "192.168.50.33:51820".to_string(),
                seen_at_ms: Some(1234),
                priority: FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY,
            }],
        );
        let mobile = MobileTunnelConfig {
            peers: vec![roster_peer],
            peer_hints,
            connect_to_non_roster_fips_peers: true,
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);
        let transit_config = config
            .peers
            .iter()
            .find(|candidate| candidate.npub == transit.endpoint_npub)
            .expect("hinted non-roster peer should seed FIPS");

        assert_eq!(transit_config.addresses.len(), 1);
        assert_eq!(transit_config.addresses[0].transport, "udp");
        assert_eq!(transit_config.addresses[0].addr, "192.168.50.33:51820");
        assert_eq!(transit_config.addresses[0].seen_at_ms, Some(1234));
        assert_eq!(
            transit_config.addresses[0].priority,
            FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY
        );
        assert!(
            transit_config.discovery_fallback_transit,
            "hinted non-roster peers should be usable as fallback transit"
        );
        assert!(
            !transit_config.auto_reconnect,
            "hinted non-roster transit peers should not retry forever"
        );
    }

    #[test]
    fn mobile_fips_config_does_not_advertise_without_peers() {
        let config = fips_endpoint_config("nostr-vpn:test", &empty_config());

        config
            .validate()
            .expect("empty mobile FIPS config should validate");
        assert!(!config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
        assert!(!config.node.discovery.lan.enabled);
        let TransportInstances::Single(udp) = &config.transports.udp else {
            panic!("expected single udp transport");
        };
        assert!(!udp.advertise_on_nostr());
        assert!(udp.accept_connections());
        assert!(config.peers.is_empty());
    }

    #[test]
    fn mobile_fips_config_uses_discovery_for_pending_join_request_without_peers() {
        let admin = Keys::generate().public_key().to_hex();
        let mobile = MobileTunnelConfig {
            pending_join_request_recipient: admin,
            pending_join_requested_at: 1,
            ..empty_config()
        };
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        assert!(config.node.discovery.nostr.enabled);
        assert!(config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            NostrDiscoveryPolicy::Open
        );
        assert!(config.peers.is_empty());
    }
