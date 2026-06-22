#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn prefer_nonself_tunnel_snapshot_ignores_tunnel_default_interface() {
        let tunnel_runtime = CliTunnelRuntime::new("utun100");
        let previous = crate::diagnostics::NetworkSnapshot {
            default_interface: Some("eth0".to_string()),
            primary_ipv4: Some(Ipv4Addr::new(192, 168, 64, 2)),
            primary_ipv6: None,
            gateway_ipv4: Some(Ipv4Addr::new(192, 168, 64, 1)),
            gateway_ipv6: None,
        };
        let latest = crate::diagnostics::NetworkSnapshot {
            default_interface: Some("utun100".to_string()),
            primary_ipv4: Some(Ipv4Addr::new(10, 44, 210, 253)),
            primary_ipv6: None,
            gateway_ipv4: None,
            gateway_ipv6: None,
        };

        let preferred = prefer_nonself_tunnel_snapshot(&tunnel_runtime, &previous, latest);

        assert_eq!(preferred.default_interface.as_deref(), Some("eth0"));
        assert_eq!(preferred.primary_ipv4, Some(Ipv4Addr::new(192, 168, 64, 2)));
    }

    #[cfg(feature = "embedded-fips")]
    #[test]
    fn endpoint_peer_signature_tracks_address_hint_metadata() {
        let static_config = vec![crate::fips_private_mesh::FipsEndpointPeerTransportConfig {
            npub: "peer".to_string(),
            addresses: vec![crate::fips_private_mesh::FipsPeerAddressHint {
                addr: "198.51.100.91:51830".to_string(),
                seen_at_ms: None,
                priority: 10,
            }],
            auto_reconnect: false,
            discovery_fallback_transit: true,
        }];
        let mut stamped_config = static_config.clone();
        stamped_config[0].addresses[0].seen_at_ms = Some(123_000);
        let mut reprioritized_config = static_config.clone();
        reprioritized_config[0].addresses[0].priority = 100;
        let mut reconnect_config = static_config.clone();
        reconnect_config[0].auto_reconnect = true;

        assert_ne!(
            endpoint_peer_signature(&static_config),
            endpoint_peer_signature(&stamped_config)
        );
        assert_ne!(
            endpoint_peer_signature(&static_config),
            endpoint_peer_signature(&reprioritized_config)
        );
        assert_ne!(
            endpoint_peer_signature(&static_config),
            endpoint_peer_signature(&reconnect_config)
        );
    }

    #[cfg(feature = "embedded-fips")]
    #[test]
    fn link_event_refresh_does_not_seed_previous_direct_endpoint_hints() {
        let idle = fips_link_event_refresh(false, false, false, false);
        assert_eq!(idle, FipsLinkEventRefresh::None);
        assert!(fips_link_event_should_seed_recent_peers(idle));

        for refresh in [
            fips_link_event_refresh(true, false, false, false),
            fips_link_event_refresh(false, true, false, false),
            fips_link_event_refresh(false, false, true, false),
            fips_link_event_refresh(false, false, false, true),
        ] {
            assert_eq!(refresh, FipsLinkEventRefresh::RefreshPaths);
            assert!(!fips_link_event_should_seed_recent_peers(refresh));
        }
    }

    #[cfg(feature = "embedded-fips")]
    #[test]
    fn stale_participant_path_refresh_targets_only_matching_endpoint_peers() {
        use nostr_sdk::prelude::{Keys, ToBech32};

        let stale_key = Keys::generate();
        let other_key = Keys::generate();
        let stale_npub = stale_key.public_key().to_bech32().expect("stale npub");
        let other_npub = other_key.public_key().to_bech32().expect("other npub");
        let peers = vec![
            crate::fips_private_mesh::FipsEndpointPeerTransportConfig {
                npub: stale_npub.clone(),
                addresses: Vec::new(),
                auto_reconnect: true,
                discovery_fallback_transit: false,
            },
            crate::fips_private_mesh::FipsEndpointPeerTransportConfig {
                npub: other_npub,
                addresses: Vec::new(),
                auto_reconnect: true,
                discovery_fallback_transit: false,
            },
        ];

        let selected_from_hex =
            endpoint_peers_for_participant_refresh(&peers, &[stale_key.public_key().to_hex()]);
        assert_eq!(selected_from_hex.len(), 1);
        assert_eq!(selected_from_hex[0].npub, stale_npub);

        let selected_from_npub =
            endpoint_peers_for_participant_refresh(&peers, std::slice::from_ref(&stale_npub));
        assert_eq!(selected_from_npub.len(), 1);
        assert_eq!(selected_from_npub[0].npub, stale_npub);
    }
}
