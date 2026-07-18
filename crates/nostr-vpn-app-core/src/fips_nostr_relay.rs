use std::sync::Arc;

use anyhow::Result;
use fips_endpoint::{
    Config, FipsEndpoint, NostrRelayConfig, PeerAddress, PeerConfig, TransportInstances,
};
use nostr_vpn_core::config::FIPS_NOSTR_RELAY_FALLBACK_PRIORITY;
use nostr_vpn_core::fips_pubsub_relay::FipsPubsubNostrRelayAdapter;

pub(crate) fn enable_transport(config: &mut Config) {
    config.transports.nostr_relay = TransportInstances::Single(NostrRelayConfig {
        auto_connect: Some(false),
        accept_connections: Some(false),
        ..NostrRelayConfig::default()
    });
}

pub(crate) async fn start_adapter(
    endpoint: &Arc<FipsEndpoint>,
    enabled: bool,
    relays: &[String],
) -> Result<Option<FipsPubsubNostrRelayAdapter>> {
    if !enabled {
        return Ok(None);
    }
    FipsPubsubNostrRelayAdapter::start(Arc::clone(endpoint), relays)
        .await
        .map(Some)
}

pub(crate) fn upsert_peer<'a>(
    peers: &'a mut Vec<PeerConfig>,
    npub: &str,
    discovery_fallback_transit: bool,
    relay_enabled: bool,
) -> &'a mut PeerConfig {
    let index = if let Some(index) = peers.iter().position(|peer| peer.npub == npub) {
        index
    } else {
        peers.push(PeerConfig {
            npub: npub.to_string(),
            discovery_fallback_transit,
            ..PeerConfig::default()
        });
        peers.len() - 1
    };
    let peer = &mut peers[index];
    if relay_enabled
        && !peer
            .addresses
            .iter()
            .any(|address| address.transport == "nostr_relay")
    {
        peer.addresses.push(PeerAddress::with_priority(
            "nostr_relay",
            peer.npub.clone(),
            FIPS_NOSTR_RELAY_FALLBACK_PRIORITY,
        ));
    }
    peer
}
