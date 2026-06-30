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
    fn tun_pipeline_packet_caches_destination_for_send_route() {
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);
        let packet = TunPipelinePacket::new(ipv4_packet(source, destination));

        assert_eq!(packet.destination, Some(IpAddr::V4(destination)));
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
    async fn tun_to_mesh_rx_exposes_bulk_backlog_for_sender_yield() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);

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
    #[tokio::test]
    async fn tun_to_mesh_discardable_bulk_waits_at_high_water() {
        let capacity = FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP * 2;
        let (tx, mut rx) = TunPipelineQueueTx::channel(capacity);
        let queued = (0..FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP)
            .map(|_| test_pipeline_packet(test_ipv6_udp_packet(8)))
            .collect::<Vec<_>>();

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, queued),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            tx.discardable_bulk_available_packet_slots(),
            0,
            "discardable high-water budget should be full"
        );
        assert_eq!(
            tx.bulk_available_packet_slots(),
            FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP,
            "full reliable bulk capacity still has room"
        );

        let submit = submit_tun_packet_batch_to_mesh_queue_with_backpressure(
            &tx,
            vec![test_pipeline_packet(test_ipv6_udp_packet(8))],
            1,
        );
        tokio::pin!(submit);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut submit)
                .await
                .is_err(),
            "discardable bulk should wait once its high-water budget is full"
        );

        let first = rx.recv().await.expect("queued discardable bulk");
        assert_eq!(first.len(), FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut submit)
                .await
                .expect("discardable bulk should continue after high-water slots release"),
            TunQueueSubmit::Enqueued
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_reliable_bulk_uses_capacity_above_discardable_high_water() {
        let capacity = FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP * 2;
        let (tx, mut rx) = TunPipelineQueueTx::channel(capacity);
        let queued = (0..FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP)
            .map(|_| test_pipeline_packet(test_ipv6_udp_packet(8)))
            .collect::<Vec<_>>();
        let reliable = test_ipv6_tcp_packet(0x18, 512);

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, queued),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue_with_backpressure(
                &tx,
                vec![test_pipeline_packet(reliable.clone())],
                1,
            )
            .await,
            TunQueueSubmit::Enqueued,
            "reliable bulk should keep the full queue capacity"
        );

        let first = rx.recv().await.expect("queued discardable bulk");
        assert_eq!(first.len(), FIPS_TUN_DISCARDABLE_BULK_BACKPRESSURE_CAP);
        let second = rx.recv().await.expect("queued reliable bulk");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].bytes, reliable);
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
