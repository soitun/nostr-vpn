use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{
    FipsEndpoint, NostrDiscoveryPolicy, PeerAddress, PeerConfig as FipsPeerConfig, PeerIdentity,
    TransportInstances,
};
use nostr_sdk::prelude::{Keys, PublicKey, ToBech32};
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::fips_control::signed_roster_control_frame;
use nostr_vpn_core::fips_control_tcp::FipsControlTcpRuntime;
use nostr_vpn_core::join_delivery::load_join_rosters;
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use crate::mobile_tunnel::{MobileTunnelConfig, fips_endpoint_config};

const HEADLESS_FIPS_WORKER_STACK_SIZE: usize = 4 * 1024 * 1024;
const JOIN_ROSTER_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);

/// Unprivileged FIPS endpoint used by production-like integration tests.
///
/// It sends the same signed roster control record as the daemon, over the same
/// FIPS-TCP service, without creating an operating-system tunnel.
pub struct HeadlessJoinRosterRuntime {
    runtime: Runtime,
    endpoint: Arc<FipsEndpoint>,
    control: Option<FipsControlTcpRuntime>,
    base_peers: Vec<FipsPeerConfig>,
}

impl HeadlessJoinRosterRuntime {
    pub fn start(config_path: &Path) -> Result<Self> {
        let app = AppConfig::load(config_path)?;
        let config = MobileTunnelConfig::from_app_with_config_path(&app, config_path)?;
        Self::start_with_config(config)
    }

    /// Use a one-shot carrier identity while retaining the admin signature on
    /// the roster itself.
    pub fn start_ephemeral(config_path: &Path) -> Result<Self> {
        let app = AppConfig::load(config_path)?;
        let mut config = MobileTunnelConfig::from_app_with_config_path(&app, config_path)?;
        config.identity_nsec = Keys::generate()
            .secret_key()
            .to_bech32()
            .context("failed to encode ephemeral FIPS delivery identity")?;
        Self::start_with_config(config)
    }

    fn start_with_config(config: MobileTunnelConfig) -> Result<Self> {
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let mut endpoint_config = fips_endpoint_config(&scope, &config);
        endpoint_config.peers.clear();
        endpoint_config.node.discovery.nostr.enabled = true;
        endpoint_config.node.discovery.nostr.advertise = false;
        endpoint_config.node.discovery.nostr.policy = NostrDiscoveryPolicy::ConfiguredOnly;
        endpoint_config
            .node
            .discovery
            .nostr
            .open_discovery_max_pending = 0;
        endpoint_config.node.discovery.nostr.share_local_candidates = false;
        endpoint_config.node.discovery.lan.enabled = false;
        endpoint_config.node.discovery.local.enabled = false;
        #[allow(clippy::default_trait_access)]
        {
            endpoint_config.transports.webrtc = TransportInstances::Single(Default::default());
        }
        let TransportInstances::Single(webrtc) = &mut endpoint_config.transports.webrtc else {
            unreachable!("headless roster endpoint uses one WebRTC transport");
        };
        webrtc.advertise_on_nostr = Some(false);
        webrtc.auto_connect = Some(false);
        webrtc.accept_connections = Some(false);
        webrtc.stun_servers = Some(config.stun_servers.clone());
        let base_peers = endpoint_config.peers.clone();
        let runtime = RuntimeBuilder::new_multi_thread()
            .enable_all()
            .thread_name("nvpn-headless-fips")
            .thread_stack_size(HEADLESS_FIPS_WORKER_STACK_SIZE)
            .build()
            .context("failed to start headless FIPS runtime")?;
        let endpoint = runtime.block_on(async {
            Box::pin(
                FipsEndpoint::builder()
                    .config(endpoint_config)
                    .identity_nsec(config.identity_nsec)
                    .discovery_scope(scope)
                    .without_system_tun()
                    .bind(),
            )
            .await
        })?;
        let endpoint = Arc::new(endpoint);
        let control = runtime.block_on(FipsControlTcpRuntime::start(Arc::clone(&endpoint)))?;
        Ok(Self {
            runtime,
            endpoint,
            control: Some(control),
            base_peers,
        })
    }

