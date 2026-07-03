    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn tun_pipeline_packet_caches_destination_for_send_route() {
        let source = Ipv4Addr::new(10, 44, 10, 1);
        let destination = Ipv4Addr::new(10, 44, 22, 44);
        let packet = TunPipelinePacket::new(ipv4_packet(source, destination));

        assert_eq!(packet.destination, Some(IpAddr::V4(destination)));
    }
