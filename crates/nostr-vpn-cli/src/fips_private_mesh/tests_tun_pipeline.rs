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
        let expected_bytes = bulk.len() + ping.len();
        let mut batch = TunWriteBatch::with_capacity(2);

        push_direct_packet_output_for_tun(bulk.into(), &mut batch);
        push_direct_packet_output_for_tun(ping.into(), &mut batch);

        assert_eq!(batch.len(), 2);
        let packets = batch.packet_slices_for_test();
        assert_eq!(packets[0][9], 6);
        assert_eq!(packets[1][9], 1);
        assert_eq!(batch.bytes(), expected_bytes);
    }