    pub fn send_queued_join_rosters(&self, config_path: &Path) -> Result<usize> {
        self.send_queued_join_rosters_to_route(config_path, None)
    }

    pub fn send_queued_join_rosters_to_route(
        &self,
        config_path: &Path,
        route_override: Option<&str>,
    ) -> Result<usize> {
        let queued = load_join_rosters(config_path);
        if queued.is_empty() {
            return Err(anyhow!("join roster outbox is empty"));
        }
        let control = self
            .control
            .as_ref()
            .ok_or_else(|| anyhow!("FIPS-TCP state-control runtime stopped"))?;
        let mut sent = 0;
        for (path, queued) in queued {
            let delivery_route = route_override.or(queued.fips_route_npub.as_deref());
            if let Some(route) = delivery_route {
                self.ensure_route(route)?;
            }
            let delivery_pubkey = delivery_route.unwrap_or(&queued.recipient_npub);
            let recipient_npub = PublicKey::from_hex(delivery_pubkey)
                .context("invalid join roster delivery peer")?
                .to_bech32()
                .context("failed to encode join roster delivery peer")?;
            let remote = PeerIdentity::from_npub(&recipient_npub)
                .context("invalid join roster delivery peer")?;
            let frame = signed_roster_control_frame(queued.signed_roster);
            self.runtime.block_on(control.send(remote, &frame))?;
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            sent += 1;
        }
        Ok(sent)
    }

    fn ensure_route(&self, route: &str) -> Result<()> {
        let route_npub = PublicKey::from_hex(route)
            .context("invalid join roster FIPS route")?
            .to_bech32()
            .context("failed to encode join roster FIPS route")?;
        let mut peers = self.base_peers.clone();
        let peer = crate::fips_nostr_relay::upsert_peer(&mut peers, &route_npub, true, false);
        add_headless_roster_webrtc_address(peer, route);
        self.runtime.block_on(self.endpoint.update_peers(peers))?;
        self.runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + JOIN_ROSTER_ROUTE_TIMEOUT;
            loop {
                if self
                    .endpoint
                    .peers()
                    .await?
                    .iter()
                    .any(|peer| peer.npub == route_npub && peer.connected)
                {
                    break Ok::<(), anyhow::Error>(());
                }
                if tokio::time::Instant::now() >= deadline {
                    break Err(anyhow!("join roster FIPS route did not connect"));
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
    }
}

fn add_headless_roster_webrtc_address(peer: &mut FipsPeerConfig, route_pubkey: &str) {
    let route_address = PeerAddress::new("webrtc", format!("02{route_pubkey}"));
    if !peer.addresses.contains(&route_address) {
        peer.addresses.push(route_address);
    }
}

impl Drop for HeadlessJoinRosterRuntime {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            self.runtime.block_on(control.stop());
        }
        let _ = self.runtime.block_on(self.endpoint.shutdown());
    }
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::ToBech32;

    use super::*;

    #[test]
    fn headless_join_roster_uses_webrtc_without_relay_transport() {
        let keys = Keys::generate();
        let route_pubkey = keys.public_key().to_hex();
        let route_npub = keys.public_key().to_bech32().expect("npub");
        let mut peers = Vec::new();
        let peer = crate::fips_nostr_relay::upsert_peer(&mut peers, &route_npub, true, false);
        add_headless_roster_webrtc_address(peer, &route_pubkey);

        assert!(peer.addresses.iter().any(|address| {
            address.transport == "webrtc" && address.addr == format!("02{route_pubkey}")
        }));
        assert!(
            peer.addresses
                .iter()
                .all(|address| address.transport != "nostr_relay")
        );
    }
}
