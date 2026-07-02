mod tests {
    use super::{FipsMeshPeerConfig, FipsMeshRuntime, endpoint_node_addr_from_pubkey_bytes};
    use nostr_sdk::prelude::{Keys, ToBech32};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[derive(Debug, Clone)]
    struct TestPeer {
        participant_pubkey: String,
        endpoint_npub: String,
        endpoint_pubkey: [u8; 32],
        endpoint_node_addr: [u8; 16],
    }

    impl TestPeer {
        fn generate() -> Self {
            let keys = Keys::generate();
            let endpoint_pubkey = keys.public_key().to_hex();
            let endpoint_pubkey_bytes = *keys.public_key().as_bytes();
            Self {
                participant_pubkey: endpoint_pubkey.clone(),
                endpoint_npub: keys.public_key().to_bech32().expect("npub"),
                endpoint_pubkey: endpoint_pubkey_bytes,
                endpoint_node_addr: endpoint_node_addr_from_pubkey_bytes(endpoint_pubkey_bytes),
            }
        }
    }

    fn runtime() -> FipsMeshRuntime {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub,
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey,
                endpoint_npub: specific.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ])
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

    fn ipv6_packet(source: Ipv6Addr, destination: Ipv6Addr) -> Vec<u8> {
        let payload = [0xde, 0xad, 0xbe, 0xef];
        let mut packet = vec![0_u8; 40 + payload.len()];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        packet[6] = 17;
        packet[7] = 64;
        packet[8..24].copy_from_slice(&source.octets());
        packet[24..40].copy_from_slice(&destination.octets());
        packet[40..].copy_from_slice(&payload);
        packet
    }

    #[test]
    fn peer_config_from_participant_pubkey_derives_endpoint_npub() {
        let peer = TestPeer::generate();

        let config = FipsMeshPeerConfig::from_participant_pubkey(
            &peer.participant_pubkey,
            vec!["10.44.22.44/32".to_string()],
        )
        .expect("peer config");

        assert_eq!(config.participant_pubkey, peer.participant_pubkey);
        assert_eq!(config.endpoint_npub, peer.endpoint_npub);
        assert_eq!(config.allowed_ips, vec!["10.44.22.44/32"]);
    }

    #[test]
    fn runtime_caches_canonical_endpoint_npub_for_edge_apis() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: format!(" {} ", hex::encode(peer.endpoint_pubkey)),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        assert_eq!(
            runtime.peer_endpoint_npub(&peer.participant_pubkey),
            Some(peer.endpoint_npub.clone())
        );
        assert_eq!(
            runtime.participant_for_endpoint_npub(&peer.endpoint_npub),
            Some(peer.participant_pubkey.clone())
        );
        assert_eq!(runtime.peer_statuses()[0].endpoint_npub, peer.endpoint_npub);
        assert_eq!(
            runtime
                .route_outbound_packet(&packet)
                .expect("packet should route")
                .endpoint_npub,
            peer.endpoint_npub
        );
    }

    #[test]
    fn runtime_indexes_participants_by_pubkey_bytes_and_outputs_hex_edges() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let inbound_packet =
            ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: format!(" {} ", peer.endpoint_npub),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        assert_eq!(
            runtime.peer_pubkeys(),
            vec![peer.participant_pubkey.clone()]
        );
        assert_eq!(runtime.peer_statuses()[0].pubkey, peer.participant_pubkey);
        assert_eq!(
            runtime.peer_endpoint_node_addr(&peer.endpoint_npub),
            Some(peer.endpoint_node_addr)
        );
        assert_eq!(
            runtime.peer_endpoint_node_addr_for_participant_pubkey_bytes(&peer.endpoint_pubkey),
            Some(peer.endpoint_node_addr)
        );
        assert_eq!(
            runtime.participant_pubkey_bytes_for_endpoint_node_addr(&peer.endpoint_node_addr),
            Some(peer.endpoint_pubkey)
        );
        assert_eq!(
            runtime
                .route_outbound_packet_with_peer(&packet)
                .expect("packet should route")
                .participant_pubkey,
            peer.participant_pubkey
        );
        assert_eq!(
            runtime
                .route_outbound_packet_peer(&packet)
                .expect("packet should route")
                .participant_pubkey_bytes,
            Some(&peer.endpoint_pubkey)
        );
        assert_eq!(
            runtime
                .receive_endpoint_data_owned_with_source_node_addr(
                    &peer.endpoint_node_addr,
                    inbound_packet
                )
                .expect("packet should be admitted")
                .source_pubkey_bytes,
            Some(&peer.endpoint_pubkey)
        );
    }

    #[test]
    fn runtime_identity_indexes_preserve_first_match_lookup_behavior() {
        let first = TestPeer::generate();
        let second = TestPeer::generate();
        let duplicate_participant = TestPeer::generate();
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: first.participant_pubkey.clone(),
                endpoint_npub: first.endpoint_npub.clone(),
                allowed_ips: Vec::new(),
            },
            FipsMeshPeerConfig {
                participant_pubkey: first.participant_pubkey.clone(),
                endpoint_npub: duplicate_participant.endpoint_npub.clone(),
                allowed_ips: Vec::new(),
            },
            FipsMeshPeerConfig {
                participant_pubkey: second.participant_pubkey.clone(),
                endpoint_npub: first.endpoint_npub.clone(),
                allowed_ips: Vec::new(),
            },
        ]);

        assert_eq!(
            runtime.peer_endpoint_npub(&first.participant_pubkey),
            Some(first.endpoint_npub.clone())
        );
        assert_eq!(
            runtime.peer_endpoint_node_addr(&first.participant_pubkey),
            Some(first.endpoint_node_addr)
        );
        assert_eq!(
            runtime.participant_for_endpoint_npub(&first.endpoint_npub),
            Some(first.participant_pubkey.clone())
        );
        assert_eq!(
            runtime.participant_for_endpoint_node_addr(&first.endpoint_node_addr),
            Some(first.participant_pubkey)
        );
    }

    #[test]
    fn peer_config_detects_default_route_advertisement() {
        let peer = TestPeer::generate();
        let config = FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub,
            allowed_ips: vec![
                "10.44.22.44/32".to_string(),
                " 0.0.0.0/0 ".to_string(),
                "::/0".to_string(),
            ],
        };

        assert!(config.advertises_default_route());
    }

    #[test]
    fn peer_config_ignores_non_default_routes() {
        let peer = TestPeer::generate();
        let config = FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub,
            allowed_ips: vec!["10.44.0.0/16".to_string(), "fd00::/8".to_string()],
        };

        assert!(!config.advertises_default_route());
    }

    #[test]
    fn outbound_packet_uses_longest_prefix_route() {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub,
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey.clone(),
                endpoint_npub: specific.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        assert_eq!(runtime.exact_route_peer_index.len(), 1);
        let outgoing = runtime
            .route_outbound_packet(&packet)
            .expect("packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, specific.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);

        let outgoing = runtime
            .route_outbound_packet_with_peer(&packet)
            .expect("borrowed-peer packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_pubkey, &specific.endpoint_pubkey);
        assert_eq!(outgoing.endpoint_node_addr, &specific.endpoint_node_addr);
        assert_eq!(outgoing.bytes, packet);

        let outgoing = runtime
            .route_outbound_packet_owned(packet.clone())
            .expect("owned packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, specific.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);

        let outgoing = runtime
            .route_outbound_packet_peer(&packet)
            .expect("metadata-only packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_pubkey, &specific.endpoint_pubkey);
        assert_eq!(outgoing.endpoint_node_addr, &specific.endpoint_node_addr);

        let outgoing = runtime
            .route_outbound_packet_owned_with_peer(packet.clone())
            .expect("borrowed-peer owned packet should route");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_pubkey, &specific.endpoint_pubkey);
        assert_eq!(outgoing.endpoint_node_addr, &specific.endpoint_node_addr);
        assert_eq!(outgoing.bytes, packet);

        let cached_destination = IpAddr::V4(Ipv4Addr::new(10, 44, 22, 44));
        let outgoing = runtime
            .route_outbound_packet_owned_with_peer_to_destination(
                packet.clone(),
                cached_destination,
            )
            .expect("cached destination should route through current table");

        assert_eq!(outgoing.participant_pubkey, specific.participant_pubkey);
        assert_eq!(outgoing.endpoint_pubkey, &specific.endpoint_pubkey);
        assert_eq!(outgoing.endpoint_node_addr, &specific.endpoint_node_addr);
        assert_eq!(outgoing.bytes, packet);

        assert!(
            runtime
                .route_outbound_packet_owned_with_peer_to_destination(
                    packet.clone(),
                    IpAddr::V4(Ipv4Addr::new(192, 0, 2, 44)),
                )
                .is_none(),
            "cached destination must still consult the current route table"
        );
    }

    #[test]
    fn fallback_prefix_index_skips_exact_routes_and_preserves_longest_prefix() {
        let exact = TestPeer::generate();
        let subnet = TestPeer::generate();
        let exit = TestPeer::generate();
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: exact.participant_pubkey.clone(),
                endpoint_npub: exact.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: subnet.participant_pubkey.clone(),
                endpoint_npub: subnet.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: exit.participant_pubkey.clone(),
                endpoint_npub: exit.endpoint_npub.clone(),
                allowed_ips: vec![
                    "0.0.0.0/0".to_string(),
                    "fd00::/8".to_string(),
                    "fd00:44::/48".to_string(),
                ],
            },
        ]);

        assert_eq!(runtime.exact_route_peer_index.len(), 1);
        assert_eq!(
            runtime
                .prefix_v4_route_peer_index
                .iter()
                .map(|route| route.route.prefix_len)
                .collect::<Vec<_>>(),
            vec![16, 0]
        );
        assert_eq!(
            runtime
                .prefix_v6_route_peer_index
                .iter()
                .map(|route| route.route.prefix_len)
                .collect::<Vec<_>>(),
            vec![48, 8]
        );

        let exact_packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let exact_outgoing = runtime
            .route_outbound_packet_with_peer(&exact_packet)
            .expect("exact route should use exact index");
        assert_eq!(exact_outgoing.participant_pubkey, exact.participant_pubkey);

        let subnet_packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 5, 9));
        let subnet_outgoing = runtime
            .route_outbound_packet_with_peer(&subnet_packet)
            .expect("subnet route should win over default route");
        assert_eq!(
            subnet_outgoing.participant_pubkey,
            subnet.participant_pubkey
        );

        let exit_packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(8, 8, 8, 8));
        let exit_outgoing = runtime
            .route_outbound_packet_with_peer(&exit_packet)
            .expect("default route should handle non-peer destination");
        assert_eq!(exit_outgoing.participant_pubkey, exit.participant_pubkey);
    }

    #[test]
    fn duplicate_exact_routes_for_same_peer_are_not_ambiguous() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(10, 44, 22, 44));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string(), "10.44.22.44/32".to_string()],
        }]);

        let outgoing = runtime
            .route_outbound_packet(&packet)
            .expect("duplicate same-peer route should still route");

        assert_eq!(outgoing.participant_pubkey, peer.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, peer.endpoint_npub);
    }

    #[test]
    fn outbound_packet_without_route_is_dropped() {
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 10, 1), Ipv4Addr::new(192, 0, 2, 10));

        assert!(runtime().route_outbound_packet(&packet).is_none());
        assert!(runtime().route_outbound_packet_peer(&packet).is_none());
    }

    #[test]
    fn inbound_endpoint_data_accepts_roster_source_with_owned_packet_source() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        let received = runtime
            .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
            .expect("source npub and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_owned(Some(&peer.endpoint_npub), packet.clone())
            .expect("owned source npub and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_owned_with_source(Some(&peer.endpoint_npub), packet.clone())
            .expect("borrowed-source npub and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_from_node_addr(&peer.endpoint_node_addr, &packet)
            .expect("source node addr and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_owned_from_node_addr(&peer.endpoint_node_addr, packet.clone())
            .expect("owned source node addr and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);

        let received = runtime
            .receive_endpoint_data_owned_with_source_node_addr(
                &peer.endpoint_node_addr,
                packet.clone(),
            )
            .expect("borrowed-source node addr and packet source should be admitted");

        assert_eq!(received.source_pubkey, peer.participant_pubkey);
        assert_eq!(received.bytes, packet);
    }

    #[test]
    fn inbound_endpoint_source_run_admits_with_single_source_identity() {
        let peer = TestPeer::generate();
        let admitted = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let rejected = ipv4_packet(Ipv4Addr::new(192, 0, 2, 10), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        let mut accepted = Vec::new();
        let source_run = runtime
            .endpoint_source_admitter(&peer.endpoint_node_addr)
            .expect("source endpoint should be configured")
            .receive_owned_source_run_into(vec![admitted.clone(), rejected], |packet| {
                accepted.push(packet)
            })
            .expect("at least one packet in source-run should be admitted");

        assert_eq!(source_run.source_pubkey, peer.participant_pubkey);
        assert_eq!(source_run.source_pubkey_bytes, Some(&peer.endpoint_pubkey));
        assert_eq!(source_run.endpoint_bytes, admitted.len());
        assert_eq!(source_run.len(), 1);
        assert_eq!(accepted, vec![admitted]);
    }

    #[test]
    fn inbound_endpoint_data_drops_unknown_source_npub() {
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));

        assert!(
            runtime()
                .receive_endpoint_data(Some("npub1unknown"), &packet)
                .is_none()
        );
    }

    #[test]
    fn inbound_endpoint_data_drops_unknown_source_node_addr() {
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));

        assert!(
            runtime()
                .receive_endpoint_data_from_node_addr(&[7_u8; 16], &packet)
                .is_none()
        );
    }

    #[test]
    fn inbound_endpoint_data_drops_known_npub_with_unowned_packet_source() {
        let peer = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(192, 0, 2, 10), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey,
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["10.44.22.44/32".to_string()],
        }]);

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn inbound_endpoint_data_rejects_broad_route_spoofing_specific_peer_source() {
        let general = TestPeer::generate();
        let specific = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: general.participant_pubkey,
                endpoint_npub: general.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.0.0/16".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: specific.participant_pubkey,
                endpoint_npub: specific.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        assert!(
            runtime
                .receive_endpoint_data(Some(&general.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn equal_prefix_route_ambiguity_is_dropped() {
        let first = TestPeer::generate();
        let second = TestPeer::generate();
        let packet = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: first.participant_pubkey,
                endpoint_npub: first.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: second.participant_pubkey,
                endpoint_npub: second.endpoint_npub,
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            },
        ]);

        assert!(runtime.route_outbound_packet(&packet).is_none());
        assert!(
            runtime
                .receive_endpoint_data(Some(&first.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn equal_prefix_ipv6_exact_route_ambiguity_is_dropped() {
        let first = TestPeer::generate();
        let second = TestPeer::generate();
        let source = "fd00:44::1".parse().expect("source");
        let destination = "fd00:44::10".parse().expect("destination");
        let packet = ipv6_packet(source, destination);
        let runtime = FipsMeshRuntime::new(vec![
            FipsMeshPeerConfig {
                participant_pubkey: first.participant_pubkey,
                endpoint_npub: first.endpoint_npub.clone(),
                allowed_ips: vec!["fd00:44::10/128".to_string()],
            },
            FipsMeshPeerConfig {
                participant_pubkey: second.participant_pubkey,
                endpoint_npub: second.endpoint_npub,
                allowed_ips: vec!["fd00:44::10/128".to_string()],
            },
        ]);

        assert!(runtime.route_outbound_packet(&packet).is_none());
        assert!(
            runtime
                .receive_endpoint_data(Some(&first.endpoint_npub), &packet)
                .is_none()
        );
    }

    #[test]
    fn local_routes_limit_inbound_packet_destinations() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::with_local_routes(
            vec![FipsMeshPeerConfig {
                participant_pubkey: peer.participant_pubkey,
                endpoint_npub: peer.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            }],
            vec!["10.44.10.1/32".to_string()],
        );
        let admitted = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 1));
        let rejected = ipv4_packet(Ipv4Addr::new(10, 44, 22, 44), Ipv4Addr::new(10, 44, 10, 2));

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &admitted)
                .is_some()
        );
        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &rejected)
                .is_none()
        );
    }

    #[test]
    fn local_default_route_allows_exit_node_destinations() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::with_local_routes(
            vec![FipsMeshPeerConfig {
                participant_pubkey: peer.participant_pubkey,
                endpoint_npub: peer.endpoint_npub.clone(),
                allowed_ips: vec!["10.44.22.44/32".to_string()],
            }],
            vec!["0.0.0.0/0".to_string()],
        );
        let packet = ipv4_packet(
            Ipv4Addr::new(10, 44, 22, 44),
            Ipv4Addr::new(203, 0, 113, 10),
        );

        assert!(
            runtime
                .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
                .is_some()
        );
    }

    #[test]
    fn two_device_private_mesh_routes_and_admits_bidirectional_packets() {
        let alice = TestPeer::generate();
        let bob = TestPeer::generate();
        let alice_ip = Ipv4Addr::new(10, 44, 1, 10);
        let bob_ip = Ipv4Addr::new(10, 44, 1, 20);
        let alice_runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: bob.participant_pubkey.clone(),
            endpoint_npub: bob.endpoint_npub.clone(),
            allowed_ips: vec![format!("{bob_ip}/32")],
        }]);
        let bob_runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: alice.participant_pubkey.clone(),
            endpoint_npub: alice.endpoint_npub.clone(),
            allowed_ips: vec![format!("{alice_ip}/32")],
        }]);

        let alice_to_bob = ipv4_packet(alice_ip, bob_ip);
        let outgoing = alice_runtime
            .route_outbound_packet(&alice_to_bob)
            .expect("Alice should route packet to Bob");
        assert_eq!(outgoing.participant_pubkey, bob.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, bob.endpoint_npub);
        let received = bob_runtime
            .receive_endpoint_data(Some(&alice.endpoint_npub), &outgoing.bytes)
            .expect("Bob should admit Alice's owned source IP");
        assert_eq!(received.source_pubkey, alice.participant_pubkey);
        assert_eq!(received.bytes, alice_to_bob);

        let bob_to_alice = ipv4_packet(bob_ip, alice_ip);
        let outgoing = bob_runtime
            .route_outbound_packet(&bob_to_alice)
            .expect("Bob should route packet to Alice");
        assert_eq!(outgoing.participant_pubkey, alice.participant_pubkey);
        assert_eq!(outgoing.endpoint_npub, alice.endpoint_npub);
        let received = alice_runtime
            .receive_endpoint_data(Some(&bob.endpoint_npub), &outgoing.bytes)
            .expect("Alice should admit Bob's owned source IP");
        assert_eq!(received.source_pubkey, bob.participant_pubkey);
        assert_eq!(received.bytes, bob_to_alice);
    }

    #[test]
    fn ipv6_routes_are_supported_for_raw_packets() {
        let peer = TestPeer::generate();
        let runtime = FipsMeshRuntime::new(vec![FipsMeshPeerConfig {
            participant_pubkey: peer.participant_pubkey.clone(),
            endpoint_npub: peer.endpoint_npub.clone(),
            allowed_ips: vec!["fd00:44::/64".to_string()],
        }]);
        let packet = ipv6_packet(
            "fd00:44::20".parse().expect("source"),
            "fd00:44::10".parse().expect("destination"),
        );

        let outgoing = runtime
            .route_outbound_packet(&packet)
            .expect("IPv6 packet should route");
        let received = runtime
            .receive_endpoint_data(Some(&peer.endpoint_npub), &packet)
            .expect("IPv6 source should be admitted");

        assert_eq!(outgoing.endpoint_npub, peer.endpoint_npub);
        assert_eq!(outgoing.bytes, packet);
        assert_eq!(received.source_pubkey, peer.participant_pubkey);
    }
}
