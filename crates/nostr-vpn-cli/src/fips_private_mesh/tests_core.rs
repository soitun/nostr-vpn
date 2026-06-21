    use super::{
        ControlFragmentBuffer, FIPS_DISCOVERY_BACKOFF_BASE_SECS, FIPS_DISCOVERY_BACKOFF_MAX_SECS,
        FIPS_DISCOVERY_FORWARD_MIN_INTERVAL_SECS, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY,
        FIPS_ENDPOINT_FAST_LINK_DEAD_TIMEOUT_SECS, FIPS_ENDPOINT_HEARTBEAT_INTERVAL_SECS,
        FIPS_ENDPOINT_LINK_DEAD_TIMEOUT_SECS, FIPS_ENDPOINT_PENDING_PACKETS_PER_DEST,
        FIPS_ENDPOINT_REKEY_AFTER_SECS, FIPS_ENDPOINT_SESSION_IDLE_TIMEOUT_SECS,
        FIPS_LAN_DISCOVERY_SCOPE_PREFIX, FIPS_MESH_EVENT_DRAIN_LIMIT, FIPS_NOSTR_DISCOVERY_APP,
        FIPS_NOSTR_EXTENDED_COOLDOWN_SECS, FIPS_NOSTR_FAILURE_STREAK_THRESHOLD,
        FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING, FIPS_NOSTR_STARTUP_SWEEP_MAX_AGE_SECS,
        FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS, FIPS_RECONNECT_BACKOFF_BASE_SECS,
        FIPS_RECONNECT_BACKOFF_MAX_SECS, FIPS_STATIC_NON_ROSTER_TRANSIT_MAX_SEEDS,
        FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY, FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY,
        FipsEndpointSendRun, FipsEndpointTransportConfig, FipsPeerActivity, FipsPeerActivitySnapshot,
        FipsPeerAddressHint, FipsPeerIdentityMap, FipsPeerRxKind, FipsPrivateMeshEvent,
        FipsPrivateMeshRuntime, FipsPrivateTunnelConfig, Ipv4Subnet,
        cap_recent_non_roster_transit_endpoints, control_frame_destination_peer,
        control_frame_source_pubkey, drain_event_batch, endpoint_identity_for_send,
        filter_stamped_tunnel_endpoints, filter_static_tunnel_endpoints,
        filter_static_tunnel_endpoints_with_policy, fips_endpoint_config,
        fips_endpoint_config_with_open_discovery_limit, fips_endpoint_peers_from_mesh,
        fips_lan_discovery_scope, fips_peer_address_from_hint,
        fips_tunnel_requires_endpoint_restart, linux_cap_eff_has_net_admin,
        linux_private_ipv4_route_subnets_from_ip_route, linux_tun_setup_error,
        macos_private_ipv4_route_subnets_from_netstat, mesh_status_from_endpoint_peer,
        other_endpoint_peer_statuses, parse_fips_nostr_discovery_policy,
        parse_linux_tun_tx_queue_len, participant_pubkey_bytes, parse_fips_mesh_recv_burst,
        peer_activity_map, peer_identity_map, strip_cidr, tag_authenticated_transport_addr,
        unix_timestamp,
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::{
        BorrowedTunFd, TunPipelineLane, TunPipelinePacket, TunPipelineQueueTx,
        TunQueueSubmit, TunWriteBatch, parse_fips_tun_to_mesh_queue_cap,
        push_mesh_packet_for_tun, raw_write_packet_to_tun, release_tun_bulk_packet_slots,
        submit_tun_packet_batch_to_mesh_queue,
        submit_tun_packet_batch_to_mesh_queue_with_backpressure,
        tun_pipeline_packet_lane,
    };
    #[cfg(target_os = "linux")]
    use super::LINUX_VIRTIO_NET_HDR_LEN;
    use fips_endpoint::{
        Config, ConnectPolicy, FipsEndpointPayload, FipsEndpointPeer, NodeAddr,
        NostrDiscoveryPolicy, PeerConfig as FipsPeerConfig, PeerIdentity, RoutingMode,
        TransportInstances, UdpConfig,
    };
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::config::ConnectedUdpConfig;
    use nostr_vpn_core::config::{AppConfig, PendingOutboundJoinRequest, derive_mesh_tunnel_ip};
    use nostr_vpn_core::data_plane::MeshPeerStatus;
    use nostr_vpn_core::fips_control::{
        FipsControlFrame, NetworkRoster, PeerEndpointHint, decode_fips_control_frame,
        encode_fips_control_messages,
    };
    use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
    use nostr_vpn_core::join_requests::MeshJoinRequest;
    use std::collections::{HashMap, HashSet};
    use std::net::{IpAddr, Ipv4Addr, UdpSocket};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn blocking_mesh_recv_defaults_on_and_accepts_explicit_disable() {
        for value in [None, Some(""), Some("1"), Some("true"), Some("blocking")] {
            assert!(super::fips_blocking_mesh_recv_enabled_from_env(value));
        }

        for value in [
            Some("0"),
            Some("false"),
            Some("no"),
            Some("off"),
            Some("async"),
        ] {
            assert!(!super::fips_blocking_mesh_recv_enabled_from_env(value));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_to_mesh_queue_cap_env_keeps_safe_bounds() {
        assert_eq!(parse_fips_tun_to_mesh_queue_cap(None, 1024), 1024);
        assert_eq!(parse_fips_tun_to_mesh_queue_cap(Some(""), 1024), 1024);
        assert_eq!(
            parse_fips_tun_to_mesh_queue_cap(Some("not-a-number"), 1024),
            1024
        );
        assert_eq!(parse_fips_tun_to_mesh_queue_cap(Some("0"), 1024), 1);
        assert_eq!(
            parse_fips_tun_to_mesh_queue_cap(Some("999999"), 1024),
            65_536
        );
        assert_eq!(parse_fips_tun_to_mesh_queue_cap(Some("4096"), 1024), 4096);
    }

    #[test]
    fn macos_bounded_bulk_queue_derives_release_defaults() {
        assert_eq!(super::macos_default_udp_send_buf_size(), 256 * 1024);
        assert_eq!(
            super::macos_tun_to_mesh_queue_cap(
                super::macos_default_udp_send_buf_size(),
                super::MESH_MIN_UNDERLAY_UDP_MTU as usize,
                64,
            ),
            256
        );

        #[cfg(target_os = "macos")]
        {
            assert_eq!(super::FIPS_MESH_SEND_BURST, 64);
            assert_eq!(super::DEFAULT_FIPS_TUN_TO_MESH_QUEUE_CAP, 256);
            assert_eq!(super::DEFAULT_FIPS_UDP_SEND_BUF_SIZE, Some(256 * 1024));
        }
    }

    #[test]
    fn linux_tun_tx_queue_len_env_keeps_bounded_default() {
        assert_eq!(parse_linux_tun_tx_queue_len(None, 4096), Some(4096));
        assert_eq!(parse_linux_tun_tx_queue_len(Some(""), 4096), Some(4096));
        assert_eq!(parse_linux_tun_tx_queue_len(Some("500"), 4096), Some(500));
        assert_eq!(parse_linux_tun_tx_queue_len(Some("1"), 4096), Some(64));
        assert_eq!(
            parse_linux_tun_tx_queue_len(Some("999999"), 4096),
            Some(65_536)
        );
        assert_eq!(parse_linux_tun_tx_queue_len(Some("0"), 4096), None);
        assert_eq!(parse_linux_tun_tx_queue_len(Some("off"), 4096), None);
        assert_eq!(parse_linux_tun_tx_queue_len(Some("no"), 4096), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn mesh_recv_burst_env_keeps_endpoint_batch_bounds() {
        assert_eq!(parse_fips_mesh_recv_burst(None, 64), 64);
        assert_eq!(parse_fips_mesh_recv_burst(Some(""), 64), 64);
        assert_eq!(parse_fips_mesh_recv_burst(Some("not-a-number"), 64), 64);
        assert_eq!(parse_fips_mesh_recv_burst(Some("0"), 64), 1);
        assert_eq!(parse_fips_mesh_recv_burst(Some("32"), 64), 32);
        assert_eq!(parse_fips_mesh_recv_burst(Some("999"), 64), 128);
    }

    #[test]
    fn parses_fips_nostr_discovery_policy_override() {
        for (raw, expected) in [
            ("configured-only", NostrDiscoveryPolicy::ConfiguredOnly),
            ("configured_only", NostrDiscoveryPolicy::ConfiguredOnly),
            ("open", NostrDiscoveryPolicy::Open),
            ("disabled", NostrDiscoveryPolicy::Disabled),
        ] {
            assert_eq!(parse_fips_nostr_discovery_policy(raw), Some(expected));
        }
        assert_eq!(parse_fips_nostr_discovery_policy("wat"), None);
    }

    #[test]
    fn authenticated_transport_addr_preserves_tcp_type_and_legacy_udp() {
        assert_eq!(
            tag_authenticated_transport_addr(
                Some("203.0.113.20:51820".to_string()),
                Some("udp".to_string())
            ),
            Some("203.0.113.20:51820".to_string())
        );
        assert_eq!(
            tag_authenticated_transport_addr(
                Some("203.0.113.20:443".to_string()),
                Some("tcp".to_string())
            ),
            Some("tcp:203.0.113.20:443".to_string())
        );
        assert_eq!(
            tag_authenticated_transport_addr(Some("tcp:203.0.113.20:443".to_string()), None),
            Some("tcp:203.0.113.20:443".to_string())
        );
    }
    #[test]
    fn raw_tun_write_keeps_fd_open_and_writes_platform_frame() {
        let mut pipe_fds = [0; 2];
        let pipe_result = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
        assert_eq!(pipe_result, 0, "pipe should open");
        let read_fd = pipe_fds[0];
        let write_fd = pipe_fds[1];

        let packet = [0x45, 0, 0, 20, 1, 2, 3, 4];
        let tun_fd = BorrowedTunFd::new(write_fd, false);
        raw_write_packet_to_tun(&tun_fd, &packet, 2).expect("write packet frame");
        raw_write_packet_to_tun(&tun_fd, &packet, 2).expect("fd should remain writable");

        let expected_frame: Vec<u8> = {
            #[cfg(target_os = "macos")]
            {
                let mut frame = vec![0, 0, 0, 2];
                frame.extend_from_slice(&packet);
                frame
            }
            #[cfg(target_os = "linux")]
            {
                packet.to_vec()
            }
        };
        let mut expected = expected_frame.clone();
        expected.extend_from_slice(&expected_frame);

        let mut read_buf = vec![0_u8; expected.len()];
        let read = unsafe {
            libc::read(
                read_fd,
                read_buf.as_mut_ptr().cast::<libc::c_void>(),
                read_buf.len(),
            )
        };

        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }

        assert_eq!(read as usize, expected.len());
        assert_eq!(read_buf, expected);
    }
    #[test]
    fn blocking_tun_write_keeps_fd_open_and_writes_platform_frame() {
        let mut pipe_fds = [0; 2];
        let pipe_result = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
        assert_eq!(pipe_result, 0, "pipe should open");
        let read_fd = pipe_fds[0];
        let write_fd = pipe_fds[1];

        let stop = std::sync::atomic::AtomicBool::new(false);
        let packet = [0x45, 0, 0, 20, 1, 2, 3, 4];
        let tun_fd = BorrowedTunFd::new(write_fd, false);
        super::write_packet_to_tun_blocking(tun_fd, &packet, &stop);
        super::write_packet_to_tun_blocking(tun_fd, &packet, &stop);

        let expected_frame: Vec<u8> = {
            #[cfg(target_os = "macos")]
            {
                let mut frame = vec![0, 0, 0, 2];
                frame.extend_from_slice(&packet);
                frame
            }
            #[cfg(target_os = "linux")]
            {
                packet.to_vec()
            }
        };
        let mut expected = expected_frame.clone();
        expected.extend_from_slice(&expected_frame);

        let mut read_buf = vec![0_u8; expected.len()];
        let read = unsafe {
            libc::read(
                read_fd,
                read_buf.as_mut_ptr().cast::<libc::c_void>(),
                read_buf.len(),
            )
        };

        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }

        assert_eq!(read as usize, expected.len());
        assert_eq!(read_buf, expected);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_vnet_tun_write_prepends_virtio_header() {
        let mut pipe_fds = [0; 2];
        let pipe_result = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
        assert_eq!(pipe_result, 0, "pipe should open");
        let read_fd = pipe_fds[0];
        let write_fd = pipe_fds[1];

        let packet = [0x45, 0, 0, 20, 1, 2, 3, 4];
        let tun_fd = BorrowedTunFd::new(write_fd, true);
        raw_write_packet_to_tun(&tun_fd, &packet, 0).expect("write vnet packet frame");

        let mut expected = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN];
        expected.extend_from_slice(&packet);
        let mut read_buf = vec![0_u8; expected.len()];
        let read = unsafe {
            libc::read(
                read_fd,
                read_buf.as_mut_ptr().cast::<libc::c_void>(),
                read_buf.len(),
            )
        };

        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }

        assert_eq!(read as usize, expected.len());
        assert_eq!(read_buf, expected);
    }

    #[test]
    fn fips_peer_address_hint_splits_transport_tags_for_live_updates() {
        let tcp = fips_peer_address_from_hint(&FipsPeerAddressHint {
            addr: "tcp:203.0.113.20:443".to_string(),
            seen_at_ms: Some(123_000),
            priority: FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY,
        });
        assert_eq!(tcp.transport, "tcp");
        assert_eq!(tcp.addr, "203.0.113.20:443");
        assert_eq!(tcp.seen_at_ms, Some(123_000));
        assert_eq!(tcp.priority, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY);

        let udp = fips_peer_address_from_hint(&FipsPeerAddressHint {
            addr: "udp:203.0.113.21:2121".to_string(),
            seen_at_ms: None,
            priority: FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY,
        });
        assert_eq!(udp.transport, "udp");
        assert_eq!(udp.addr, "203.0.113.21:2121");
        assert_eq!(udp.priority, FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY);
    }

    #[test]
    fn fips_private_tunnel_config_uses_non_roster_peer_setting_for_discovery_policy() {
        if std::env::var("NVPN_FIPS_NOSTR_DISCOVERY_POLICY").is_ok() {
            return;
        }

        let mut app = AppConfig::generated();
        app.fips_host_tunnel_enabled = false;
        app.connect_to_non_roster_fips_peers = false;
        let network_id = app.effective_network_id();
        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            &network_id,
            "utun100",
            app.own_nostr_pubkey_hex().ok().as_deref(),
            None,
            &[],
        )
        .expect("configured-only tunnel config");
        assert_eq!(
            config.nostr_discovery_policy,
            NostrDiscoveryPolicy::ConfiguredOnly
        );

        app.connect_to_non_roster_fips_peers = true;
        let config = FipsPrivateTunnelConfig::from_app(
            &app,
            &network_id,
            "utun100",
            app.own_nostr_pubkey_hex().ok().as_deref(),
            None,
            &[],
        )
        .expect("open tunnel config");
        assert_eq!(config.nostr_discovery_policy, NostrDiscoveryPolicy::Open);
    }

    #[test]
    fn fips_restart_predicate_includes_nostr_discovery_enabled() {
        let app = AppConfig::generated();
        let network_id = app.effective_network_id();
        let current = FipsPrivateTunnelConfig::from_app(
            &app,
            &network_id,
            "utun100",
            app.own_nostr_pubkey_hex().ok().as_deref(),
            None,
            &[],
        )
        .expect("fips tunnel config");
        let mut next = current.clone();

        next.nostr_discovery_enabled = !current.nostr_discovery_enabled;

        assert!(
            fips_tunnel_requires_endpoint_restart(&current, &next),
            "toggling Nostr discovery must tear down old relay subscriptions"
        );
    }

    #[test]
    fn linux_cap_eff_parsing_detects_net_admin() {
        assert_eq!(
            linux_cap_eff_has_net_admin("CapEff:\t0000000000000000\n"),
            Some(false)
        );
        assert_eq!(
            linux_cap_eff_has_net_admin("CapEff:\t0000000000001000\n"),
            Some(true)
        );
        assert_eq!(linux_cap_eff_has_net_admin("Name:\tnvpn\n"), None);
    }

    #[test]
    fn linux_tun_setup_error_points_to_root_service_or_docker_flags() {
        let message = linux_tun_setup_error("utun100", "current process lacks CAP_NET_ADMIN");

        assert!(message.contains("CAP_NET_ADMIN"));
        assert!(message.contains("/dev/net/tun"));
        assert!(message.contains("utun100"));
        assert!(message.contains("sudo nvpn start --connect"));
        assert!(message.contains("system service"));
        assert!(message.contains("--cap-add NET_ADMIN --device /dev/net/tun"));
    }

    fn ipv4_packet(source: Ipv4Addr, destination: Ipv4Addr) -> Vec<u8> {
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let total_len = 20 + payload.len();
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&source.octets());
        packet[16..20].copy_from_slice(&destination.octets());
        packet[20..].copy_from_slice(&payload);
        packet
    }

    fn assert_peer_data_activity(
        runtime: &FipsPrivateMeshRuntime,
        participant_pubkey: &str,
        expected_endpoint_data_bytes: u64,
    ) {
        let status = runtime
            .peer_statuses()
            .into_iter()
            .find(|status| status.pubkey == participant_pubkey)
            .expect("peer status");

        assert_eq!(status.last_seen_at, status.last_data_seen_at);
        assert!(
            status.last_data_seen_at.is_some(),
            "admitted endpoint data should stamp data freshness"
        );
        assert_eq!(status.last_control_seen_at, None);
        assert_eq!(status.tx_bytes, expected_endpoint_data_bytes);
        assert_eq!(status.rx_bytes, expected_endpoint_data_bytes);
    }

    #[test]
    fn drain_event_batch_respects_limit() {
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<FipsPrivateMeshEvent>(FIPS_MESH_EVENT_DRAIN_LIMIT + 8);
        for index in 0..(FIPS_MESH_EVENT_DRAIN_LIMIT + 5) {
            tx.try_send(FipsPrivateMeshEvent::Presence {
                participant_pubkey: format!("peer-{index}"),
                last_seen_at: index as u64,
            })
            .expect("queue test event");
        }

        let drained = drain_event_batch(&mut rx, FIPS_MESH_EVENT_DRAIN_LIMIT);

        assert_eq!(drained.len(), FIPS_MESH_EVENT_DRAIN_LIMIT);
        assert_eq!(rx.len(), 5);
    }

    #[test]
    fn peer_activity_map_preserves_existing_configured_peer_activity() {
        use std::sync::Arc;

        let alice = Keys::generate().public_key().to_hex();
        let bob = Keys::generate().public_key().to_hex();
        let removed = Keys::generate().public_key().to_hex();
        let alice_key = participant_pubkey_bytes(&alice).expect("alice key");
        let bob_key = participant_pubkey_bytes(&bob).expect("bob key");
        let removed_key = participant_pubkey_bytes(&removed).expect("removed key");
        let alice_activity = Arc::new(FipsPeerActivity::default());
        alice_activity.note_tx(42);
        alice_activity.note_rx(7, 123, FipsPeerRxKind::Control);
        alice_activity.note_rx(11, 130, FipsPeerRxKind::Data);
        let mut previous = HashMap::new();
        previous.insert(alice_key, Arc::clone(&alice_activity));
        previous.insert(removed_key, Arc::new(FipsPeerActivity::default()));

        let next = peer_activity_map(&[alice.clone(), bob.clone()], Some(&previous));

        assert!(Arc::ptr_eq(next.get(&alice_key).unwrap(), &alice_activity));
        assert_eq!(
            next.get(&alice_key).unwrap().snapshot(),
            FipsPeerActivitySnapshot {
                last_seen_at: Some(130),
                last_control_seen_at: Some(123),
                last_data_seen_at: Some(130),
                tx_bytes: 42,
                rx_bytes: 18,
            }
        );
        assert_eq!(
            next.get(&bob_key).unwrap().snapshot(),
            FipsPeerActivitySnapshot::default()
        );
        assert!(!next.contains_key(&removed_key));
    }

    #[test]
    fn peer_identity_map_resolves_endpoint_identities_and_skips_invalid_npubs() {
        let participant = Keys::generate().public_key().to_hex();
        let endpoint_keys = Keys::generate();
        let endpoint_hex = endpoint_keys.public_key().to_hex();
        let endpoint_npub = endpoint_keys.public_key().to_bech32().expect("npub");
        let invalid_participant = "invalid-participant".to_string();

        let identities = peer_identity_map(&[
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
        let participant_key = participant_pubkey_bytes(&participant).expect("participant key");
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
            endpoint_identity_for_send(&identities, Some(&participant_key), &endpoint_node_addr)
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

    #[test]
    fn endpoint_send_run_batches_configured_peer_without_participant_string() {
        let participant = Keys::generate().public_key().to_hex();
        let participant_key = participant_pubkey_bytes(&participant).expect("participant key");
        let endpoint_npub = Keys::generate().public_key().to_bech32().expect("npub");
        let identity = PeerIdentity::from_npub(&endpoint_npub).expect("peer identity");
        let endpoint_node_addr = *identity.node_addr().as_bytes();
        let mut identity_map = FipsPeerIdentityMap::default();
        identity_map
            .by_endpoint_node_addr
            .insert(endpoint_node_addr, identity);
        let mut runs = Vec::new();

        FipsPrivateMeshRuntime::push_endpoint_send_run(
            &mut runs,
            &identity_map,
            &participant,
            Some(participant_key),
            &endpoint_node_addr,
            FipsEndpointPayload::new(vec![1]),
        );
        FipsPrivateMeshRuntime::push_endpoint_send_run(
            &mut runs,
            &identity_map,
            &participant,
            Some(participant_key),
            &endpoint_node_addr,
            FipsEndpointPayload::new(vec![2]),
        );

        assert_eq!(runs.len(), 1);
        let FipsEndpointSendRun::Identity(run) = &runs[0];
        assert!(run.participant_fallback.is_none());
        assert_eq!(run.participant_key, Some(participant_key));
        assert_eq!(run.identity, identity);
        let payloads = run
            .payloads
            .iter()
            .map(|payload| payload.as_slice().to_vec())
            .collect::<Vec<_>>();
        assert_eq!(payloads, vec![vec![1], vec![2]]);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_ipv6_tcp_packet(flags: u8, tcp_payload_len: usize) -> Vec<u8> {
        let tcp_len = 20 + tcp_payload_len;
        let mut packet = vec![0u8; 40 + tcp_len];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(tcp_len as u16).to_be_bytes());
        packet[6] = 6;
        packet[40 + 12] = 5 << 4;
        packet[40 + 13] = flags;
        packet
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_ipv6_udp_packet(payload_len: usize) -> Vec<u8> {
        let udp_len = 8 + payload_len;
        let mut packet = vec![0u8; 40 + udp_len];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
        packet[6] = 17;
        packet
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_ipv4_tcp_packet(flags: u8, tcp_payload_len: usize) -> Vec<u8> {
        let total_len = 20 + 20 + tcp_payload_len;
        let mut packet = vec![0u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[9] = 6;
        packet[20 + 12] = 5 << 4;
        packet[20 + 13] = flags;
        packet
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_ipv4_icmp_packet() -> Vec<u8> {
        let mut packet = vec![0u8; 28];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&28u16.to_be_bytes());
        packet[9] = 1;
        packet[20] = 8;
        packet
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_pipeline_packet(bytes: Vec<u8>) -> TunPipelinePacket {
        TunPipelinePacket::new(bytes)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_to_mesh_classifier_reserves_liveness_and_tcp_control_packets() {
        assert_eq!(
            tun_pipeline_packet_lane(&test_ipv4_icmp_packet()),
            TunPipelineLane::Priority
        );

        let mut icmpv6 = vec![0u8; 48];
        icmpv6[0] = 0x60;
        icmpv6[4..6].copy_from_slice(&8u16.to_be_bytes());
        icmpv6[6] = 58;
        assert_eq!(tun_pipeline_packet_lane(&icmpv6), TunPipelineLane::Priority);

        for packet in [
            test_ipv4_tcp_packet(0x10, 0),
            test_ipv4_tcp_packet(0x02, 0),
            test_ipv4_tcp_packet(0x18, 64),
            test_ipv6_tcp_packet(0x10, 0),
            test_ipv6_tcp_packet(0x02, 0),
            test_ipv6_tcp_packet(0x18, 64),
        ] {
            assert_eq!(tun_pipeline_packet_lane(&packet), TunPipelineLane::Priority);
        }

        for packet in [
            test_ipv4_tcp_packet(0x18, 512),
            test_ipv6_tcp_packet(0x18, 512),
            test_ipv6_udp_packet(8),
            vec![0xaa; 32],
        ] {
            assert_eq!(tun_pipeline_packet_lane(&packet), TunPipelineLane::Bulk);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn mesh_receive_write_batch_prioritizes_liveness_packets() {
        let bulk = test_ipv4_tcp_packet(0x18, 512);
        let ping = test_ipv4_icmp_packet();
        let mut batch = TunWriteBatch::with_capacity(2);

        push_mesh_packet_for_tun(bulk, &mut batch);
        push_mesh_packet_for_tun(ping, &mut batch);

        assert_eq!(batch.priority.len(), 1);
        assert_eq!(batch.bulk.len(), 1);
        assert_eq!(batch.priority[0][9], 1);
        assert_eq!(batch.bulk[0][9], 6);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn full_tun_to_mesh_queue_drops_bulk_without_waiting() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(1);

        let first = vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512))];
        let second = vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512))];

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, first),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, second),
            TunQueueSubmit::DroppedBulk
        );

        let queued = rx.bulk.try_recv().expect("first batch should stay queued");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].bytes, test_ipv6_tcp_packet(0x18, 512));
        assert!(
            rx.bulk.try_recv().is_err(),
            "full-queue bulk drop must not smuggle a pending batch into the queue"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_to_mesh_queue_counts_bulk_capacity_by_packets() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(3);

        let first = vec![
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512)),
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 513)),
        ];
        let second = vec![
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 514)),
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 515)),
        ];

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, first),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            tx.bulk_queued_packets
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, second),
            TunQueueSubmit::DroppedBulk
        );
        assert_eq!(
            tx.bulk_queued_packets
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );

        let queued = rx.bulk.try_recv().expect("first batch should stay queued");
        assert_eq!(queued.len(), 2);
        assert!(rx.bulk.try_recv().is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_to_mesh_release_bulk_packet_slots_subtracts_exact_count() {
        let counter = AtomicUsize::new(5);

        release_tun_bulk_packet_slots(&counter, 0);
        assert_eq!(counter.load(Ordering::Relaxed), 5);

        release_tun_bulk_packet_slots(&counter, 3);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_queue_releases_bulk_packet_slots_on_recv() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![
                    test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512)),
                    test_pipeline_packet(test_ipv6_tcp_packet(0x18, 513)),
                ],
            ),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 514)),],
            ),
            TunQueueSubmit::DroppedBulk
        );

        let queued = rx.recv().await.expect("queued bulk batch");
        assert_eq!(queued.len(), 2);
        assert_eq!(
            tx.bulk_queued_packets
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 515)),],
            ),
            TunQueueSubmit::Enqueued
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_rx_exposes_bulk_backlog_for_adaptive_sender_yield() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);

        assert_eq!(rx.bulk_backlog_capacity(), 2);
        assert!(!rx.has_bulk_backlog());
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512))],
            ),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(rx.bulk_backlog_packets(), 1);
        assert!(rx.has_bulk_backlog());

        let queued = rx.recv().await.expect("queued bulk batch");
        assert_eq!(queued.len(), 1);
        assert_eq!(rx.bulk_backlog_packets(), 0);
        assert!(!rx.has_bulk_backlog());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_read_backpressure_waits_for_bulk_headroom() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![
                    test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512)),
                    test_pipeline_packet(test_ipv6_tcp_packet(0x18, 513)),
                ],
            ),
            TunQueueSubmit::Enqueued
        );
        assert!(!tx.tun_read_backpressure_ready(2));

        let wait = tx.wait_for_tun_read_bulk_headroom(2);
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut wait)
                .await
                .is_err(),
            "reader should wait while the bulk packet cap is full"
        );

        let queued = rx.recv().await.expect("queued bulk batch");
        assert_eq!(queued.len(), 2);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), &mut wait)
                .await
                .expect("reader should wake after bulk slots are released")
        );
        assert!(tx.tun_read_backpressure_ready(2));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_backpressured_submission_splits_oversized_bulk_batch() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);
        let first = test_ipv6_tcp_packet(0x18, 512);
        let second = test_ipv6_tcp_packet(0x18, 513);
        let third = test_ipv6_tcp_packet(0x18, 514);

        let submit = submit_tun_packet_batch_to_mesh_queue_with_backpressure(
            &tx,
            vec![
                test_pipeline_packet(first.clone()),
                test_pipeline_packet(second.clone()),
                test_pipeline_packet(third.clone()),
            ],
            2,
        );
        tokio::pin!(submit);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut submit)
                .await
                .is_err(),
            "oversized bulk batch should wait after filling the first chunk"
        );

        let first_chunk = rx.recv().await.expect("first bulk chunk");
        assert_eq!(first_chunk.len(), 2);
        assert_eq!(first_chunk[0].bytes, first);
        assert_eq!(first_chunk[1].bytes, second);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut submit)
                .await
                .expect("submission should finish after first chunk is released"),
            TunQueueSubmit::Enqueued
        );

        let second_chunk = rx.recv().await.expect("second bulk chunk");
        assert_eq!(second_chunk.len(), 1);
        assert_eq!(second_chunk[0].bytes, third);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn full_tun_to_mesh_queue_preserves_priority_progress() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(1);
        let bulk_first = test_ipv6_tcp_packet(0x18, 512);
        let bulk_dropped = test_ipv6_tcp_packet(0x18, 512);
        let priority = test_ipv4_icmp_packet();

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(bulk_first.clone())],
            ),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(bulk_dropped)],),
            TunQueueSubmit::DroppedBulk
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(priority.clone())]
            ),
            TunQueueSubmit::Enqueued
        );

        let queued_priority = rx
            .priority
            .try_recv()
            .expect("priority packet should bypass full bulk queue");
        assert_eq!(queued_priority.len(), 1);
        assert_eq!(queued_priority[0].bytes, priority);

        let queued_bulk = rx.bulk.try_recv().expect("first bulk should stay queued");
        assert_eq!(queued_bulk.len(), 1);
        assert_eq!(queued_bulk[0].bytes, bulk_first);
        assert!(rx.bulk.try_recv().is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_to_mesh_queue_splits_mixed_batch_into_priority_and_bulk_lanes() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);
        let bulk = test_ipv6_tcp_packet(0x18, 512);
        let ack = test_ipv4_tcp_packet(0x10, 0);
        let ping = test_ipv4_icmp_packet();

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![
                    test_pipeline_packet(bulk.clone()),
                    test_pipeline_packet(ack.clone()),
                    test_pipeline_packet(ping.clone()),
                ],
            ),
            TunQueueSubmit::Enqueued
        );

        let queued_priority = rx.priority.try_recv().expect("priority batch");
        assert_eq!(queued_priority.len(), 2);
        assert_eq!(queued_priority[0].bytes, ack);
        assert_eq!(queued_priority[1].bytes, ping);

        let queued_bulk = rx.bulk.try_recv().expect("bulk batch");
        assert_eq!(queued_bulk.len(), 1);
        assert_eq!(queued_bulk[0].bytes, bulk);
    }
