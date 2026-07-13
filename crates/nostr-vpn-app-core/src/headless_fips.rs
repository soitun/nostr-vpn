use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use fips_endpoint::{FipsEndpoint, PeerConfig as FipsPeerConfig, PeerIdentity};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_pubsub::{
    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT, delivered_approval_event_datagram,
    load_direct_join_approvals,
};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use crate::mobile_tunnel::{MobileTunnelConfig, fips_endpoint_config};

/// Unprivileged FIPS endpoint used by production-like integration tests.
///
/// Desktop production uses the long-running nVPN daemon. This runtime shares
/// its endpoint configuration and direct approval wire format, but deliberately
/// omits the operating-system tunnel so tests never need administrator access.
pub struct HeadlessDirectApprovalRuntime {
    runtime: Runtime,
    endpoint: Arc<FipsEndpoint>,
    base_peers: Vec<FipsPeerConfig>,
}

impl HeadlessDirectApprovalRuntime {
    pub fn start(config_path: &Path) -> Result<Self> {
        let app = AppConfig::load(config_path)?;
        let config = MobileTunnelConfig::from_app_with_config_path(&app, config_path)?;
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let endpoint_config = fips_endpoint_config(&scope, &config);
        let base_peers = endpoint_config.peers.clone();
        let runtime = RuntimeBuilder::new_multi_thread()
            .enable_all()
            .thread_name("nvpn-headless-fips")
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
        Ok(Self {
            runtime,
            endpoint: Arc::new(endpoint),
            base_peers,
        })
    }

    pub fn send_queued_approvals(&self, config_path: &Path) -> Result<usize> {
        let queued = load_direct_join_approvals(config_path);
        if queued.is_empty() {
            return Err(anyhow!("direct join approval outbox is empty"));
        }
        let mut sent = 0;
        for (path, approval) in queued {
            if let Some(route) = approval.fips_route_npub.as_deref() {
                let route_npub = PublicKey::from_hex(route)
                    .context("invalid direct join approval FIPS return route")?
                    .to_bech32()
                    .context("failed to encode direct join approval FIPS return route")?;
                let mut peers = self.base_peers.clone();
                if !peers.iter().any(|peer| peer.npub == route_npub) {
                    peers.push(FipsPeerConfig {
                        npub: route_npub,
                        discovery_fallback_transit: true,
                        ..FipsPeerConfig::default()
                    });
                }
                self.runtime.block_on(self.endpoint.update_peers(peers))?;
            }
            let recipient_npub = PublicKey::from_hex(&approval.recipient_npub)
                .context("invalid direct join approval recipient")?
                .to_bech32()
                .context("failed to encode direct join approval recipient")?;
            let remote = PeerIdentity::from_npub(&recipient_npub)
                .context("invalid direct join approval recipient")?;
            for event in &approval.events {
                let datagram = delivered_approval_event_datagram(&approval.request_pubkey, event)?;
                self.runtime.block_on(self.endpoint.send_datagram(
                    remote,
                    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    datagram.payload,
                ))?;
                sent += 1;
            }
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(sent)
    }
}

impl Drop for HeadlessDirectApprovalRuntime {
    fn drop(&mut self) {
        let _ = self.runtime.block_on(self.endpoint.shutdown());
    }
}
