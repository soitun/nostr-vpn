use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fips_core::FipsEndpointServiceReceiver;
use fips_endpoint::{
    FipsEndpoint, NostrDiscoveryPolicy, PeerAddress, PeerConfig as FipsPeerConfig, PeerIdentity,
    TransportInstances,
};
use nostr_sdk::prelude::{Keys, PublicKey, ToBech32};
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
const DIRECT_APPROVAL_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);
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
        Self::start_with_config(config)
    }

    /// Start a one-shot delivery endpoint whose transport identity is not the
    /// long-lived VPN mesh identity. The queued approval events remain signed
    /// by the roster administrator; this identity is only the FIPS carrier.
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
        // Direct approval delivery is a single explicit WebRTC dial, not a
        // second VPN mesh. Keep relay signaling available even when the app's
        // ambient WebRTC preference is off, and remove every ambient path.
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
            unreachable!("headless approval endpoint uses one WebRTC transport");
        };
        webrtc.advertise_on_nostr = Some(false);
        webrtc.auto_connect = Some(false);
        webrtc.accept_connections = Some(false);
        webrtc.signal_relays = Some(config.nostr_relays.clone());
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
        self.send_queued_approvals_to_route(config_path, None)
    }

    /// Deliver queued approvals through an explicit route without rewriting
    /// the durable outbox. This lets a fresh browser route supersede a stale
    /// route while preserving the same signed approval and ACK contract.
    pub fn send_queued_approvals_to_route(
        &self,
        config_path: &Path,
        route_override: Option<&str>,
    ) -> Result<usize> {
        let queued = load_direct_join_approvals(config_path);
        if queued.is_empty() {
            return Err(anyhow!("direct join approval outbox is empty"));
        }
        let mut sent = 0;
        for (path, approval) in queued {
            let delivery_route = route_override.or(approval.fips_route_npub.as_deref());
            if let Some(route) = delivery_route {
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
            let delivery_pubkey = delivery_route.unwrap_or(&approval.recipient_npub);
            let recipient_npub = PublicKey::from_hex(delivery_pubkey)
                .context("invalid direct join approval delivery peer")?
                .to_bech32()
                .context("failed to encode direct join approval delivery peer")?;
            let remote = PeerIdentity::from_npub(&recipient_npub)
                .context("invalid direct join approval delivery peer")?;
            for event in &approval.events {
                let datagram = if delivery_route.is_some() {
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
            self.wait_for_approval_ack(&approval, delivery_route)?;
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        Ok(sent)
    }

    fn wait_for_approval_ack(
        &self,
        approval: &nostr_vpn_core::join_pubsub::QueuedNostrJoinApproval,
        delivery_route: Option<&str>,
    ) -> Result<()> {
        let expected_source = delivery_route.unwrap_or(&approval.recipient_npub);
        self.runtime.block_on(async {
            let deadline = tokio::time::Instant::now() + DIRECT_APPROVAL_ACK_TIMEOUT;
            let mut datagrams = Vec::with_capacity(8);
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(self.approval_ack_timeout_error(expected_source).await);
                }
                let Ok(received) = tokio::time::timeout(
                    remaining,
                    self.approval_ack_receiver
                        .recv_batch_into(&mut datagrams, 8),
                )
                .await
                else {
                    return Err(self.approval_ack_timeout_error(expected_source).await);
                };
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

    async fn approval_ack_timeout_error(&self, expected_source: &str) -> anyhow::Error {
        let peer = self.endpoint.peers().await.ok().and_then(|peers| {
            peers.into_iter().find(|peer| {
                normalize_nostr_pubkey(&peer.npub).is_ok_and(|source| source == expected_source)
            })
        });
        match peer {
            Some(peer) => anyhow!(
                "direct join approval apply acknowledgment timed out: connected={}, route={}, packets_sent={}, packets_recv={}, bytes_sent={}, bytes_recv={}",
                peer.connected,
                peer.last_outbound_route.as_deref().unwrap_or("none"),
                peer.packets_sent,
                peer.packets_recv,
                peer.bytes_sent,
                peer.bytes_recv,
            ),
            None => {
                anyhow!("direct join approval apply acknowledgment timed out: delivery peer absent")
            }
        }
    }
}

impl Drop for HeadlessDirectApprovalRuntime {
    fn drop(&mut self) {
        let _ = self.runtime.block_on(self.endpoint.shutdown());
    }
}
