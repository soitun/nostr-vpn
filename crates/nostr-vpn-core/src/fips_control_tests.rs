#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_roundtrips_with_magic_prefix() {
        let frame = FipsControlFrame::Ping {
            network_id: "mesh".to_string(),
            sent_at: 42,
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        assert!(encoded.starts_with(FIPS_CONTROL_MAGIC));

        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn raw_packet_is_not_control() {
        let packet = [0x45, 0, 0, 20];

        assert!(
            decode_fips_control_frame(&packet)
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn capabilities_frame_roundtrips() {
        let frame = FipsControlFrame::Capabilities {
            network_id: "mesh".to_string(),
            capabilities: PeerCapabilities {
                advertised_routes: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
                endpoint_hints: vec![PeerEndpointHint::udp("192.168.50.22:51820")],
                dataplane_features: vec!["future_feature".to_string()],
                signed_at: 99,
            },
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn old_capabilities_decode_with_empty_endpoint_hints() {
        let caps: PeerCapabilities =
            serde_json::from_str(r#"{"advertised_routes":["0.0.0.0/0"],"signed_at":99}"#)
                .expect("decode old capabilities");

        assert_eq!(caps.advertised_routes, vec!["0.0.0.0/0".to_string()]);
        assert!(caps.endpoint_hints.is_empty());
        assert!(caps.dataplane_features.is_empty());
        assert_eq!(caps.signed_at, 99);
    }

    #[test]
    fn peer_capabilities_match_dataplane_features_case_insensitively() {
        let caps = PeerCapabilities {
            dataplane_features: vec!["FUTURE_FEATURE".to_string()],
            ..PeerCapabilities::default()
        };

        assert!(caps.supports_dataplane_feature("future_feature"));
        assert!(!caps.supports_dataplane_feature("unknown_feature"));
    }

    #[test]
    fn local_dataplane_features_are_empty_without_protocol_extensions() {
        assert!(local_fips_dataplane_features().is_empty());
    }

    #[test]
    fn signed_roster_verifies_independent_of_alias_map_order() {
        let admin = Keys::generate();
        let alice = Keys::generate().public_key().to_hex();
        let bob = Keys::generate().public_key().to_hex();
        let mut aliases = HashMap::new();
        aliases.insert(bob.clone(), "bob".to_string());
        aliases.insert(alice.clone(), "alice".to_string());
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            devices: vec![bob.clone(), alice.clone()],
            admins: vec![admin.public_key().to_hex()],
            aliases,
            signed_at: 123,
        };

        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");

        signed.verify().expect("verify signed roster");
        assert_eq!(
            signed.signer_pubkey_hex().unwrap(),
            admin.public_key().to_hex()
        );
        assert_eq!(signed.network_id().unwrap(), "mesh");
        assert_eq!(signed.roster().unwrap().network_name, "Home");
        assert_eq!(signed.content_hash().len(), 64);
        assert_eq!(signed.artifact_hash().len(), 64);
    }

    #[test]
    fn signed_roster_puts_roster_fields_in_tags() {
        let admin = Keys::generate();
        let member = Keys::generate().public_key().to_hex();
        let mut aliases = HashMap::new();
        aliases.insert(member.clone(), "phone".to_string());
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            devices: vec![member.clone()],
            admins: vec![admin.public_key().to_hex()],
            aliases,
            signed_at: 123,
        };

        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");
        let tags = signed
            .event
            .tags
            .iter()
            .map(Tag::as_slice)
            .collect::<Vec<_>>();

        assert!(signed.event.content.is_empty());
        assert!(tags.contains(&vec!["d".to_string(), "mesh".to_string()].as_slice()));
        assert!(tags.contains(&vec!["member".to_string(), member].as_slice()));
        assert!(
            tags.iter()
                .any(|tag| tag.first().is_some_and(|tag| tag == "alias"))
        );
    }

    #[test]
    fn signed_roster_rejects_tampered_content() {
        let admin = Keys::generate();
        let member = Keys::generate().public_key().to_hex();
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            devices: vec![member],
            admins: vec![admin.public_key().to_hex()],
            aliases: HashMap::new(),
            signed_at: 123,
        };
        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");
        let mut event = signed.event.clone();
        event.tags.push(roster_tag(&["name", "Office"]).unwrap());
        let signed = SignedRoster { event };

        assert!(signed.verify().is_err());
    }

    #[test]
    fn endpoint_hints_default_to_udp_transport() {
        let hint: PeerEndpointHint =
            serde_json::from_str(r#"{"addr":"192.168.50.22:51820"}"#).expect("decode hint");

        assert_eq!(hint.transport, "udp");
        assert_eq!(hint.addr, "192.168.50.22:51820");
    }

    #[test]
    fn peer_endpoint_hint_addr_accepts_lan_and_dns_udp_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22:51820")),
            Some("192.168.50.22:51820".to_string())
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("peer.example.com:51820")),
            Some("peer.example.com:51820".to_string())
        );
    }

    #[test]
    fn peer_endpoint_hint_addr_rejects_unusable_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint {
                transport: "tcp".to_string(),
                addr: "192.168.50.22:51820".to_string(),
            }),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("127.0.0.1:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("100.120.94.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("198.51.100.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("0.0.0.0:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("localhost:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("not an endpoint:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp(format!(
                "{}:51820",
                "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"
            ))),
            None
        );
    }

    #[test]
    fn large_control_frame_fragments_under_direct_limit() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            devices: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };

        let messages = encode_fips_control_messages(&frame).expect("fragment");

        assert!(messages.len() > 1);
        for message in messages {
            assert!(message.len() <= FIPS_CONTROL_DIRECT_FRAME_LIMIT);
            assert!(matches!(
                decode_fips_control_frame(&message).expect("decode"),
                Some(FipsControlFrame::Fragment { .. })
            ));
        }
    }

    #[test]
    fn fragment_buffer_decodes_fragmented_frame() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            devices: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };
        let messages = encode_fips_control_messages(&frame).expect("fragment messages");
        let mut buffer = FipsControlFragmentBuffer::default();
        let mut decoded = None;

        for message in messages {
            decoded = buffer
                .decode("npub1source", &message, 1)
                .expect("decode with fragments")
                .or(decoded);
        }

        assert_eq!(decoded, Some(frame));
    }

    #[test]
    fn fragment_buffer_keys_sources_by_bytes() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            devices: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };
        let messages = encode_fips_control_messages(&frame).expect("fragment messages");
        assert!(messages.len() > 1);
        let fragments: Vec<_> = messages
            .iter()
            .map(|message| {
                let fragment = decode_fips_control_frame(message)
                    .expect("decode fragment")
                    .expect("fragment frame");
                let FipsControlFrame::Fragment {
                    id,
                    index,
                    total,
                    data,
                } = fragment
                else {
                    panic!("expected fragment");
                };
                (id, index, total, data)
            })
            .collect();

        let mut buffer = FipsControlFragmentBuffer::default();
        let source_a = [1u8; 16];
        let source_b = [2u8; 16];

        for (offset, (id, index, total, data)) in fragments.iter().cloned().enumerate() {
            let source = if offset == 0 { source_a } else { source_b };
            assert!(
                buffer
                    .push(source, id, index, total, data, 1)
                    .expect("push mixed source fragment")
                    .is_none()
            );
        }

        let mut reassembled = None;
        for (id, index, total, data) in fragments.into_iter().skip(1) {
            reassembled = buffer
                .push(source_a, id, index, total, data, 1)
                .expect("push same source fragment")
                .or(reassembled);
        }
        let decoded = decode_fips_control_frame(&reassembled.expect("reassembled frame"))
            .expect("decode reassembled")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn fragment_buffer_limits_pending_entries_per_source() {
        let mut buffer = FipsControlFragmentBuffer::default();
        let source = [7u8; 16];
        let data = URL_SAFE_NO_PAD.encode(b"x");

        for index in 0..FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES_PER_SOURCE {
            assert!(
                buffer
                    .push(source, format!("fragment-{index}"), 0, 2, data.clone(), 1)
                    .expect("push fragment")
                    .is_none()
            );
        }
        assert_eq!(
            buffer.entries.len(),
            FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES_PER_SOURCE
        );

        assert!(
            buffer
                .push(source, "overflow".to_string(), 0, 2, data, 1)
                .expect("push overflow fragment")
                .is_none()
        );
        assert_eq!(
            buffer.entries.len(),
            FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES_PER_SOURCE
        );
    }

    #[test]
    fn fragment_buffer_limits_pending_entries_globally() {
        let mut buffer = FipsControlFragmentBuffer::default();
        let data = URL_SAFE_NO_PAD.encode(b"x");
        let mut inserted = 0usize;

        'outer: for source_index in 0u16.. {
            let source = source_index.to_be_bytes();
            for entry_index in 0..FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES_PER_SOURCE {
                buffer
                    .push(
                        source,
                        format!("fragment-{source_index}-{entry_index}"),
                        0,
                        2,
                        data.clone(),
                        1,
                    )
                    .expect("push fragment");
                inserted += 1;
                if inserted == FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES {
                    break 'outer;
                }
            }
        }
        assert_eq!(
            buffer.entries.len(),
            FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES
        );

        buffer
            .push([0xff, 0xff], "global-overflow".to_string(), 0, 2, data, 1)
            .expect("push overflow fragment");
        assert_eq!(
            buffer.entries.len(),
            FIPS_CONTROL_MAX_PENDING_FRAGMENT_ENTRIES
        );
    }

    #[test]
    fn unknown_kind_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes.extend_from_slice(br#"{"v":1,"frame":{"kind":"future_kind","x":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }

    #[test]
    fn future_version_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes
            .extend_from_slice(br#"{"v":99,"frame":{"kind":"ping","network_id":"x","sent_at":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }
}
