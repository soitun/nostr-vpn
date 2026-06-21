    #[test]
    fn mobile_admin_records_inbound_join_request_from_unknown_sender() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-mobile-join-request-{nonce}"));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let config_path = dir.join("config.toml");

        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Home".to_string(),
            enabled: true,
            network_id: "mesh-home".to_string(),
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
        let requester = Keys::generate().public_key().to_hex();
        let app_config = Arc::new(RwLock::new(app));
        let dirty = AtomicBool::new(false);
        let request = MeshJoinRequest {
            network_id: "mesh-home".to_string(),
            invite_secret: "join-secret".to_string(),
            requester_node_name: "iPhone".to_string(),
        };

        assert!(
            record_mobile_join_request(
                &app_config,
                &dirty,
                Some(&config_path),
                &requester,
                1_778_998_000,
                &request,
            )
            .expect("record join request")
        );
        assert!(dirty.load(Ordering::Relaxed));

        let saved = AppConfig::load(&config_path).expect("load persisted config");
        assert_eq!(saved.networks[0].inbound_join_requests.len(), 1);
        assert_eq!(
            saved.networks[0].inbound_join_requests[0].requester,
            requester
        );
        assert_eq!(
            saved.networks[0].inbound_join_requests[0].requester_node_name,
            "iPhone"
        );
        assert_eq!(
            saved.networks[0].inbound_join_requests[0].requested_at,
            1_778_998_000
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn available_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("bind test port")
            .local_addr()
            .expect("local addr")
            .port()
    }

    async fn shutdown_started_mobile_tunnel(started: MobileTunnelStarted) {
        let MobileTunnelStarted {
            endpoint,
            tasks,
            wg_upstream,
            ..
        } = started;
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        if let Some(wg) = wg_upstream {
            wg.shutdown().await;
        }
        if let Ok(endpoint) = Arc::try_unwrap(endpoint) {
            let _ = endpoint.shutdown().await;
        }
    }

    fn local_mobile_fips_config(scope: &str, mobile: &MobileTunnelConfig) -> FipsConfig {
        let mut config = fips_endpoint_config(scope, mobile);
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.lan.enabled = false;
        config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some(format!("127.0.0.1:{}", mobile.listen_port)),
            outbound_only: Some(false),
            accept_connections: Some(true),
            advertise_on_nostr: Some(false),
            public: Some(false),
            ..UdpConfig::default()
        });
        config
    }

    fn bind_local_mobile_endpoint<'a>(
        scope: &'a str,
        mobile: &'a MobileTunnelConfig,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = FipsEndpoint> + 'a>> {
        Box::pin(async move {
            Box::pin(
                FipsEndpoint::builder()
                    .config(local_mobile_fips_config(scope, mobile))
                    .identity_nsec(mobile.identity_nsec.clone())
                    .discovery_scope(scope.to_string())
                    .without_system_tun()
                    .bind(),
            )
            .await
            .expect("bind local mobile FIPS endpoint")
        })
    }

    fn admin_join_request_app(admin_nsec: &str, admin_pubkey: &str, network_id: &str) -> AppConfig {
        let mut admin_app = AppConfig::generated();
        admin_app.nostr.secret_key = admin_nsec.to_string();
        admin_app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Home".to_string(),
            enabled: true,
            network_id: network_id.to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![admin_pubkey.to_string()],
            admins: vec![admin_pubkey.to_string()],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        admin_app.ensure_defaults();
        admin_app
    }

    fn admin_mobile_join_request_config(
        admin_nsec: String,
        network_id: &str,
        listen_port: u16,
    ) -> MobileTunnelConfig {
        MobileTunnelConfig {
            identity_nsec: admin_nsec,
            node_name: "admin".to_string(),
            network_id: network_id.to_string(),
            local_address: "10.44.10.1/32".to_string(),
            listen_port,
            join_requests_enabled: true,
            ..empty_config()
        }
    }

    fn requester_mobile_join_request_config(
        requester_nsec: String,
        admin_pubkey: String,
        admin_port: u16,
        requester_port: u16,
        network_id: &str,
        requested_at: u64,
    ) -> MobileTunnelConfig {
        let admin_peer = FipsMeshPeerConfig::from_participant_pubkey(&admin_pubkey, Vec::new())
            .expect("admin control peer");
        let mut requester_peer_hints = HashMap::new();
        requester_peer_hints.insert(
            admin_pubkey.clone(),
            vec![FipsPeerAddressHint {
                addr: format!("127.0.0.1:{admin_port}"),
                seen_at_ms: None,
                priority: FIPS_STATIC_PEER_ENDPOINT_PRIORITY,
            }],
        );
        MobileTunnelConfig {
            identity_nsec: requester_nsec,
            node_name: "iPhone".to_string(),
            network_id: network_id.to_string(),
            local_address: "10.44.10.2/32".to_string(),
            listen_port: requester_port,
            peers: vec![admin_peer],
            peer_hints: requester_peer_hints,
            pending_join_request_recipient: admin_pubkey,
            pending_join_invite_secret: "join-secret".to_string(),
            pending_join_requested_at: requested_at,
            ..empty_config()
        }
    }

    fn fips_exit_mobile_config(
        exit_nsec: String,
        exit_pubkey: &str,
        network_id: &str,
        listen_port: u16,
    ) -> MobileTunnelConfig {
        MobileTunnelConfig {
            identity_nsec: exit_nsec,
            node_name: "fips-exit".to_string(),
            network_id: network_id.to_string(),
            local_address: derive_mesh_tunnel_ip(network_id, exit_pubkey).expect("exit tunnel ip"),
            listen_port,
            nostr_discovery_enabled: false,
            ..empty_config()
        }
    }

    fn fips_exit_client_app(
        client_nsec: &str,
        client_pubkey: &str,
        exit_pubkey: &str,
        network_id: &str,
    ) -> AppConfig {
        let mut app = AppConfig::generated();
        app.nostr.secret_key = client_nsec.to_string();
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: network_id.to_string(),
            invite_secret: "join-secret".to_string(),
            devices: vec![client_pubkey.to_string(), exit_pubkey.to_string()],
            admins: vec![client_pubkey.to_string()],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.exit_node = exit_pubkey.to_string();
        app.ensure_defaults();
        app
    }

    fn assert_mobile_fips_exit_config(
        client_mobile: &MobileTunnelConfig,
        exit_pubkey: &str,
    ) -> Ipv4Addr {
        assert!(
            client_mobile
                .route_targets
                .iter()
                .any(|route| route == "0.0.0.0/0"),
            "selected FIPS exit node must install a mobile default route"
        );
        assert_eq!(client_mobile.wireguard_exit, None);
        let exit_peer = client_mobile
            .peers
            .iter()
            .find(|peer| peer.participant_pubkey == exit_pubkey)
            .expect("selected exit peer");
        assert!(
            client_mobile
                .peer_hints
                .contains_key(&exit_peer.participant_pubkey),
            "selected FIPS exit member should have a static local endpoint hint in this test"
        );
        assert!(
            fips_peer_configs_from_mesh(
                &client_mobile.peers,
                &client_mobile.peer_hints,
                &client_mobile.bootstrap_peers,
                client_mobile.connect_to_non_roster_fips_peers,
            )
            .iter()
            .any(|peer| peer.npub == exit_peer.endpoint_npub && !peer.addresses.is_empty()),
            "selected FIPS exit member should bind with a static address"
        );
        assert!(
            exit_peer
                .allowed_ips
                .iter()
                .any(|route| route == "0.0.0.0/0"),
            "default traffic should route to the selected FIPS member"
        );
        parse_ipv4(&client_mobile.local_address).expect("client tunnel ip")
    }

    async fn send_pending_mobile_join_request(
        requester_endpoint: &FipsEndpoint,
        admin_endpoint: &FipsEndpoint,
        requester_mobile: &MobileTunnelConfig,
    ) -> FipsEndpointMessage {
        let (recipient_npub, frame) = pending_mobile_join_request_frame(requester_mobile)
            .expect("pending join request frame")
            .expect("pending join request should exist");
        let encoded = encode_fips_control_frame(&frame).expect("encode join request");

        for _ in 0..50 {
            requester_endpoint
                .send(recipient_npub.clone(), encoded.clone())
                .await
                .expect("send join request over FIPS");
            if let Ok(Some(message)) =
                tokio::time::timeout(Duration::from_millis(100), admin_endpoint.recv()).await
            {
                return message;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("admin should receive mobile join request over FIPS");
    }

    async fn send_mobile_packets_until_received(
        started: &MobileTunnelStarted,
        recipient: &FipsEndpoint,
        packets: &[Vec<u8>],
    ) -> Vec<FipsEndpointMessage> {
        if packets.is_empty() {
            return Vec::new();
        }
        let mut messages = Vec::with_capacity(packets.len());
        for _ in 0..50 {
            for packet in packets {
                started
                    .outbound_tx
                    .send(packet.clone())
                    .await
                    .expect("send packet into mobile tunnel");
            }
            for _ in 0..packets.len().saturating_mul(2).max(1) {
                match tokio::time::timeout(Duration::from_millis(100), recipient.recv()).await {
                    Ok(Some(message)) => {
                        if message.source_npub() == started.endpoint.npub()
                            && message.data == packets[messages.len()]
                        {
                            messages.push(message);
                            if messages.len() == packets.len() {
                                return messages;
                            }
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("recipient should receive mobile packets over FIPS");
    }

    async fn receive_mobile_inbound_packets_until(
        started: &MobileTunnelStarted,
        packets: &[Vec<u8>],
    ) {
        let mut remaining = packets.to_vec();
        for _ in 0..50 {
            loop {
                match started.inbound_rx.try_recv() {
                    Ok(bytes) => {
                        if let Some(index) = remaining
                            .iter()
                            .position(|packet| packet.as_slice() == bytes)
                        {
                            remaining.swap_remove(index);
                            if remaining.is_empty() {
                                return;
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(error) => panic!("mobile inbound channel closed: {error}"),
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "mobile tunnel should receive all inbound FIPS packets; missing {} packet(s)",
            remaining.len()
        );
    }

    async fn handle_admin_mobile_join_request(
        admin_endpoint: &FipsEndpoint,
        admin_app: AppConfig,
        admin_mobile: MobileTunnelConfig,
        config_path: &Path,
        network_id: &str,
        message: &FipsEndpointMessage,
    ) -> (Arc<RwLock<AppConfig>>, AtomicBool) {
        let admin_app_config = Arc::new(RwLock::new(admin_app));
        let app_config_dirty = AtomicBool::new(false);
        let mesh = Arc::new(RwLock::new(FipsMeshRuntime::with_local_routes(
            Vec::new(),
            vec![admin_mobile.local_address.clone()],
        )));
        let mesh_peers = Arc::new(RwLock::new(Vec::new()));
        let peer_identities = Arc::new(RwLock::new(MobilePeerIdentityMap::default()));
        let peer_hints = Arc::new(RwLock::new(HashMap::new()));
        let presence = Arc::new(RwLock::new(HashMap::new()));
        let config_state = Arc::new(RwLock::new(admin_mobile));
        let join_request_active = AtomicBool::new(false);
        let mut control_fragments = FipsControlFragmentBuffer::default();

        let handled = handle_mobile_control_frame(
            admin_endpoint,
            &mesh,
            &mesh_peers,
            &peer_identities,
            &peer_hints,
            &presence,
            &config_state,
            &admin_app_config,
            &app_config_dirty,
            Some(config_path),
            network_id,
            &join_request_active,
            &mut control_fragments,
            message,
        )
        .await
        .expect("handle mobile join request frame");

        assert!(handled);
        (admin_app_config, app_config_dirty)
    }

    #[tokio::test]
    async fn mobile_join_request_sends_and_records_over_real_fips_endpoint() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-mobile-fips-join-request-{nonce}"));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let config_path = dir.join("config.toml");

        let admin_keys = Keys::generate();
        let requester_keys = Keys::generate();
        let admin_nsec = admin_keys.secret_key().to_bech32().expect("admin nsec");
        let requester_nsec = requester_keys
            .secret_key()
            .to_bech32()
            .expect("requester nsec");
        let admin_pubkey = admin_keys.public_key().to_hex();
        let requester_pubkey = requester_keys.public_key().to_hex();
        let network_id = format!("mobile-fips-join-{nonce}");
        let requested_at = 1_778_998_000;
        let scope = format!("nostr-vpn:{network_id}");

        let admin_app = admin_join_request_app(&admin_nsec, &admin_pubkey, &network_id);
        let admin_mobile =
            admin_mobile_join_request_config(admin_nsec, &network_id, available_udp_port());
        let admin_endpoint = bind_local_mobile_endpoint(&scope, &admin_mobile).await;
        let requester_mobile = requester_mobile_join_request_config(
            requester_nsec,
            admin_pubkey,
            admin_mobile.listen_port,
            available_udp_port(),
            &network_id,
            requested_at,
        );
        let requester_endpoint = bind_local_mobile_endpoint(&scope, &requester_mobile).await;

        let message = send_pending_mobile_join_request(
            &requester_endpoint,
            &admin_endpoint,
            &requester_mobile,
        )
        .await;
        assert_eq!(message.source_npub(), requester_endpoint.npub());
        let (admin_app_config, app_config_dirty) = handle_admin_mobile_join_request(
            &admin_endpoint,
            admin_app,
            admin_mobile,
            &config_path,
            &network_id,
            &message,
        )
        .await;

        assert!(app_config_dirty.load(Ordering::Relaxed));
        {
            let saved = admin_app_config.read().expect("admin app config");
            let inbound = &saved.networks[0].inbound_join_requests;
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].requester, requester_pubkey);
            assert_eq!(inbound[0].requester_node_name, "iPhone");
            assert_eq!(inbound[0].requested_at, requested_at);
        }
        let saved = AppConfig::load(&config_path).expect("load persisted admin config");
        assert_eq!(saved.networks[0].inbound_join_requests.len(), 1);
        assert_eq!(
            saved.networks[0].inbound_join_requests[0].requester,
            requester_pubkey
        );

        requester_endpoint
            .shutdown()
            .await
            .expect("shutdown requester endpoint");
        admin_endpoint
            .shutdown()
            .await
            .expect("shutdown admin endpoint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mobile_runtime_state_marks_authenticated_endpoint_peer_reachable() {
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
        let endpoint_node_addr = *PeerIdentity::from_npub(&config.peers[0].endpoint_npub)
            .expect("endpoint identity")
            .node_addr();
        let endpoint_peer = FipsEndpointPeer {
            npub: config.peers[0].endpoint_npub.clone(),
            node_addr: endpoint_node_addr,
            connected: true,
            transport_addr: Some("192.168.50.10:51820".to_string()),
            transport_type: Some("udp".to_string()),
            link_id: 7,
            srtt_ms: Some(14),
            srtt_age_ms: Some(250),
            packets_sent: 3,
            packets_recv: 4,
            bytes_sent: 120,
            bytes_recv: 240,
            rekey_in_progress: true,
            rekey_draining: false,
            current_k_bit: Some(true),
            last_outbound_route: Some("direct".to_string()),
            direct_probe_pending: true,
            direct_probe_after_ms: Some(98_765),
            direct_probe_retry_count: 4,
            direct_probe_auto_reconnect: true,
            direct_probe_expires_at_ms: Some(123_456),
            nostr_traversal_consecutive_failures: 3,
            nostr_traversal_in_cooldown: true,
            nostr_traversal_cooldown_until_ms: Some(99_000),
            nostr_traversal_last_observed_skew_ms: Some(-75),
        };

        let state = mobile_runtime_state(
            &config,
            &mesh,
            &HashMap::new(),
            vec![endpoint_peer],
            Vec::new(),
            1_778_998_000,
        );

        assert_eq!(state.expected_peer_count, 1);
        assert_eq!(state.connected_peer_count, 1);
        assert!(state.mesh_ready);
        assert_eq!(state.peers[0].participant_pubkey, peer);
        assert!(state.peers[0].reachable);
        assert_eq!(state.peers[0].fips_transport_type, "udp");
        assert_eq!(state.peers[0].fips_srtt_ms, Some(14));
        assert_eq!(state.peers[0].fips_srtt_age_ms, Some(250));
        assert!(state.peers[0].direct_probe_pending);
        assert_eq!(state.peers[0].direct_probe_after_ms, Some(98_765));
        assert_eq!(state.peers[0].direct_probe_retry_count, 4);
        assert!(state.peers[0].direct_probe_auto_reconnect);
        assert_eq!(state.peers[0].direct_probe_expires_at_ms, Some(123_456));
        assert_eq!(state.peers[0].fips_nostr_traversal_failures, 3);
        assert!(state.peers[0].fips_nostr_traversal_in_cooldown);
        assert_eq!(
            state.peers[0].fips_nostr_traversal_cooldown_until_ms,
            Some(99_000)
        );
        assert_eq!(
            state.peers[0].fips_nostr_traversal_last_observed_skew_ms,
            Some(-75)
        );
    }

    #[test]
    fn mobile_runtime_state_marks_recent_control_presence_reachable_without_link() {
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
                last_seen_at: Some(now - 10),
                last_control_seen_at: Some(now - 10),
                last_data_seen_at: Some(now - 20),
                rtt_ms: Some(91),
                tx_bytes: 32,
                rx_bytes: 64,
                ..MobilePeerPresence::default()
            },
        );

        let state = mobile_runtime_state(&config, &mesh, &presence, Vec::new(), Vec::new(), now);

        assert_eq!(state.expected_peer_count, 1);
        assert_eq!(state.connected_peer_count, 1);
        assert!(state.mesh_ready);
        assert!(state.peers[0].reachable);
        assert_eq!(state.peers[0].fips_srtt_ms, Some(91));
        assert_eq!(state.peers[0].tx_bytes, 32);
        assert_eq!(state.peers[0].rx_bytes, 64);
        assert_eq!(state.peers[0].last_fips_seen_at, Some(now - 10));
        assert_eq!(state.peers[0].last_fips_control_seen_at, Some(now - 10));
        assert_eq!(state.peers[0].last_fips_data_seen_at, Some(now - 20));
    }

    #[test]
    fn mobile_runtime_state_rejects_far_future_presence_without_link() {
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
                last_seen_at: Some(now + 60),
                last_control_seen_at: Some(now + 60),
                last_data_seen_at: Some(now + 60),
                rtt_ms: Some(91),
                tx_bytes: 32,
                rx_bytes: 64,
                ..MobilePeerPresence::default()
            },
        );

        let state = mobile_runtime_state(&config, &mesh, &presence, Vec::new(), Vec::new(), now);

        assert_eq!(state.expected_peer_count, 1);
        assert_eq!(state.connected_peer_count, 0);
        assert!(!state.mesh_ready);
        assert!(!state.peers[0].reachable);
        assert_eq!(state.peers[0].last_mesh_seen_at, 0);
        assert_eq!(state.peers[0].last_fips_seen_at, None);
        assert_eq!(state.peers[0].last_fips_control_seen_at, None);
        assert_eq!(state.peers[0].last_fips_data_seen_at, None);
        assert_eq!(state.peers[0].last_handshake_at, None);
        assert_eq!(state.peers[0].error.as_deref(), Some("fips link pending"));
    }
