use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_core::FipsEndpointServiceReceiver;
use fips_endpoint::{FipsEndpoint, PeerAddress, PeerConfig as FipsPeerConfig, PeerIdentity};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::join_pubsub::{
    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT, NostrJoinFipsPubsubDatagram,
    approval_applied_ack_matches_queued, delivered_approval_event_datagram,
    load_direct_join_approvals, parse_approval_applied_ack_datagram,
    routed_approval_event_datagram,
};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use crate::mobile_tunnel::{MobileTunnelConfig, fips_endpoint_config};

const HEADLESS_FIPS_WORKER_STACK_SIZE: usize = 4 * 1024 * 1024;
const DIRECT_APPROVAL_ROUTE_TIMEOUT: Duration = Duration::from_secs(5);
const DIRECT_APPROVAL_ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// Unprivileged FIPS endpoint used by production-like integration tests.
///
/// Desktop production uses the long-running nVPN daemon. This runtime shares
/// its endpoint configuration and direct approval wire format, but deliberately
/// omits the operating-system tunnel so tests never need administrator access.
pub struct HeadlessDirectApprovalRuntime {
    runtime: Runtime,
    endpoint: Arc<FipsEndpoint>,
    approval_ack_receiver: FipsEndpointServiceReceiver,
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
        let approval_ack_receiver = runtime
            .block_on(endpoint.register_service_receiver(NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT))?;
        Ok(Self {
            runtime,
            endpoint,
            approval_ack_receiver,
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
                let route_address = PeerAddress::new("webrtc", format!("02{route}"));
                if let Some(peer) = peers.iter_mut().find(|peer| peer.npub == route_npub) {
                    if !peer
                        .addresses
                        .iter()
                        .any(|address| address == &route_address)
                    {
                        peer.addresses.push(route_address);
                    }
                } else {
                    peers.push(FipsPeerConfig {
                        npub: route_npub.clone(),
                        addresses: vec![route_address],
                        discovery_fallback_transit: true,
                        ..FipsPeerConfig::default()
                    });
                }
                self.runtime.block_on(self.endpoint.update_peers(peers))?;
                self.runtime.block_on(async {
                    let deadline = tokio::time::Instant::now() + DIRECT_APPROVAL_ROUTE_TIMEOUT;
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
                            break Err(anyhow!("direct join approval FIPS route did not connect"));
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })?;
            }
            let delivery_pubkey = approval
                .fips_route_npub
                .as_deref()
                .unwrap_or(&approval.recipient_npub);
            let recipient_npub = PublicKey::from_hex(delivery_pubkey)
                .context("invalid direct join approval delivery peer")?
                .to_bech32()
                .context("failed to encode direct join approval delivery peer")?;
            let remote = PeerIdentity::from_npub(&recipient_npub)
                .context("invalid direct join approval delivery peer")?;
            for event in &approval.events {
                let datagram = if approval.fips_route_npub.is_some() {
                    routed_approval_event_datagram(
                        &approval.recipient_npub,
                        &approval.request_pubkey,
                        event,
                    )?
                } else {
                    delivered_approval_event_datagram(&approval.request_pubkey, event)?
                };
                self.runtime.block_on(self.endpoint.send_datagram(
                    remote,
                    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    datagram.payload,
                ))?;
                sent += 1;
            }
            self.wait_for_approval_ack(&approval)?;
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(sent)
    }

    fn wait_for_approval_ack(
        &self,
        approval: &nostr_vpn_core::join_pubsub::QueuedNostrJoinApproval,
    ) -> Result<()> {
        let expected_source = approval
            .fips_route_npub
            .as_deref()
            .unwrap_or(&approval.recipient_npub);
        self.runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + DIRECT_APPROVAL_ACK_TIMEOUT;
            let mut datagrams = Vec::with_capacity(8);
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(anyhow!(
                        "direct join approval apply acknowledgment timed out"
                    ));
                }
                let received = tokio::time::timeout(
                    remaining,
                    self.approval_ack_receiver
                        .recv_batch_into(&mut datagrams, 8),
                )
                .await
                .map_err(|_| anyhow!("direct join approval apply acknowledgment timed out"))?;
                let Some(_) = received else {
                    return Err(anyhow!(
                        "direct join approval acknowledgment service closed"
                    ));
                };
                for datagram in &datagrams {
                    let Ok(source_peer) = normalize_nostr_pubkey(&datagram.source_peer.npub())
                    else {
                        continue;
                    };
                    if source_peer != expected_source {
                        continue;
                    }
                    let inbound = NostrJoinFipsPubsubDatagram {
                        source_port: datagram.source_port,
                        destination_port: datagram.destination_port,
                        payload: datagram.data.as_ref().to_vec(),
                    };
                    let Ok(ack) = parse_approval_applied_ack_datagram(&inbound) else {
                        continue;
                    };
                    if approval_applied_ack_matches_queued(&ack, approval) {
                        return Ok(());
                    }
                }
            }
        })
    }
}

impl Drop for HeadlessDirectApprovalRuntime {
    fn drop(&mut self) {
        let _ = self.runtime.block_on(self.endpoint.shutdown());
    }
}
