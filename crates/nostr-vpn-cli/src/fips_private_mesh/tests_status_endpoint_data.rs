    async fn bind_endpoint_data_test_runtime(
        identity_nsec: impl Into<String>,
        network_id: impl AsRef<str>,
        peers: Vec<FipsMeshPeerConfig>,
    ) -> FipsPrivateMeshRuntime {
        let scope = super::fips_lan_discovery_scope(network_id.as_ref());
        let endpoint_peers = super::fips_endpoint_peers_from_mesh(&peers, Vec::new(), Vec::new());
        let config = super::fips_endpoint_config_with_open_discovery_limit(
            &endpoint_peers,
            None,
            super::private_mesh_mtu_from_app(None),
            fips_endpoint::NostrDiscoveryPolicy::ConfiguredOnly,
            super::FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING,
        );
        FipsPrivateMeshRuntime::bind_with_config_scoped(
            identity_nsec,
            Some(scope),
            peers,
            config,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect("runtime should bind")
    }

    #[tokio::test]
    async fn endpoint_data_runtime_sends_and_receives_raw_packet_batch() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().expect("nsec");
        let participant_pubkey = keys.public_key().to_hex();
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);

        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec![format!("{source}/32"), format!("{destination}/32")],
        )
        .expect("peer config");
        let runtime = bind_endpoint_data_test_runtime(nsec, "test-network", vec![peer]).await;
        let first = ipv4_packet(source, destination);
        let second = ipv4_packet(source, destination);
        let expected_endpoint_data_bytes = (first.len() + second.len()) as u64;

        let sent = send_tunnel_packet_batch_owned_with_capacity(
            &runtime,
            vec![first.clone(), second.clone()],
            2,
        )
        .expect("send packet batch");
        assert_eq!(sent, 2);

        let (mut messages, mut events) = (Vec::with_capacity(4), Vec::with_capacity(4));
        tokio::time::timeout(
            Duration::from_secs(2),
            recv_mesh_event_batch_into(&runtime, &mut messages, &mut events, 4),
        )
        .await
        .expect("packet batch should arrive")
        .expect("receive packet batch")
        .expect("batch should contain admitted packets");
        assert_eq!(events.len(), 2);

        let mut packets = events.into_iter().map(|event| match event {
            FipsPrivateMeshEvent::Packet(packet) => packet,
            event => panic!("expected packet event, got {event:?}"),
        });
        assert_eq!(
            packets.next().map(|packet| packet.as_ref().to_vec()),
            Some(first)
        );
        assert_eq!(
            packets.next().map(|packet| packet.as_ref().to_vec()),
            Some(second)
        );
        assert_peer_data_activity(&runtime, &participant_pubkey, expected_endpoint_data_bytes);
        runtime.shutdown().await.expect("shutdown");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn local_tun_pipeline_packets_are_detected_before_mesh_send() {
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let local_destination = Ipv4Addr::new(10, 44, 10, 2);
        let mesh_destination = Ipv4Addr::new(10, 44, 22, 44);
        let mut local_tunnel_ips = HashSet::new();
        local_tunnel_ips.insert(IpAddr::V4(local_destination));

        let local_packet = TunPipelinePacket::new(ipv4_packet(source, local_destination));
        let mesh_packet = TunPipelinePacket::new(ipv4_packet(source, mesh_destination));

        assert!(super::tun_pipeline_packet_targets_local_tunnel(
            &local_tunnel_ips,
            &local_packet
        ));
        assert!(!super::tun_pipeline_packet_targets_local_tunnel(
            &local_tunnel_ips,
            &mesh_packet
        ));
        local_tunnel_ips.clear();
        assert!(!super::tun_pipeline_packet_targets_local_tunnel(
            &local_tunnel_ips,
            &local_packet
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn endpoint_data_runtime_sends_tun_pipeline_batch_without_repacking() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().expect("nsec");
        let participant_pubkey = keys.public_key().to_hex();
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);

        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec![format!("{source}/32"), format!("{destination}/32")],
        )
        .expect("peer config");
        let runtime = bind_endpoint_data_test_runtime(nsec, "test-network", vec![peer]).await;
        let mut first = ipv4_packet(source, destination);
        let mut second = ipv4_packet(source, destination);
        first[20] = 1;
        second[20] = 2;
        let expected_endpoint_data_bytes = (first.len() + second.len()) as u64;

        let mut batch = Vec::with_capacity(4);
        let batch_capacity = batch.capacity();
        batch.push(TunPipelinePacket::new(first.clone()));
        batch.push(TunPipelinePacket::new(second.clone()));

        let turn_capacity = batch.len();
        let mut send_runs = Vec::new();
        let sent = runtime
            .blocking_send_tun_pipeline_packet_turn(
                batch.drain(..),
                turn_capacity,
                &mut send_runs,
            )
            .expect("send TUN pipeline packet batch");
        assert_eq!(sent, 2);
        assert!(batch.is_empty());
        assert_eq!(batch.capacity(), batch_capacity);

        let (mut messages, mut events) = (Vec::with_capacity(4), Vec::with_capacity(4));
        tokio::time::timeout(
            Duration::from_secs(2),
            recv_mesh_event_batch_into(&runtime, &mut messages, &mut events, 4),
        )
        .await
        .expect("packet batch should arrive")
        .expect("receive packet batch")
        .expect("batch should contain admitted packets");
        assert_eq!(events.len(), 2);

        let packets: Vec<_> = events
            .into_iter()
            .map(|event| match event {
                FipsPrivateMeshEvent::Packet(packet) => packet.as_ref().to_vec(),
                event => panic!("expected packet event, got {event:?}"),
            })
            .collect();
        assert_eq!(packets, vec![first, second]);
        assert_peer_data_activity(&runtime, &participant_pubkey, expected_endpoint_data_bytes);
        runtime.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn endpoint_data_runtime_recv_batch_into_reuses_buffers_and_respects_limit() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_bech32().expect("nsec");
        let participant_pubkey = keys.public_key().to_hex();
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);

        let peer = FipsMeshPeerConfig::from_participant_pubkey(
            &participant_pubkey,
            vec![format!("{source}/32"), format!("{destination}/32")],
        )
        .expect("peer config");
        let runtime = bind_endpoint_data_test_runtime(nsec, "test-network", vec![peer]).await;
        let mut first = ipv4_packet(source, destination);
        let mut second = ipv4_packet(source, destination);
        let mut third = ipv4_packet(source, destination);
        first[20] = 1;
        second[20] = 2;
        third[20] = 3;

        let sent = send_tunnel_packet_batch_owned_with_capacity(
            &runtime,
            vec![first.clone(), second.clone(), third.clone()],
            3,
        )
        .expect("send packet batch");
        assert_eq!(sent, 3);

        let mut messages = Vec::with_capacity(8);
        let mut events = Vec::with_capacity(8);
        let message_capacity = messages.capacity();
        let event_capacity = events.capacity();

        let received = tokio::time::timeout(
            Duration::from_secs(2),
            recv_mesh_event_batch_into(&runtime, &mut messages, &mut events, 2),
        )
        .await
        .expect("packet batch should arrive")
        .expect("receive packet batch")
        .expect("batch should contain admitted packets");
        assert_eq!(received, 2);
        assert!(messages.is_empty());
        assert_eq!(messages.capacity(), message_capacity);
        assert_eq!(events.capacity(), event_capacity);

        let packets: Vec<_> = events
            .drain(..)
            .map(|event| match event {
                FipsPrivateMeshEvent::Packet(packet) => packet.as_ref().to_vec(),
                event => panic!("expected packet event, got {event:?}"),
            })
            .collect();
        assert_eq!(packets, vec![first, second]);

        let received = tokio::time::timeout(
            Duration::from_secs(2),
            recv_mesh_event_batch_into(&runtime, &mut messages, &mut events, 8),
        )
        .await
        .expect("packet batch should arrive")
        .expect("receive packet batch")
        .expect("batch should contain admitted packets");
        assert_eq!(received, 1);
        assert!(messages.is_empty());
        assert_eq!(messages.capacity(), message_capacity);
        assert_eq!(events.capacity(), event_capacity);

        let packets: Vec<_> = events
            .drain(..)
            .map(|event| match event {
                FipsPrivateMeshEvent::Packet(packet) => packet.as_ref().to_vec(),
                event => panic!("expected packet event, got {event:?}"),
            })
            .collect();
        assert_eq!(packets, vec![third]);
        runtime.shutdown().await.expect("shutdown");
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn direct_endpoint_source_run_admission_uses_current_mesh_after_replace() {
        let _local_udp_guard = LOCAL_UDP_ENDPOINT_TEST_LOCK.lock().await;
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let bob_nsec = bob_keys.secret_key().to_bech32().expect("bob nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let alice_npub = alice_keys.public_key().to_bech32().expect("alice npub");
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let alice_port = available_udp_port();
        let bob_port = available_udp_port();
        let old_source = Ipv4Addr::new(10, 44, 10, 1);
        let new_source = Ipv4Addr::new(10, 44, 10, 2);
        let destination = Ipv4Addr::new(10, 44, 22, 44);
        let scope = "nostr-vpn:direct-source-run-replace";

        let alice_runtime = FipsPrivateMeshRuntime::bind_with_config_scoped(
            alice_nsec,
            Some(scope.to_string()),
            vec![FipsMeshPeerConfig {
                participant_pubkey: bob_pubkey.clone(),
                endpoint_npub: bob_npub.clone(),
                allowed_ips: vec![format!("{destination}/32")],
            }],
            direct_udp_endpoint_config(alice_port, &bob_npub, bob_port, true),
            vec![format!("{old_source}/32"), format!("{new_source}/32")],
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect("alice endpoint should bind");
        let bob_runtime = FipsPrivateMeshRuntime::bind_with_config_scoped(
            bob_nsec,
            Some(scope.to_string()),
            vec![FipsMeshPeerConfig {
                participant_pubkey: alice_pubkey.clone(),
                endpoint_npub: alice_npub.clone(),
                allowed_ips: vec![format!("{old_source}/32")],
            }],
            direct_udp_endpoint_config(bob_port, &alice_npub, alice_port, false),
            vec![format!("{destination}/32")],
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect("bob endpoint should bind");

        wait_for_fips_peer(&alice_runtime, &bob_npub).await;
        wait_for_fips_peer(&bob_runtime, &alice_npub).await;

        let mut warmup_packet = ipv4_packet(old_source, destination);
        let mut old_packet = ipv4_packet(old_source, destination);
        let mut new_packet = ipv4_packet(new_source, destination);
        warmup_packet[20] = 0;
        old_packet[20] = 1;
        new_packet[20] = 2;

        send_with_retry(&alice_runtime, &warmup_packet).await;
        let bob_runtime = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let stop = AtomicBool::new(false);
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
            let mut packets = DirectTunWriteBatch::with_capacity(1);
            let mut direct_rx = bob_runtime.direct_endpoint_rx.cursor();

            let emitted = bob_runtime
                .recv_direct_endpoint_tun_batch_blocking(
                    &mut direct_rx,
                    1,
                    &stop,
                    &mut packets,
                    &event_tx,
                )?
                .expect("warmup packet should be admitted");
            assert_eq!(emitted, 1);
            assert_eq!(packets.len(), 1);
            assert_eq!(
                packets.run_slices().next(),
                Some(warmup_packet.as_slice())
            );
            bob_runtime.finalize_direct_endpoint_tun_batch_blocking(&mut packets)?;

            Ok(bob_runtime)
        })
        .await
        .expect("warmup receiver should join")
        .expect("warmup receive should succeed");

        send_with_retry(&alice_runtime, &old_packet).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let new_peer = FipsMeshPeerConfig {
            participant_pubkey: alice_pubkey.clone(),
            endpoint_npub: alice_npub.clone(),
            allowed_ips: vec![format!("{new_source}/32")],
        };
        bob_runtime
            .replace_peers(
                vec![new_peer],
                vec![format!("{destination}/32")],
                Vec::new(),
            )
            .expect("replace runtime mesh");

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let expected_new_packet = new_packet.clone();
        let old_len = old_packet.len() as u64;
        let new_len = new_packet.len() as u64;
        let receiver = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
            let mut packets = DirectTunWriteBatch::with_capacity(4);
            let mut direct_rx = bob_runtime.direct_endpoint_rx.cursor();

            let emitted = bob_runtime
                .recv_direct_endpoint_tun_batch_blocking(
                    &mut direct_rx,
                    8,
                    &thread_stop,
                    &mut packets,
                    &event_tx,
                )?
                .expect("new-config packet should be admitted");
            assert!(emitted >= 1);
            assert_eq!(packets.len(), emitted);
            for packet in packets.run_slices() {
                assert_eq!(packet, expected_new_packet.as_slice());
            }
            bob_runtime.finalize_direct_endpoint_tun_batch_blocking(&mut packets)?;

            Ok((bob_runtime, emitted as u64))
        });

        let mut new_sends = 0u64;
        for _ in 0..50 {
            if receiver.is_finished() {
                break;
            }
            send_with_retry(&alice_runtime, &new_packet).await;
            new_sends += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !receiver.is_finished() {
            stop.store(true, Ordering::Release);
        }
        let (bob_runtime, emitted_new_packets) =
            tokio::time::timeout(Duration::from_secs(5), receiver)
                .await
                .expect("new-config receiver timed out")
                .expect("new-config receiver should join")
                .expect("blocking source-run receive should succeed");
        assert!(new_sends >= emitted_new_packets);

        let warmup_len = old_len;
        let expected_tx_bytes = warmup_len + old_len + new_len * new_sends;
        let expected_rx_bytes = warmup_len + new_len * emitted_new_packets;

        let alice_status = alice_runtime
            .peer_statuses()
            .into_iter()
            .find(|status| status.pubkey == bob_pubkey)
            .expect("Bob status");
        assert_eq!(alice_status.tx_bytes, expected_tx_bytes);
        let bob_status = bob_runtime
            .peer_statuses()
            .into_iter()
            .find(|status| status.pubkey == alice_pubkey)
            .expect("Alice status");
        assert_eq!(bob_status.rx_bytes, expected_rx_bytes);

        alice_runtime.shutdown().await.expect("shutdown alice");
        bob_runtime.shutdown().await.expect("shutdown bob");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn endpoint_data_runtime_direct_tun_batch_respects_limit() {
        let _local_udp_guard = LOCAL_UDP_ENDPOINT_TEST_LOCK.lock().await;
        let alice_keys = Keys::generate();
        let bob_keys = Keys::generate();
        let alice_nsec = alice_keys.secret_key().to_bech32().expect("alice nsec");
        let bob_nsec = bob_keys.secret_key().to_bech32().expect("bob nsec");
        let alice_pubkey = alice_keys.public_key().to_hex();
        let bob_pubkey = bob_keys.public_key().to_hex();
        let alice_npub = alice_keys.public_key().to_bech32().expect("alice npub");
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let alice_port = available_udp_port();
        let bob_port = available_udp_port();
        let source = Ipv4Addr::new(10, 44, 30, 1);
        let destination = Ipv4Addr::new(10, 44, 30, 2);
        let scope = "nostr-vpn:direct-batch-limit";

        let alice_runtime = FipsPrivateMeshRuntime::bind_with_config_scoped(
            alice_nsec,
            Some(scope.to_string()),
            vec![FipsMeshPeerConfig {
                participant_pubkey: bob_pubkey,
                endpoint_npub: bob_npub.clone(),
                allowed_ips: vec![format!("{destination}/32")],
            }],
            direct_udp_endpoint_config(alice_port, &bob_npub, bob_port, true),
            vec![format!("{source}/32")],
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect("alice endpoint should bind");
        let bob_runtime = FipsPrivateMeshRuntime::bind_with_config_scoped(
            bob_nsec,
            Some(scope.to_string()),
            vec![FipsMeshPeerConfig {
                participant_pubkey: alice_pubkey,
                endpoint_npub: alice_npub.clone(),
                allowed_ips: vec![format!("{source}/32")],
            }],
            direct_udp_endpoint_config(bob_port, &alice_npub, alice_port, false),
            vec![format!("{destination}/32")],
            Vec::new(),
            Vec::new(),
        )
        .await
        .expect("bob endpoint should bind");

        wait_for_fips_peer(&alice_runtime, &bob_npub).await;
        wait_for_fips_peer(&bob_runtime, &alice_npub).await;

        let mut warmup = ipv4_packet(source, destination);
        let mut first = ipv4_packet(source, destination);
        let mut second = ipv4_packet(source, destination);
        let mut third = ipv4_packet(source, destination);
        warmup[20] = 0;
        first[20] = 1;
        second[20] = 2;
        third[20] = 3;

        send_with_retry(&alice_runtime, &warmup).await;
        let bob_runtime = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let stop = AtomicBool::new(false);
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
            let mut packets = DirectTunWriteBatch::with_capacity(1);
            let mut direct_rx = bob_runtime.direct_endpoint_rx.cursor();

            let received = bob_runtime
                .recv_direct_endpoint_tun_batch_blocking(
                    &mut direct_rx,
                    1,
                    &stop,
                    &mut packets,
                    &event_tx,
                )?
                .expect("warmup packet should be admitted");
            assert_eq!(received, 1);
            assert_eq!(packets.len(), 1);
            assert_eq!(packets.run_slices().next(), Some(warmup.as_slice()));
            bob_runtime.finalize_direct_endpoint_tun_batch_blocking(&mut packets)?;

            Ok(bob_runtime)
        })
        .await
        .expect("warmup receiver should join")
        .expect("warmup receive should succeed");

        send_with_retry(&alice_runtime, &first).await;
        send_with_retry(&alice_runtime, &second).await;
        send_with_retry(&alice_runtime, &third).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let bob_runtime = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let stop = AtomicBool::new(false);
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
            let mut packets = DirectTunWriteBatch::with_capacity(8);
            let mut direct_rx = bob_runtime.direct_endpoint_rx.cursor();

            let received = bob_runtime
                .recv_direct_endpoint_tun_batch_blocking(
                    &mut direct_rx,
                    2,
                    &stop,
                    &mut packets,
                    &event_tx,
                )?
                .expect("batch should contain admitted packets");
            assert_eq!(received, 2);

            assert_eq!(packets.len(), 2);
            let packet_slices: Vec<_> = packets.run_slices().collect();
            assert_eq!(packet_slices[0], first.as_slice());
            assert_eq!(packet_slices[1], second.as_slice());
            packets.clear();

            let received = bob_runtime
                .recv_direct_endpoint_tun_batch_blocking(
                    &mut direct_rx,
                    8,
                    &stop,
                    &mut packets,
                    &event_tx,
                )?
                .expect("batch should contain admitted packets");
            assert_eq!(received, 1);

            assert_eq!(packets.len(), 1);
            assert_eq!(packets.run_slices().next(), Some(third.as_slice()));
            bob_runtime.finalize_direct_endpoint_tun_batch_blocking(&mut packets)?;

            Ok(bob_runtime)
        })
        .await
        .expect("blocking receiver should join")
        .expect("blocking callback receive should succeed");

        alice_runtime.shutdown().await.expect("shutdown alice");
        bob_runtime.shutdown().await.expect("shutdown bob");
    }
