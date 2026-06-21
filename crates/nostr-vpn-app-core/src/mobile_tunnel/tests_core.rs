    use super::*;
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::config::{NetworkConfig, PendingOutboundJoinRequest};

    fn mobile_app_with_admin(admin_hex: String) -> Arc<RwLock<AppConfig>> {
        let mut app = AppConfig::generated();
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Original".to_string(),
            enabled: true,
            network_id: "mesh".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: Vec::new(),
            admins: vec![admin_hex],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        Arc::new(RwLock::new(app))
    }

    fn dns_query(name: &str, query_type: u16) -> Vec<u8> {
        let mut bytes = vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        for label in name.split('.') {
            bytes.push(u8::try_from(label.len()).expect("label length fits"));
            bytes.extend_from_slice(label.as_bytes());
        }
        bytes.push(0);
        bytes.extend_from_slice(&query_type.to_be_bytes());
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        bytes
    }

    fn ipv4_udp_packet(
        source: Ipv4Addr,
        destination: Ipv4Addr,
        source_port: u16,
        destination_port: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let query = MobileDnsQuery {
            source: destination,
            destination: source,
            source_port: destination_port,
            destination_port: source_port,
            payload,
        };
        build_mobile_dns_response_packet(&query, payload).expect("test packet length fits")
    }

    fn test_ipv4_packet(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
        ipv4_udp_packet(source, destination, 53123, 443, b"mobile-vpn-basic")
    }

    #[test]
    fn mobile_inbound_roster_requires_signed_event() {
        let admin_hex = Keys::generate().public_key().to_hex();
        let app = mobile_app_with_admin(admin_hex);
        let dirty = AtomicBool::new(false);

        let error = apply_mobile_roster(&app, &dirty, None, None)
            .expect_err("unsigned mobile roster frame must be rejected");

        assert!(
            error.to_string().contains("missing signed roster event"),
            "unexpected error: {error:#}"
        );
        assert!(!dirty.load(Ordering::Relaxed));
    }

    #[test]
    fn mobile_inbound_roster_ignores_non_admin_event_author() {
        let known_admin = Keys::generate();
        let outsider = Keys::generate();
        let member_hex = Keys::generate().public_key().to_hex();
        let known_admin_hex = known_admin.public_key().to_hex();
        let outsider_hex = outsider.public_key().to_hex();
        let app = mobile_app_with_admin(known_admin_hex.clone());
        let dirty = AtomicBool::new(false);
        let signed = SignedRoster::sign(
            "mesh",
            NetworkRoster {
                network_name: "Home".to_string(),
                devices: vec![member_hex],
                admins: vec![known_admin_hex, outsider_hex],
                aliases: HashMap::new(),
                signed_at: 1_726_000_000,
            },
            &outsider,
        )
        .expect("sign roster");

        let updated = apply_mobile_roster(&app, &dirty, None, Some(&signed))
            .expect("valid event from non-admin author should be ignored");

        assert!(updated.is_none());
        assert!(!dirty.load(Ordering::Relaxed));
        assert_eq!(
            app.read()
                .expect("app config")
                .networks
                .first()
                .expect("network")
                .shared_roster_updated_at,
            0
        );
    }

    #[test]
    fn mobile_config_stays_split_tunnel_without_exit() {
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

        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.route_targets.len(), 2);
        assert_eq!(config.peers[0].allowed_ips.len(), 1);
        assert!(
            config
                .route_targets
                .iter()
                .any(|route| route == MESH_TUNNEL_IPV4_CIDR)
        );
        let peer_route = config
            .route_targets
            .iter()
            .find(|route| route.as_str() != MESH_TUNNEL_IPV4_CIDR)
            .expect("peer route");
        assert!(peer_route.starts_with("10."));
        assert!(
            !config
                .route_targets
                .iter()
                .any(|route| route == "0.0.0.0/0")
        );
        assert_eq!(
            config.dns_servers,
            vec![nostr_vpn_core::MESH_MAGIC_DNS_SERVER]
        );
        assert_eq!(
            config.magic_dns_server,
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER
        );
    }

    #[test]
    fn mobile_config_selected_exit_node_adds_default_route() {
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
        app.exit_node = peer.to_string();

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
        assert!(
            config.peers[0]
                .allowed_ips
                .iter()
                .any(|route| route == "0.0.0.0/0")
        );
        assert_eq!(config.mtu, nostr_vpn_core::MESH_TUNNEL_MTU);
        assert_eq!(
            config.dns_servers,
            vec![nostr_vpn_core::MESH_MAGIC_DNS_SERVER, "1.1.1.1", "9.9.9.9"]
        );
        assert_eq!(
            config.magic_dns_server,
            nostr_vpn_core::MESH_MAGIC_DNS_SERVER
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mobile_fips_exit_node_routes_default_traffic_to_selected_member() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let client_keys = Keys::generate();
        let exit_keys = Keys::generate();
        let client_nsec = client_keys.secret_key().to_bech32().expect("client nsec");
        let exit_nsec = exit_keys.secret_key().to_bech32().expect("exit nsec");
        let client_pubkey = client_keys.public_key().to_hex();
        let exit_pubkey = exit_keys.public_key().to_hex();
        let network_id = format!("mobile-fips-exit-{nonce}");
        let scope = format!("nostr-vpn:{network_id}");
        let exit_port = available_udp_port();

        let exit_mobile = fips_exit_mobile_config(exit_nsec, &exit_pubkey, &network_id, exit_port);
        let exit_endpoint = bind_local_mobile_endpoint(&scope, &exit_mobile).await;
        let client_app =
            fips_exit_client_app(&client_nsec, &client_pubkey, &exit_pubkey, &network_id);

        let mut client_mobile =
            MobileTunnelConfig::from_app(&client_app).expect("client mobile config");
        client_mobile.listen_port = available_udp_port();
        // fips-core rejects loopback-only static peers unless Nostr discovery is
        // available as fallback. The packet assertion below still exercises the
        // deterministic static hint path.
        client_mobile.nostr_discovery_enabled = true;
        client_mobile.peer_hints.insert(
            exit_pubkey.clone(),
            vec![FipsPeerAddressHint {
                addr: format!("127.0.0.1:{exit_port}"),
                seen_at_ms: None,
                priority: FIPS_STATIC_PEER_ENDPOINT_PRIORITY,
            }],
        );

        let client_tunnel_ip = assert_mobile_fips_exit_config(&client_mobile, &exit_pubkey);
        let packet = test_ipv4_packet(client_tunnel_ip, Ipv4Addr::new(203, 0, 113, 45));
        let packet_two = test_ipv4_packet(client_tunnel_ip, Ipv4Addr::new(203, 0, 113, 46));
        let started = Box::pin(MobileTunnel::start_async(client_mobile, client_app))
            .await
            .expect("start client mobile tunnel");
        let mut messages = send_mobile_packets_until_received(
            &started,
            &exit_endpoint,
            &[packet.clone(), packet_two.clone()],
        )
        .await;
        let message = messages.remove(0);
        let message_two = messages.remove(0);

        let exit_runtime = FipsMeshRuntime::with_local_routes(
            vec![
                FipsMeshPeerConfig::from_participant_pubkey(
                    &client_pubkey,
                    vec![format!("{client_tunnel_ip}/32")],
                )
                .expect("client peer config"),
            ],
            vec!["0.0.0.0/0".to_string()],
        );
        assert!(
            exit_runtime
                .receive_endpoint_data_from_node_addr(
                    message.source_peer.node_addr().as_bytes(),
                    &message.data,
                )
                .is_some(),
            "a FIPS exit node with a local default route should admit the first forwarded packet"
        );
        assert!(
            exit_runtime
                .receive_endpoint_data_from_node_addr(
                    message_two.source_peer.node_addr().as_bytes(),
                    &message_two.data,
                )
                .is_some(),
            "a FIPS exit node with a local default route should admit the second forwarded packet"
        );
        let reply = test_ipv4_packet(Ipv4Addr::new(203, 0, 113, 45), client_tunnel_ip);
        let reply_two = test_ipv4_packet(Ipv4Addr::new(198, 51, 100, 7), client_tunnel_ip);
        exit_endpoint
            .send_to_peer(message.source_peer, reply.clone())
            .await
            .expect("send reply to mobile tunnel");
        exit_endpoint
            .send_to_peer(message.source_peer, reply_two.clone())
            .await
            .expect("send second reply to mobile tunnel");
        let mut expected_reply = reply;
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut expected_reply);
        let mut expected_reply_two = reply_two;
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut expected_reply_two);
        let expected_replies = vec![expected_reply, expected_reply_two];
        receive_mobile_inbound_packets_until(&started, &expected_replies).await;

        shutdown_started_mobile_tunnel(started).await;
        exit_endpoint
            .shutdown()
            .await
            .expect("shutdown exit endpoint");
    }

    #[test]
    fn mobile_magic_dns_answers_peer_name_from_tun_packet() {
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
        app.set_peer_alias(peer, "fixture-peer")
            .expect("peer alias");
        let app = Arc::new(RwLock::new(app));
        let source = Ipv4Addr::new(10, 44, 206, 222);
        let dns_server =
            parse_ipv4(nostr_vpn_core::MESH_MAGIC_DNS_SERVER).expect("magic dns server");
        let query = ipv4_udp_packet(
            source,
            dns_server,
            53000,
            53,
            &dns_query("fixture-peer.nvpn", 1),
        );
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let response = runtime
            .block_on(mobile_magic_dns_response_packet(&query, &app, &[]))
            .expect("dns response packet");

        assert_eq!(&response[12..16], &dns_server.octets());
        assert_eq!(&response[16..20], &source.octets());
        assert_eq!(u16::from_be_bytes([response[20], response[21]]), 53);
        assert_eq!(u16::from_be_bytes([response[22], response[23]]), 53000);
        let expected_ip = derive_mesh_tunnel_ip("test", peer)
            .and_then(|value| strip_cidr(&value).parse::<Ipv4Addr>().ok())
            .expect("peer tunnel ip");
        let expected_octets = expected_ip.octets();
        assert!(
            response.windows(4).any(|window| window == expected_octets),
            "response did not include {expected_ip}: {response:?}"
        );
    }

    #[test]
    fn mobile_config_includes_static_peer_hints_from_app() {
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
        app.fips_peer_endpoints
            .insert(peer.to_string(), vec!["192.168.50.10:51820".to_string()]);
        app.ensure_defaults();

        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let hints = config
            .peer_hints
            .get(peer)
            .expect("static peer hint should be serialized into mobile config");

        assert_eq!(
            hints,
            &vec![FipsPeerAddressHint {
                addr: "192.168.50.10:51820".to_string(),
                seen_at_ms: None,
                priority: FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY,
            }]
        );
    }

    #[test]
    fn mobile_config_keeps_join_request_admin_as_control_peer_without_route() {
        let admin_keys = Keys::generate();
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let admin = admin_keys.public_key().to_hex();
        let admin_npub = admin_keys.public_key().to_bech32().expect("admin npub");
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: Vec::new(),
            admins: vec![admin.clone()],
            listen_for_join_requests: false,
            invite_inviter: admin.clone(),
            outbound_join_request: Some(PendingOutboundJoinRequest {
                recipient: admin.clone(),
                requested_at: 1,
            }),
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        app.fips_peer_endpoints
            .insert(admin.clone(), vec!["192.168.50.10:51820".to_string()]);
        app.ensure_defaults();

        let config = MobileTunnelConfig::from_app(&app).expect("mobile config");

        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].participant_pubkey, admin);
        assert!(config.peers[0].allowed_ips.is_empty());
        assert!(
            !config
                .route_targets
                .iter()
                .any(|route| route.starts_with("10.") && route.ends_with("/32"))
        );
        let hints = config
            .peer_hints
            .get(&admin)
            .expect("admin static hint should stay available for FIPS control");
        assert_eq!(
            hints,
            &vec![FipsPeerAddressHint {
                addr: "192.168.50.10:51820".to_string(),
                seen_at_ms: None,
                priority: FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY,
            }]
        );
        let endpoint_config =
            fips_peer_configs_from_mesh(
                &config.peers,
                &config.peer_hints,
                &config.bootstrap_peers,
                false,
            );
        let endpoint_peer = endpoint_config
            .iter()
            .find(|peer| peer.npub == admin_npub)
            .expect("admin endpoint config");
        assert_eq!(endpoint_peer.addresses.len(), 1);
        assert_eq!(endpoint_peer.addresses[0].addr, "192.168.50.10:51820");
        assert_eq!(
            endpoint_peer.addresses[0].priority,
            FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY
        );
    }

    #[test]
    fn mobile_admin_listener_without_roster_peers_keeps_fips_discovery_enabled() {
        let mut app = AppConfig::generated();
        app.ensure_defaults();
        let own = app.own_nostr_pubkey_hex().expect("own pubkey");
        app.networks = vec![NetworkConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            enabled: true,
            network_id: "test".to_string(),
            invite_secret: "join-secret".to_string(),
            devices: Vec::new(),
            admins: vec![own],
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        }];
        // Isolate the admin-listener behavior from the built-in bootstrap nodes,
        // which would otherwise populate config.peers as fallback transit.
        app.fips_bootstrap_enabled = false;
        app.ensure_defaults();

        let mobile = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        assert!(mobile.join_requests_enabled);
        assert!(mobile.peers.is_empty());
        assert!(config.node.discovery.nostr.enabled);
        assert!(config.node.discovery.nostr.advertise);
        assert_eq!(
            config.node.discovery.nostr.policy,
            NostrDiscoveryPolicy::Open
        );
        assert!(config.peers.is_empty());
    }

    #[test]
    fn mobile_config_seeds_bootstrap_transit_peers() {
        let mut app = AppConfig::generated();
        app.connect_to_non_roster_fips_peers = true;
        app.fips_bootstrap_enabled = true;
        app.ensure_defaults();
        let mobile = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        let bootstrap_count = nostr_vpn_core::config::DEFAULT_FIPS_BOOTSTRAP_PEERS.len();
        assert_eq!(config.peers.len(), bootstrap_count);
        assert!(
            config
                .peers
                .iter()
                .all(|peer| peer.discovery_fallback_transit)
        );
        assert!(
            config.peers.iter().all(|peer| !peer.auto_reconnect),
            "bootstrap/transit peers should not use nvpn roster-style fast reconnect"
        );
        assert!(mobile.nostr_discovery_enabled);
    }

    #[test]
    fn mobile_config_omits_bootstrap_and_relays_when_disabled() {
        let mut app = AppConfig::generated();
        app.fips_bootstrap_enabled = false;
        app.fips_nostr_discovery_enabled = false;
        app.ensure_defaults();
        let mobile = MobileTunnelConfig::from_app(&app).expect("mobile config");
        let config = fips_endpoint_config("nostr-vpn:test", &mobile);

        assert!(config.peers.is_empty());
        assert!(!config.node.discovery.nostr.enabled);
        assert!(!config.node.discovery.nostr.advertise);
    }

    #[test]
    fn pending_mobile_join_request_targets_invite_admin() {
        let admin = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        let expected_recipient = FipsMeshPeerConfig::from_participant_pubkey(admin, Vec::new())
            .expect("recipient")
            .endpoint_npub;
        let mobile = MobileTunnelConfig {
            network_id: "mesh-home".to_string(),
            node_name: "iPhone".to_string(),
            pending_join_request_recipient: admin.to_string(),
            pending_join_requested_at: 1_778_998_000,
            ..empty_config()
        };

        let (recipient, frame) = pending_mobile_join_request_frame(&mobile)
            .expect("join request frame")
            .expect("pending frame");

        assert_eq!(recipient, expected_recipient);
        assert_eq!(
            frame,
            FipsControlFrame::JoinRequest {
                requested_at: 1_778_998_000,
                request: MeshJoinRequest {
                    network_id: "mesh-home".to_string(),
                    invite_secret: String::new(),
                    requester_node_name: "iPhone".to_string(),
                },
            }
        );
    }

    #[test]
    fn mobile_control_source_accepts_unknown_sender_only_for_join_request() {
        let roster_peer =
            "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc".to_string();
        let peer = FipsMeshPeerConfig::from_participant_pubkey(&roster_peer, Vec::new())
            .expect("roster peer");
        let peer_npub = peer.endpoint_npub.clone();
        let mesh = FipsMeshRuntime::with_local_routes(vec![peer], Vec::new());
        let unknown_keys = Keys::generate();
        let unknown_npub = unknown_keys.public_key().to_bech32().expect("unknown npub");
        let unknown_hex = unknown_keys.public_key().to_hex();
        let peer_identity = PeerIdentity::from_npub(&peer_npub).expect("peer identity");
        let unknown_identity = PeerIdentity::from_npub(&unknown_npub).expect("unknown identity");
        let ping = FipsControlFrame::Ping {
            network_id: "mesh-home".to_string(),
            sent_at: 1,
        };
        let join_request = FipsControlFrame::JoinRequest {
            requested_at: 2,
            request: MeshJoinRequest {
                network_id: "mesh-home".to_string(),
                invite_secret: String::new(),
                requester_node_name: "iPhone".to_string(),
            },
        };

        assert_eq!(
            control_frame_source_pubkey(&mesh, peer_identity, &ping),
            Some(roster_peer)
        );
        assert_eq!(
            control_frame_source_pubkey(&mesh, unknown_identity, &ping),
            None
        );
        assert_eq!(
            control_frame_source_pubkey(&mesh, unknown_identity, &join_request),
            Some(unknown_hex)
        );
    }

    #[test]
    fn mobile_peer_identity_map_resolves_endpoint_identities_and_skips_invalid_npubs() {
        let participant = Keys::generate().public_key().to_hex();
        let endpoint_keys = Keys::generate();
        let endpoint_hex = endpoint_keys.public_key().to_hex();
        let endpoint_npub = endpoint_keys.public_key().to_bech32().expect("npub");
        let invalid_participant = "invalid-participant".to_string();

        let identities = mobile_peer_identity_map(&[
            FipsMeshPeerConfig {
                participant_pubkey: participant.clone(),
                endpoint_npub: format!(" {endpoint_hex} "),
                allowed_ips: Vec::new(),
            },
            FipsMeshPeerConfig {
                participant_pubkey: invalid_participant.clone(),
                endpoint_npub: "not-an-npub".to_string(),
                allowed_ips: Vec::new(),
            },
        ]);

        let endpoint_node_addr = *PeerIdentity::from_npub(&endpoint_npub)
            .expect("endpoint identity")
            .node_addr()
            .as_bytes();
        let participant_key = mobile_participant_pubkey_bytes(&participant).expect("participant");
        assert_eq!(identities.by_participant.len(), 1);
        assert!(identities.by_participant.contains_key(&participant_key));
        assert_eq!(identities.by_endpoint_node_addr.len(), 1);
        assert_eq!(
            identities
                .identity_for_participant(&participant)
                .expect("resolved endpoint identity")
                .npub(),
            endpoint_npub
        );
        assert_eq!(
            identities
                .identity_for_send(Some(&participant_key), &endpoint_node_addr)
                .expect("resolved endpoint identity by node addr")
                .npub(),
            endpoint_npub
        );
        assert_eq!(
            identities
                .identity_for_send(None, &endpoint_node_addr)
                .expect("resolved endpoint identity by node addr without participant")
                .npub(),
            endpoint_npub
        );
        assert_eq!(
            mobile_identity_for_send(&identities, Some(&participant_key), &endpoint_node_addr)
                .expect("send identity")
                .npub(),
            endpoint_npub
        );
        assert!(
            identities
                .identity_for_participant(&invalid_participant)
                .is_none()
        );
    }
