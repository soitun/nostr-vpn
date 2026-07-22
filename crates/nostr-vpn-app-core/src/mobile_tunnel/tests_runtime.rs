    fn available_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("bind test port")
            .local_addr()
            .expect("local addr")
            .port()
    }

    fn available_tcp_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind test TCP port")
            .local_addr()
            .expect("local TCP addr")
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
        let _ = endpoint.shutdown().await;
    }

    fn local_mobile_fips_config(scope: &str, mobile: &MobileTunnelConfig) -> FipsConfig {
        let mut config = fips_endpoint_config(scope, mobile);
        config.node.discovery.nostr.enabled = false;
        config.node.discovery.nostr.advertise = false;
        config.node.discovery.lan.enabled = false;
        config.transports.webrtc = TransportInstances::default();
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Arc<FipsEndpoint>> + 'a>> {
        Box::pin(async move {
            Arc::new(
                Box::pin(
                    FipsEndpoint::builder()
                        .config(local_mobile_fips_config(scope, mobile))
                        .identity_nsec(mobile.identity_nsec.clone())
                        .discovery_scope(scope.to_string())
                        .without_system_tun()
                        .bind(),
                )
                .await
                .expect("bind local mobile FIPS endpoint"),
            )
        })
    }

    fn admin_join_request_app(admin_nsec: &str, admin_pubkey: &str, network_id: &str) -> AppConfig {
        let mut admin_app = AppConfig::generated();
        admin_app.nostr.public_key = admin_pubkey.to_string();
        admin_app.nostr.secret_key = admin_nsec.to_string();
        admin_app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Home".to_string(),
            enabled: true,
            network_id: network_id.to_string(),
            join_secret: "join-secret".to_string(),
            devices: vec![admin_pubkey.to_string()],
            removed_devices: Vec::new(),
            admins: vec![admin_pubkey.to_string()],
            listen_for_join_requests: true,
            join_request_admin: String::new(),
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
            pending_join_secret: "join-secret".to_string(),
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
            join_secret: "join-secret".to_string(),
            devices: vec![client_pubkey.to_string(), exit_pubkey.to_string()],
            removed_devices: Vec::new(),
            admins: vec![client_pubkey.to_string()],
            listen_for_join_requests: true,
            join_request_admin: String::new(),
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
        requester_endpoint: &Arc<FipsEndpoint>,
        admin_endpoint: &Arc<FipsEndpoint>,
        requester_mobile: &MobileTunnelConfig,
    ) -> (
        ReceivedFipsControlFrame,
        FipsControlTcpRuntime,
        FipsControlTcpRuntime,
    ) {
        let (recipient_npub, frame) = pending_mobile_join_request_frame(requester_mobile)
            .expect("pending join request frame")
            .expect("pending join request should exist");
        let recipient_peer =
            PeerIdentity::from_npub(&recipient_npub).expect("recipient endpoint identity");
        let requester_control = FipsControlTcpRuntime::start(Arc::clone(requester_endpoint))
            .await
            .expect("start requester state control");
        let mut admin_control = FipsControlTcpRuntime::start(Arc::clone(admin_endpoint))
            .await
            .expect("start admin state control");
        let (sent, received) = tokio::join!(
            requester_control.send(recipient_peer, &frame),
            tokio::time::timeout(Duration::from_secs(5), admin_control.recv()),
        );
        sent.expect("send join request over FIPS-TCP");
        let received = received
            .expect("admin state-control receive timeout")
            .expect("admin state-control service closed");
        (received, requester_control, admin_control)
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
        let receive_limit = packets.len().saturating_mul(2).max(1);
        let mut received_batch = Vec::with_capacity(receive_limit);
        for _ in 0..50 {
            started
                .outbound_tx
                .send(packets.to_vec())
                .await
                .expect("send packet batch into mobile tunnel");
            for _ in 0..receive_limit {
                match tokio::time::timeout(
                    Duration::from_millis(100),
                    recipient.recv_batch_into(&mut received_batch, receive_limit),
                )
                .await
                {
                    Ok(Some(received)) if received > 0 => {
                        for message in received_batch.drain(..) {
                            if message.source_peer.npub() == started.endpoint.npub()
                                && message.data.as_slice() == packets[messages.len()].as_slice()
                            {
                                messages.push(message);
                                if messages.len() == packets.len() {
                                    return messages;
                                }
                            }
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("recipient should receive mobile packets over FIPS");
    }

    async fn receive_mobile_inbound_packets_until(
        started: &mut MobileTunnelStarted,
        packets: &[Vec<u8>],
    ) {
        let mut remaining = packets.to_vec();
        for _ in 0..50 {
            loop {
                match started.inbound_rx.try_recv() {
                    Ok(batch) => {
                        for bytes in batch {
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
                    }
                    Err(tokio_mpsc::error::TryRecvError::Empty) => break,
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
        state_control: &FipsControlTcpSender,
        received: ReceivedFipsControlFrame,
    ) -> (Arc<RwLock<AppConfig>>, AtomicBool) {
        let admin_app_config = Arc::new(RwLock::new(admin_app));
        let app_config_dirty = AtomicBool::new(false);
        let mesh = new_mobile_mesh(FipsMeshRuntime::with_local_routes(
            Vec::new(),
            vec![admin_mobile.local_address.clone()],
        ));
        let mesh_peers = Arc::new(RwLock::new(Vec::new()));
        let peer_identities = Arc::new(RwLock::new(MobilePeerIdentityMap::default()));
        let peer_hints = Arc::new(RwLock::new(HashMap::new()));
        let presence = Arc::new(RwLock::new(HashMap::new()));
        let config_state = Arc::new(RwLock::new(admin_mobile));
        let join_request_active = AtomicBool::new(false);
        let control = MobileEndpointReceiveContext {
            endpoint: admin_endpoint,
            mesh: &mesh,
            mesh_peers: &mesh_peers,
            peer_identities: &peer_identities,
            peer_hints: &peer_hints,
            presence: &presence,
            config_state: &config_state,
            app_config: &admin_app_config,
            app_config_dirty: &app_config_dirty,
            config_path: Some(config_path),
            join_request_active: &join_request_active,
            state_control,
        };

        handle_mobile_state_control_frame(&control, received)
            .await
            .expect("handle mobile join request frame");
        (admin_app_config, app_config_dirty)
    }

    #[test]
    fn mobile_join_request_sends_and_records_over_real_fips_endpoint() {
        std::thread::Builder::new()
            .name("mobile-join-fips".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("mobile join test runtime")
                    .block_on(mobile_join_request_roundtrip());
            })
            .expect("spawn mobile join test")
            .join()
            .expect("mobile join test thread");
    }

    async fn mobile_join_request_roundtrip() {
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

        let (received, requester_control, admin_control) = send_pending_mobile_join_request(
            &requester_endpoint,
            &admin_endpoint,
            &requester_mobile,
        )
        .await;
        assert_eq!(received.source_peer.npub(), requester_endpoint.npub());
        let admin_control_sender = admin_control.sender();
        let (admin_app_config, app_config_dirty) = handle_admin_mobile_join_request(
            &admin_endpoint,
            admin_app,
            admin_mobile,
            &config_path,
            &admin_control_sender,
            received,
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

        requester_control.stop().await;
        admin_control.stop().await;
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
    fn websocket_seed_router_delivers_join_roster_to_guest_without_preconfigured_admin() {
        std::thread::Builder::new()
            .name("mobile-wss-join-roster".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("WSS join test runtime")
                    .block_on(websocket_seed_router_join_roster_roundtrip());
            })
            .expect("spawn WSS join test")
            .join()
            .expect("WSS join test thread");
    }

    #[allow(clippy::too_many_lines)]
    async fn websocket_seed_router_join_roster_roundtrip() {
        let test_started_at = Instant::now();
        let requested_at = unix_timestamp().saturating_sub(1);
        let approved_at = unix_timestamp();

        let seed_keys = Keys::generate();
        let seed_nsec = seed_keys.secret_key().to_bech32().expect("seed nsec");
        let seed_port = available_tcp_port();
        let seed_url = format!("ws://127.0.0.1:{seed_port}/fips");
        let mut seed_config = FipsConfig::new();
        seed_config.node.routing.mode = fips_endpoint::RoutingMode::ReplyLearned;
        seed_config.node.discovery.nostr.enabled = false;
        seed_config.node.discovery.nostr.advertise = false;
        seed_config.node.discovery.lan.enabled = false;
        seed_config.transports.websocket = TransportInstances::Single(WebSocketConfig {
            bind_addr: Some(format!("127.0.0.1:{seed_port}")),
            ..WebSocketConfig::default()
        });
        let seed = Arc::new(
            Box::pin(
                FipsEndpoint::builder()
                    .config(seed_config)
                    .identity_nsec(seed_nsec)
                    .without_system_tun()
                    .bind(),
            )
                .await
                .expect("bind WebSocket seed"),
        );

        let mut guest_app = AppConfig::generated_without_networks();
        guest_app.node_name = "Joining device".to_string();
        guest_app.fips_nostr_discovery_enabled = false;
        guest_app.fips_webrtc_enabled = false;
        guest_app
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending guest join request");
        let bootstrap = nostr_vpn_core::identity_bridge::nostr_identity_device_approval_bootstrap(
            &guest_app
                .pending_nostr_join_request
                .as_ref()
                .expect("pending request")
                .request,
        )
        .expect("guest join bootstrap");

        let admin_keys = Keys::generate();
        let admin_pubkey = admin_keys.public_key().to_hex();
        let mut admin_app = AppConfig::generated();
        admin_app.nostr.secret_key = admin_keys.secret_key().to_secret_hex();
        admin_app.nostr.public_key = admin_pubkey.clone();
        admin_app.networks[0].name = "Home".to_string();
        admin_app.networks[0].enabled = true;
        admin_app.networks[0].network_id = "wss-join-roster".to_string();
        admin_app.networks[0].devices = vec![admin_pubkey.clone()];
        admin_app.networks[0].admins = vec![admin_pubkey.clone()];
        // A real household/team roster pushes the completed config past the
        // safe NetworkExtension response size, so the UI handoff must use the
        // chunked protocol instead of relying on a tiny two-device fixture.
        for index in 0..16 {
            let participant = Keys::generate().public_key().to_hex();
            admin_app.networks[0].devices.push(participant.clone());
            admin_app.peer_aliases.insert(
                participant,
                format!("offline-regression-participant-{index:02}"),
            );
        }
        admin_app.ensure_defaults();
        let network_entry_id = admin_app.networks[0].id.clone();
        let prepared = crate::join_approval::prepare_join_approval(
            &admin_app,
            &network_entry_id,
            &bootstrap,
            approved_at,
        )
        .expect("prepare ordinary signed join roster");
        admin_app = prepared.updated_config;
        admin_app.fips_websocket_seed_urls = vec![seed_url.clone()];
        admin_app.fips_nostr_discovery_enabled = false;
        admin_app.fips_webrtc_enabled = false;

        let admin_dir = std::env::temp_dir().join(format!(
            "nvpn-mobile-queued-join-{}-{}",
            std::process::id(),
            approved_at
        ));
        std::fs::create_dir_all(&admin_dir).expect("create admin config directory");
        let admin_config_path = admin_dir.join("config.toml");
        admin_app
            .save(&admin_config_path)
            .expect("persist approved admin config");
        nostr_vpn_core::join_delivery::queue_join_roster(
            &admin_config_path,
            &bootstrap.device_app_key_npub,
            &prepared.join_roster,
        )
        .expect("queue approval through the production outbox");

        let mut guest_mobile = MobileTunnelConfig::from_app(&guest_app).expect("guest config");
        guest_mobile.listen_port = available_udp_port();
        let mut admin_mobile =
            MobileTunnelConfig::from_app_with_config_path(&admin_app, &admin_config_path)
                .expect("admin config");
        let queued_join_rosters = nostr_vpn_core::join_delivery::load_join_rosters(
            &admin_config_path,
        )
        .into_iter()
        .map(|(_, queued)| queued)
        .collect();
        // Match iOS provider options: the packet extension receives a complete
        // launch snapshot but cannot read the containing app's config path.
        admin_mobile.detach_from_persisted_config_path();
        admin_mobile.listen_port = available_udp_port();
        assert!(guest_mobile.peers.is_empty(), "guest must not know the admin");

        let guest_udp_addr = format!("127.0.0.1:{}", guest_mobile.listen_port);
        let guest = Box::pin(MobileTunnel::start_async(guest_mobile, guest_app))
            .await
            .expect("start guest on its physical edge");

        let router_keys = Keys::generate();
        let router_port = available_udp_port();
        let mut router_config = FipsConfig::new();
        router_config.node.routing.mode = fips_endpoint::RoutingMode::ReplyLearned;
        router_config.node.discovery.nostr.enabled = false;
        router_config.node.discovery.nostr.advertise = false;
        router_config.node.discovery.lan.enabled = false;
        router_config.transports.websocket = TransportInstances::Single(WebSocketConfig {
            seed_urls: vec![seed_url],
            ..WebSocketConfig::default()
        });
        router_config.transports.udp = TransportInstances::Single(UdpConfig {
            bind_addr: Some(format!("127.0.0.1:{router_port}")),
            outbound_only: Some(false),
            accept_connections: Some(true),
            advertise_on_nostr: Some(false),
            public: Some(false),
            ..UdpConfig::default()
        });
        router_config.peers = vec![FipsPeerConfig::new(
            guest.endpoint.npub().to_string(),
            "udp",
            guest_udp_addr,
        )];
        let router = Arc::new(
            Box::pin(
                FipsEndpoint::builder()
                    .config(router_config)
                    .identity_nsec(router_keys.secret_key().to_bech32().expect("router nsec"))
                    .without_system_tun()
                    .bind(),
            )
            .await
            .expect("bind WSS-to-physical router"),
        );
        let admin = Box::pin(MobileTunnel::start_async_with_launch_state(
            admin_mobile,
            admin_app,
            queued_join_rosters,
            None,
        ))
            .await
            .expect("start admin through WebSocket seed");

        let seed_npub = seed.npub().to_string();
        let router_npub = router.npub().to_string();
        let guest_npub = guest.endpoint.npub().to_string();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let guest_ready = guest
                    .endpoint
                    .peers()
                    .await
                    .is_ok_and(|peers| {
                        peers
                            .iter()
                            .any(|peer| peer.npub == router_npub && peer.connected)
                    });
                let admin_ready = admin
                    .endpoint
                    .peers()
                    .await
                    .is_ok_and(|peers| {
                        peers
                            .iter()
                            .any(|peer| peer.npub == seed_npub && peer.connected)
                    });
                let router_ready = router.peers().await.is_ok_and(|peers| {
                    peers.iter().any(|peer| peer.npub == seed_npub && peer.connected)
                        && peers
                            .iter()
                            .any(|peer| peer.npub == guest_npub && peer.connected)
                });
                let seed_ready = seed.peers().await.is_ok_and(|peers| {
                    peers.iter().filter(|peer| peer.connected).count() == 2
                });
                if guest_ready && admin_ready && router_ready && seed_ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("admin, WSS seed, router, and physical guest should authenticate");
        let authenticated_at = Instant::now();

        let guest_identity = PeerIdentity::from_npub(guest.endpoint.npub())
            .expect("guest endpoint identity");
        let queued_delivery = tokio::time::timeout(Duration::from_secs(25), async {
            loop {
                let applied = guest.app_config.read().is_ok_and(|app| {
                    app.pending_nostr_join_request.is_some()
                        && app
                            .active_network_opt()
                            .is_some_and(|network| network.network_id == "wss-join-roster")
                });
                if applied {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            queued_delivery.is_ok(),
                "production queued approval was not delivered through WSS seed and physical router; admin={:?}; guest={:?}; router={:?}; seed={:?}",
                admin.endpoint.peers().await,
                guest.endpoint.peers().await,
                router.peers().await,
                seed.peers().await,
        );
        let roster_applied_at = Instant::now();

        let capabilities = FipsControlFrame::Capabilities {
            network_id: "wss-join-roster".to_string(),
            capabilities: PeerCapabilities::default(),
        };
        let received_before = guest
            .presence
            .read()
            .ok()
            .and_then(|presence| presence.get(&admin_pubkey).map(|peer| peer.rx_bytes))
            .unwrap_or_default();
        tokio::time::timeout(
            Duration::from_secs(5),
            admin.state_control.send(guest_identity, &capabilities),
        )
        .await
        .expect("post-join capabilities delivery timeout")
        .expect("send post-join capabilities to guest");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let capabilities_processed = guest.presence.read().is_ok_and(|presence| {
                    presence
                        .get(&admin_pubkey)
                        .is_some_and(|peer| peer.rx_bytes > received_before)
                });
                if capabilities_processed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("joined guest must accept its new network id and process peer capabilities");
        let new_network_control_at = Instant::now();

        let joined_config = guest
            .take_app_config_toml()
            .expect("take joined app config for UI handoff");
        assert!(
            joined_config.len() > 3_072,
            "fixture must require a chunked iOS provider response"
        );
        let joined_app: AppConfig =
            toml::from_str(&joined_config).expect("decode joined app config handoff");
        assert_eq!(
            joined_app
                .active_network_opt()
                .expect("UI handoff must leave QR onboarding")
                .network_id,
            "wss-join-roster"
        );
        let retried_config = guest
            .take_app_config_toml()
            .expect("retry interrupted UI handoff");
        assert!(!retried_config.is_empty());
        assert_eq!(
            toml::from_str::<toml::Value>(&retried_config)
                .expect("decode retried UI handoff"),
            toml::from_str::<toml::Value>(&joined_config)
                .expect("decode initial UI handoff"),
            "an interrupted provider-to-app handoff must remain retryable"
        );
        assert!(
            guest
                .acknowledge_app_config_toml(&joined_config)
                .expect("acknowledge persisted UI handoff")
        );
        assert_eq!(
            guest
                .take_app_config_toml()
                .expect("read acknowledged UI handoff"),
            ""
        );
        let ui_handoff_acknowledged_at = Instant::now();
        let approval_to_ack =
            ui_handoff_acknowledged_at.duration_since(authenticated_at);
        eprintln!(
            "mobile QR join latency: setup_auth={:?} signed_roster_delivery_and_durable_apply={:?} new_network_control={:?} ui_handoff_ack={:?} total={:?}",
            authenticated_at.duration_since(test_started_at),
            roster_applied_at.duration_since(authenticated_at),
            new_network_control_at.duration_since(roster_applied_at),
            ui_handoff_acknowledged_at.duration_since(new_network_control_at),
            ui_handoff_acknowledged_at.duration_since(test_started_at),
        );
        assert!(
            approval_to_ack < Duration::from_secs(3),
            "an authenticated QR approval should be durably applied and acknowledged in under 3s; took {approval_to_ack:?}"
        );

        shutdown_started_mobile_tunnel(admin).await;
        shutdown_started_mobile_tunnel(guest).await;
        router.shutdown().await.expect("shutdown router");
        seed.shutdown().await.expect("shutdown WebSocket seed");
        let _ = std::fs::remove_dir_all(admin_dir);
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
            join_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            removed_devices: Vec::new(),
            admins: vec![own],
            listen_for_join_requests: true,
            join_request_admin: String::new(),
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
        let other_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("other endpoint npub");
        let other_node_addr = *PeerIdentity::from_npub(&other_npub)
            .expect("other endpoint identity")
            .node_addr();
        let other_endpoint_peer = FipsEndpointPeer {
            npub: other_npub,
            node_addr: other_node_addr,
            ..endpoint_peer.clone()
        };

        let state = mobile_runtime_state_with_tun_counters(
            &config,
            &mesh,
            &HashMap::new(),
            vec![endpoint_peer, other_endpoint_peer],
            Vec::new(),
            MobileTunCounters::default(),
            1_778_998_000,
        );

        assert_eq!(state.expected_peer_count, 1);
        assert_eq!(state.connected_peer_count, 1);
        assert_eq!(state.fips_direct_roster_peer_count, 1);
        assert_eq!(state.fips_other_peer_count, 1);
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
            join_secret: "join-secret".to_string(),
            devices: vec![peer.to_string()],
            removed_devices: Vec::new(),
            admins: vec![own],
            listen_for_join_requests: true,
            join_request_admin: String::new(),
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

        let state = mobile_runtime_state_with_tun_counters(
            &config,
            &mesh,
            &presence,
            Vec::new(),
            Vec::new(),
            MobileTunCounters::default(),
            now,
        );

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
