#[cfg(test)]
mod linux_vnet_tun_tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn linux_vnet_tun_env_parser_defaults_on() {
        assert!(linux_vnet_tun_enabled_from_env(None));
        assert!(linux_vnet_tun_enabled_from_env(Some("")));
        assert!(!linux_vnet_tun_enabled_from_env(Some("off")));
        assert!(!linux_vnet_tun_enabled_from_env(Some("0")));
        assert!(linux_vnet_tun_enabled_from_env(Some("1")));
        assert!(linux_vnet_tun_enabled_from_env(Some("true")));
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_env_parser_defaults_on() {
        assert!(linux_vnet_tcp4_gro_write_enabled_from_env(None));
        assert!(linux_vnet_tcp4_gro_write_enabled_from_env(Some("")));
        assert!(linux_vnet_tcp4_gro_write_enabled_from_env(Some("1")));
        assert!(linux_vnet_tcp4_gro_write_enabled_from_env(Some("true")));
        assert!(!linux_vnet_tcp4_gro_write_enabled_from_env(Some("0")));
        assert!(!linux_vnet_tcp4_gro_write_enabled_from_env(Some("off")));
    }

    #[test]
    fn linux_vnet_plain_read_strips_virtio_header() {
        let packet = ipv4_tcp_gso_packet(16, 16, 0x10);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: 0,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_NONE,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("plain vnet read");
        assert_eq!(count, 1);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].bytes.len(), packet.len());
        assert_eq!(&batch[0].bytes[..20], &packet[..20]);
    }

    #[test]
    fn linux_vnet_tcp4_gso_read_splits_into_checked_segments() {
        let packet = ipv4_tcp_gso_packet(2400, 1200, 0x18);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_TCPV4,
            hdr_len: 40,
            gso_size: 1200,
            csum_start: 20,
            csum_offset: 16,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("tcp4 gso read");
        assert_eq!(count, 2);
        assert_eq!(batch.len(), 2);
        assert_eq!(
            batch[0].destination,
            Some(IpAddr::V4(Ipv4Addr::new(10, 44, 0, 2)))
        );
        assert_eq!(
            batch[1].destination,
            Some(IpAddr::V4(Ipv4Addr::new(10, 44, 0, 2)))
        );

        let first = &batch[0].bytes;
        let second = &batch[1].bytes;
        assert_eq!(first.len(), 1240);
        assert_eq!(second.len(), 1240);
        assert_eq!(u16::from_be_bytes([first[2], first[3]]), 1240);
        assert_eq!(u16::from_be_bytes([second[2], second[3]]), 1240);
        assert_eq!(u16::from_be_bytes([first[4], first[5]]), 0x1234);
        assert_eq!(u16::from_be_bytes([second[4], second[5]]), 0x1235);
        assert_eq!(u32::from_be_bytes([first[24], first[25], first[26], first[27]]), 1000);
        assert_eq!(
            u32::from_be_bytes([second[24], second[25], second[26], second[27]]),
            2200
        );
        assert_eq!(first[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_ne!(second[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_eq!(linux_vnet_checksum(&first[..20], 0), 0xffff);
        assert_eq!(linux_vnet_checksum(&second[..20], 0), 0xffff);
        assert_eq!(ipv4_transport_sum(first), 0xffff);
        assert_eq!(ipv4_transport_sum(second), 0xffff);
    }

    #[test]
    fn linux_vnet_tcp4_gso_read_keeps_final_tiny_segment_in_order() {
        let packet = ipv4_tcp_packet(1000, 2500, 0x18);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_TCPV4,
            hdr_len: 40,
            gso_size: 1200,
            csum_start: 20,
            csum_offset: 16,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("tcp4 gso read");
        assert_eq!(count, 3);
        assert_eq!(batch.len(), 3);

        assert_eq!(batch[0].bytes[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_eq!(batch[1].bytes[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_ne!(batch[2].bytes[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_eq!(batch[2].bytes.len(), 140);
    }

    #[test]
    fn linux_vnet_tcp4_gso_read_wraps_sequence_numbers() {
        let first_seq = u32::MAX - 599;
        let packet = ipv4_tcp_packet(first_seq, 2400, 0x18);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_TCPV4,
            hdr_len: 40,
            gso_size: 1200,
            csum_start: 20,
            csum_offset: 16,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("tcp4 gso read");
        assert_eq!(count, 2);
        assert_eq!(
            u32::from_be_bytes([
                batch[0].bytes[24],
                batch[0].bytes[25],
                batch[0].bytes[26],
                batch[0].bytes[27]
            ]),
            first_seq
        );
        assert_eq!(
            u32::from_be_bytes([
                batch[1].bytes[24],
                batch[1].bytes[25],
                batch[1].bytes[26],
                batch[1].bytes[27]
            ]),
            first_seq.wrapping_add(1200)
        );
        assert_eq!(ipv4_transport_sum(&batch[0].bytes), 0xffff);
        assert_eq!(ipv4_transport_sum(&batch[1].bytes), 0xffff);
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_coalesces_adjacent_segments() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(1800, 600, LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first, second];
        let original_packets = packets.clone();
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(packets, original_packets);
        assert_eq!(frames.len(), 1);

        let frame = prepared_write_frame_bytes(&frames[0]);
        let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
        assert_eq!(hdr.flags, LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM);
        assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_TCPV4);
        assert_eq!(hdr.hdr_len, 40);
        assert_eq!(hdr.gso_size, 800);
        assert_eq!(hdr.csum_start, 20);
        assert_eq!(hdr.csum_offset, 16);

        let packet = &frame[LINUX_VIRTIO_NET_HDR_LEN..];
        assert_eq!(packet.len(), 20 + 20 + 1400);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 1440);
        assert_eq!(linux_vnet_checksum(&packet[..20], 0), 0xffff);
        assert_ne!(packet[33] & LINUX_TCP_FLAG_PSH, 0);

        let pseudo = linux_vnet_pseudo_header_sum(
            LINUX_IPPROTO_TCP,
            &packet[12..16],
            &packet[16..20],
            (packet.len() - 20) as u16,
        );
        let expected_partial = !linux_vnet_checksum(&[], pseudo);
        assert_eq!(
            u16::from_be_bytes([packet[36], packet[37]]),
            expected_partial
        );
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_coalesces_interleaved_flows() {
        let mut first_a = ipv4_tcp_packet_with_ports(1000, 800, LINUX_TCP_FLAG_ACK, 443, 45172);
        let mut first_b = ipv4_tcp_packet_with_ports(7000, 800, LINUX_TCP_FLAG_ACK, 443, 45173);
        let mut second_a = ipv4_tcp_packet_with_ports(
            1800,
            600,
            LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH,
            443,
            45172,
        );
        let mut second_b = ipv4_tcp_packet_with_ports(
            7800,
            600,
            LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH,
            443,
            45173,
        );
        for packet in [&mut first_a, &mut first_b, &mut second_a, &mut second_b] {
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(packet);
        }

        let packets = vec![first_a, first_b, second_a, second_b];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 2);

        let first = prepared_write_frame_bytes(&frames[0]);
        let first_hdr = LinuxVirtioNetHdr::decode(&first).expect("first virtio header");
        assert_eq!(first_hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_TCPV4);
        assert_eq!(first_hdr.gso_size, 800);
        assert_eq!(u16::from_be_bytes([first[32], first[33]]), 45172);
        assert_eq!(first.len(), LINUX_VIRTIO_NET_HDR_LEN + 20 + 20 + 1400);

        let second = prepared_write_frame_bytes(&frames[1]);
        let second_hdr = LinuxVirtioNetHdr::decode(&second).expect("second virtio header");
        assert_eq!(second_hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_TCPV4);
        assert_eq!(second_hdr.gso_size, 800);
        assert_eq!(u16::from_be_bytes([second[32], second[33]]), 45173);
        assert_eq!(second.len(), LINUX_VIRTIO_NET_HDR_LEN + 20 + 20 + 1400);
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_keeps_noncandidate_boundary() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut fin = ipv4_tcp_packet(1800, 1, LINUX_TCP_FLAG_FIN | LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(1800, 600, LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH);
        for packet in [&mut first, &mut fin, &mut second] {
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(packet);
        }

        let packets = vec![first, fin, second];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 3);
        for frame in [frames.first().unwrap(), frames.last().unwrap()] {
            let frame = prepared_write_frame_bytes(frame);
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
        }
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_coalesces_wrapped_sequences() {
        let first_seq = u32::MAX - 399;
        let mut first = ipv4_tcp_packet(first_seq, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(
            first_seq.wrapping_add(800),
            600,
            LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH,
        );
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first, second];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 1);

        let frame = prepared_write_frame_bytes(&frames[0]);
        let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
        assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_TCPV4);
        assert_eq!(hdr.gso_size, 800);

        let packet = &frame[LINUX_VIRTIO_NET_HDR_LEN..];
        assert_eq!(
            u32::from_be_bytes([packet[24], packet[25], packet[26], packet[27]]),
            first_seq
        );
        assert_eq!(packet.len(), 20 + 20 + 1400);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 1440);
        assert_ne!(packet[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_eq!(linux_vnet_checksum(&packet[..20], 0), 0xffff);
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_keeps_sequence_gap_separate() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(2000, 600, LINUX_TCP_FLAG_ACK);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first, second];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 2);
        for frame in frames {
            let frame = prepared_write_frame_bytes(&frame);
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
            assert_eq!(hdr.gso_size, 0);
        }
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_can_be_disabled() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(1800, 600, LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first.clone(), second.clone()];
        let frames = linux_vnet_prepare_write_frames_with_gro(&packets, false);
        assert_eq!(frames.len(), 2);

        for (frame, packet) in frames.iter().zip([first, second]) {
            assert!(matches!(frame.0, LinuxVnetPreparedWriteFrame::RawPacket(_)));
            let frame = prepared_write_frame_bytes(frame);
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
            assert_eq!(hdr.gso_size, 0);
            assert_eq!(&frame[LINUX_VIRTIO_NET_HDR_LEN..], packet.as_slice());
        }
    }

    #[test]
    fn linux_vnet_udp4_gro_write_coalesces_adjacent_datagrams() {
        let mut first = ipv4_udp_packet(1000);
        let mut second = ipv4_udp_packet(600);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first.clone(), second.clone()];
        let frames = linux_vnet_prepare_write_frames_with_udp4_gro(&packets);
        assert_eq!(frames.len(), 1);

        let frame = prepared_write_frame_bytes(&frames[0]);
        let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
        assert_eq!(hdr.flags, LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM);
        assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_UDP_L4);
        assert_eq!(hdr.hdr_len, 28);
        assert_eq!(hdr.gso_size, 1000);
        assert_eq!(hdr.csum_start, 20);
        assert_eq!(hdr.csum_offset, 6);

        let packet = &frame[LINUX_VIRTIO_NET_HDR_LEN..];
        assert_eq!(packet.len(), 20 + LINUX_UDP_HEADER_LEN + 1600);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 1628);
        assert_eq!(u16::from_be_bytes([packet[24], packet[25]]), 1608);
        assert_eq!(linux_vnet_checksum(&packet[..20], 0), 0xffff);
        assert_eq!(
            &packet[28..28 + 1000],
            &first[20 + LINUX_UDP_HEADER_LEN..]
        );
        assert_eq!(&packet[28 + 1000..], &second[20 + LINUX_UDP_HEADER_LEN..]);

        let pseudo = linux_vnet_pseudo_header_sum(
            LINUX_IPPROTO_UDP,
            &packet[12..16],
            &packet[16..20],
            (packet.len() - 20) as u16,
        );
        let expected_partial = !linux_vnet_checksum(&[], pseudo);
        assert_eq!(
            u16::from_be_bytes([packet[26], packet[27]]),
            expected_partial
        );
    }

    #[test]
    fn linux_vnet_udp4_gro_write_caps_at_sixteen_segments() {
        let mut packets = Vec::new();
        for index in 0..17 {
            let mut packet = ipv4_udp_packet(100);
            packet[4..6].copy_from_slice(&(0x4000_u16 + index).to_be_bytes());
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut packet);
            packets.push(packet);
        }

        let frames = linux_vnet_prepare_write_frames_with_udp4_gro(&packets);
        assert_eq!(frames.len(), 2);

        let first = prepared_write_frame_bytes(&frames[0]);
        let first_hdr = LinuxVirtioNetHdr::decode(&first).expect("first virtio header");
        assert_eq!(first_hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_UDP_L4);
        assert_eq!(first_hdr.gso_size, 100);
        assert_eq!(
            first.len(),
            LINUX_VIRTIO_NET_HDR_LEN + 20 + LINUX_UDP_HEADER_LEN + 1600
        );

        let second = prepared_write_frame_bytes(&frames[1]);
        let second_hdr = LinuxVirtioNetHdr::decode(&second).expect("second virtio header");
        assert_eq!(second_hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
        assert_eq!(second_hdr.gso_size, 0);
        assert_eq!(
            second.len(),
            LINUX_VIRTIO_NET_HDR_LEN + 20 + LINUX_UDP_HEADER_LEN + 100
        );
    }

    #[test]
    fn linux_vnet_udp4_gro_write_skips_zero_or_bad_checksum() {
        let mut zero_first = ipv4_udp_packet(500);
        let mut zero_second = ipv4_udp_packet(500);
        let mut bad_first = ipv4_udp_packet(500);
        let mut bad_second = ipv4_udp_packet(500);
        for packet in [&mut zero_first, &mut zero_second] {
            packet[26] = 0;
            packet[27] = 0;
        }
        for packet in [&mut bad_first, &mut bad_second] {
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(packet);
            packet[27] ^= 0x7f;
        }

        let packets = [zero_first, zero_second, bad_first, bad_second];
        let frames = linux_vnet_prepare_write_frames_with_udp4_gro(&packets);
        assert_eq!(frames.len(), packets.len());
        for (frame, packet) in frames.iter().zip(packets) {
            assert!(matches!(frame.0, LinuxVnetPreparedWriteFrame::RawPacket(_)));
            let frame = prepared_write_frame_bytes(frame);
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
            assert_eq!(&frame[LINUX_VIRTIO_NET_HDR_LEN..], packet.as_slice());
        }
    }

    #[test]
    fn linux_vnet_udp4_gro_write_keeps_tcp_boundary() {
        let mut first_tcp = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut udp = ipv4_udp_packet(100);
        let mut second_tcp =
            ipv4_tcp_packet(1800, 600, LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH);
        for packet in [&mut first_tcp, &mut udp, &mut second_tcp] {
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(packet);
        }

        let packets = vec![first_tcp, udp, second_tcp];
        let frames = linux_vnet_prepare_write_frames_with_udp4_gro(&packets);
        assert_eq!(frames.len(), 3);
        for frame in frames {
            let frame = prepared_write_frame_bytes(&frame);
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
        }
    }

    fn prepared_write_frame_bytes(frame: &(LinuxVnetPreparedWriteFrame, Vec<u8>)) -> Vec<u8> {
        frame.1.clone()
    }

    fn ipv4_tcp_gso_packet(payload_len: usize, gso_size: usize, flags: u8) -> Vec<u8> {
        let packet = ipv4_tcp_packet(1000, payload_len, flags);
        assert_eq!(payload_len % gso_size, 0);
        packet
    }

    fn ipv4_tcp_packet(seq: u32, payload_len: usize, flags: u8) -> Vec<u8> {
        ipv4_tcp_packet_with_ports(seq, payload_len, flags, 443, 45172)
    }

    fn ipv4_tcp_packet_with_ports(
        seq: u32,
        payload_len: usize,
        flags: u8,
        src_port: u16,
        dst_port: u16,
    ) -> Vec<u8> {
        let total_len = 20 + 20 + payload_len;
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[4..6].copy_from_slice(&0x1234_u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = LINUX_IPPROTO_TCP;
        packet[12..16].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 1).octets());
        packet[16..20].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 2).octets());
        packet[20..22].copy_from_slice(&src_port.to_be_bytes());
        packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
        packet[24..28].copy_from_slice(&seq.to_be_bytes());
        packet[28..32].copy_from_slice(&777_u32.to_be_bytes());
        packet[32] = 5 << 4;
        packet[33] = flags;
        packet[34..36].copy_from_slice(&65535_u16.to_be_bytes());
        for i in 0..payload_len {
            packet[40 + i] = (i % 251) as u8;
        }
        packet
    }

    fn ipv4_udp_packet(payload_len: usize) -> Vec<u8> {
        ipv4_udp_packet_with_ports(payload_len, 51820, 45172)
    }

    fn ipv4_udp_packet_with_ports(payload_len: usize, src_port: u16, dst_port: u16) -> Vec<u8> {
        let total_len = 20 + LINUX_UDP_HEADER_LEN + payload_len;
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[4..6].copy_from_slice(&0x4321_u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = LINUX_IPPROTO_UDP;
        packet[12..16].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 1).octets());
        packet[16..20].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 2).octets());
        packet[20..22].copy_from_slice(&src_port.to_be_bytes());
        packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
        packet[24..26].copy_from_slice(&((LINUX_UDP_HEADER_LEN + payload_len) as u16).to_be_bytes());
        packet[26..28].copy_from_slice(&0xffff_u16.to_be_bytes());
        for i in 0..payload_len {
            packet[20 + LINUX_UDP_HEADER_LEN + i] = (i % 251) as u8;
        }
        packet
    }

    fn ipv4_transport_sum(packet: &[u8]) -> u16 {
        let transport_len = packet.len() - 20;
        let pseudo = linux_vnet_pseudo_header_sum(
            packet[9],
            &packet[12..16],
            &packet[16..20],
            transport_len as u16,
        );
        linux_vnet_checksum(&packet[20..], pseudo)
    }
}
