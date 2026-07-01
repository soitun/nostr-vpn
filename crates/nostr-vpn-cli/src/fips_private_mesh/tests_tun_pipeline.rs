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
    fn mesh_receive_write_batch_preserves_app_packet_order() {
        let bulk = test_ipv4_tcp_packet(0x18, 512);
        let ping = test_ipv4_icmp_packet();
        let mut batch = TunWriteBatch::with_capacity(2);

        push_mesh_packet_for_tun(bulk.into(), &mut batch);
        push_mesh_packet_for_tun(ping.into(), &mut batch);

        assert_eq!(batch.packets.len(), 2);
        assert_eq!(batch.packets[0][9], 6);
        assert_eq!(batch.packets[1][9], 1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn full_tun_to_mesh_queue_drops_bulk_without_waiting() {
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

        let queued = rx.recv().await.expect("first batch should stay queued");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].bytes, test_ipv6_tcp_packet(0x18, 512));
        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_rx_exposes_bulk_backlog_counter() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(2);

        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(
                &tx,
                vec![test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512))],
            ),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(rx.bulk_backlog_batches(), 1);
        assert_eq!(rx.bulk_backlog_packets(), 1);

        let queued = rx.recv().await.expect("queued bulk batch");
        assert_eq!(queued.len(), 1);
        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_queue_is_bounded_by_packets_not_batches() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(3);
        let first = vec![
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 512)),
            test_pipeline_packet(test_ipv6_tcp_packet(0x18, 513)),
        ];
        let second = vec![
            test_pipeline_packet(test_ipv6_udp_packet(8)),
            test_pipeline_packet(test_ipv6_udp_packet(9)),
        ];

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, first),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(rx.bulk_backlog_batches(), 1);
        assert_eq!(rx.bulk_backlog_packets(), 2);
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, second),
            TunQueueSubmit::DroppedBulk
        );
        assert_eq!(rx.bulk_backlog_batches(), 1);
        assert_eq!(rx.bulk_backlog_packets(), 2);

        let queued = rx.recv().await.expect("first batch should remain");
        assert_eq!(queued.len(), 2);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_app_bulk_drops_on_bounded_batch_channel() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(1);
        let first = test_ipv6_tcp_packet(0x18, 512);
        let second = test_ipv6_tcp_packet(0x18, 513);

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(first.clone())]),
            TunQueueSubmit::Enqueued
        );

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(second)]),
            TunQueueSubmit::DroppedBulk
        );

        let first_batch = rx.recv().await.expect("first app batch");
        assert_eq!(first_batch[0].bytes, first);
        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_udp_app_packet_drops_when_batch_channel_full() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(1);
        let first = test_ipv6_udp_packet(8);
        let second = test_ipv6_udp_packet(9);

        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(first.clone())]),
            TunQueueSubmit::Enqueued
        );
        assert_eq!(
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(second)]),
            TunQueueSubmit::DroppedBulk
        );

        let first_batch = rx.recv().await.expect("first UDP app batch");
        assert_eq!(first_batch[0].bytes, first);
        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn full_tun_to_mesh_queue_treats_icmp_as_bulk_app_payload() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(1);
        let bulk_first = test_ipv6_tcp_packet(0x18, 512);
        let bulk_dropped = test_ipv6_tcp_packet(0x18, 512);
        let ping = test_ipv4_icmp_packet();

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
            submit_tun_packet_batch_to_mesh_queue(&tx, vec![test_pipeline_packet(ping)],),
            TunQueueSubmit::DroppedBulk
        );

        let queued_bulk = rx.recv().await.expect("first bulk should stay queued");
        assert_eq!(queued_bulk.len(), 1);
        assert_eq!(queued_bulk[0].bytes, bulk_first);
        assert_eq!(rx.bulk_backlog_batches(), 0);
        assert_eq!(rx.bulk_backlog_packets(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn tun_to_mesh_queue_keeps_mixed_app_batch_together() {
        let (tx, mut rx) = TunPipelineQueueTx::channel(3);
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

        let queued_bulk = rx.recv().await.expect("bulk batch");
        assert_eq!(queued_bulk.len(), 3);
        assert_eq!(queued_bulk[0].bytes, bulk);
        assert_eq!(queued_bulk[1].bytes, ack);
        assert_eq!(queued_bulk[2].bytes, ping);
    }
