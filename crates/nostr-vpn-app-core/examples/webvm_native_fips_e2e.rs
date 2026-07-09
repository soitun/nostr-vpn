use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use fips_core::config::{NostrDiscoveryPolicy, TransportInstances};
use fips_core::{Config, Identity, IdentityConfig, WebRtcConfig};
use fips_endpoint::FipsEndpoint;
use nostr_sdk::prelude::{Client, Event, Keys};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalSidecarRequest, NostrIdentityId, NostrVpnJoinApprovalContextRequest,
    build_device_approval_sidecar, build_nostr_vpn_join_approval_context_event,
    parse_nostr_identity_device_approval_request,
};
use serde_json::json;
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
const DEFAULT_DISCOVERY_APP: &str = "fips-overlay-v1";

#[derive(Debug)]
struct Args {
    relay: String,
    join_request: String,
    mesh_network_id: String,
    network_name: Option<String>,
    timeout_ms: u64,
    discovery_app: String,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut relay = None;
        let mut join_request = None;
        let mut mesh_network_id = Some("8d4f34f5425bc50e".to_string());
        let mut network_name = Some("Home".to_string());
        let mut timeout_ms = 60_000;
        let mut discovery_app = DEFAULT_DISCOVERY_APP.to_string();

        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                "--relay" => relay = Some(require_value(&mut args, "--relay")?),
                "--join-request" => {
                    join_request = Some(require_value(&mut args, "--join-request")?);
                }
                "--mesh-network-id" => {
                    mesh_network_id = Some(require_value(&mut args, "--mesh-network-id")?);
                }
                "--network-name" => {
                    let value = require_value(&mut args, "--network-name")?;
                    network_name = (!value.trim().is_empty()).then_some(value);
                }
                "--timeout-ms" => {
                    timeout_ms = require_value(&mut args, "--timeout-ms")?
                        .parse()
                        .context("invalid --timeout-ms")?;
                }
                "--discovery-app" => {
                    discovery_app = require_value(&mut args, "--discovery-app")?;
                }
                unknown => bail!("unknown argument: {unknown}"),
            }
        }

        let relay = relay.context("--relay is required")?;
        let join_request = join_request.context("--join-request is required")?;
        let mesh_network_id = mesh_network_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .context("--mesh-network-id must not be empty")?;
        let discovery_app = discovery_app.trim().to_string();
        if discovery_app.is_empty() {
            bail!("--discovery-app must not be empty");
        }

        Ok(Self {
            relay,
            join_request,
            mesh_network_id,
            network_name,
            timeout_ms,
            discovery_app,
        })
    }
}

fn require_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .filter(|value| !value.starts_with("--"))
        .with_context(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "Usage: cargo run -p nostr-vpn-app-core --example webvm_native_fips_e2e -- \\
         --relay ws://127.0.0.1:1234 \\
         --join-request nvpn://join-request/... \\
         [--mesh-network-id 8d4f34f5425bc50e] [--network-name Home] [--timeout-ms 60000]"
    );
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn print_json(value: serde_json::Value) -> Result<()> {
    println!("{value}");
    std::io::stdout().flush().context("failed to flush stdout")
}

async fn print_peer_snapshot(endpoint: &FipsEndpoint, label: &str) -> Result<()> {
    let peers = endpoint
        .peers()
        .await
        .context("failed to snapshot native FIPS peers")?;
    print_json(json!({
        "type": "peer-snapshot",
        "label": label,
        "peers": peers.into_iter().map(|peer| json!({
            "npub": peer.npub,
            "connected": peer.connected,
            "transportType": peer.transport_type,
            "transportAddr": peer.transport_addr,
            "packetsSent": peer.packets_sent,
            "packetsRecv": peer.packets_recv,
            "bytesSent": peer.bytes_sent,
            "bytesRecv": peer.bytes_recv,
            "lastOutboundRoute": peer.last_outbound_route,
        })).collect::<Vec<_>>(),
    }))
}

#[derive(Debug)]
struct Ipv4UdpPacket {
    source_ip: Ipv4Addr,
    destination_ip: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: Vec<u8>,
}

fn parse_ipv4_udp(packet: &[u8]) -> Option<Ipv4UdpPacket> {
    if packet.len() < 28 || packet[0] >> 4 != 4 {
        return None;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    if header_len < 20 || packet.len() < header_len + 8 || packet[9] != 17 {
        return None;
    }
    let total_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if total_len < header_len + 8 || total_len > packet.len() {
        return None;
    }
    let udp = &packet[header_len..total_len];
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_len < 8 || udp_len > udp.len() {
        return None;
    }
    Some(Ipv4UdpPacket {
        source_ip: Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
        destination_ip: Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]),
        source_port: u16::from_be_bytes([udp[0], udp[1]]),
        destination_port: u16::from_be_bytes([udp[2], udp[3]]),
        payload: udp[8..udp_len].to_vec(),
    })
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = header.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    if let Some(byte) = chunks.remainder().first() {
        sum += u32::from(*byte) << 8;
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    !(sum as u16)
}

fn build_ipv4_udp_packet(
    source_ip: Ipv4Addr,
    destination_ip: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let udp_len = 8usize
        .checked_add(payload.len())
        .context("UDP payload length overflow")?;
    let total_len = 20usize
        .checked_add(udp_len)
        .context("IPv4 packet length overflow")?;
    let total_len_u16 = u16::try_from(total_len).context("IPv4 response is too large")?;
    let udp_len_u16 = u16::try_from(udp_len).context("UDP response is too large")?;
    let mut packet = vec![0u8; total_len];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&total_len_u16.to_be_bytes());
    packet[4..6].copy_from_slice(&0x4e56u16.to_be_bytes());
    packet[6] = 0x40;
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&source_ip.octets());
    packet[16..20].copy_from_slice(&destination_ip.octets());
    let checksum = ipv4_checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());

    let udp_offset = 20;
    packet[udp_offset..udp_offset + 2].copy_from_slice(&source_port.to_be_bytes());
    packet[udp_offset + 2..udp_offset + 4].copy_from_slice(&destination_port.to_be_bytes());
    packet[udp_offset + 4..udp_offset + 6].copy_from_slice(&udp_len_u16.to_be_bytes());
    packet[udp_offset + 6..udp_offset + 8].copy_from_slice(&0u16.to_be_bytes());
    packet[udp_offset + 8..].copy_from_slice(payload);
    Ok(packet)
}

async fn try_udp_exit(packet: &[u8]) -> Result<Option<(Vec<u8>, SocketAddrV4, usize)>> {
    let Some(parsed) = parse_ipv4_udp(packet) else {
        return Ok(None);
    };
    let target = SocketAddrV4::new(parsed.destination_ip, parsed.destination_port);
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .await
        .context("failed to bind UDP exit socket")?;
    socket
        .send_to(&parsed.payload, SocketAddr::V4(target))
        .await
        .with_context(|| format!("failed to send UDP exit packet to {target}"))?;
    let mut response = vec![0u8; 2048];
    let (len, source) = timeout(Duration::from_secs(10), socket.recv_from(&mut response))
        .await
        .with_context(|| format!("timed out waiting for UDP exit response from {target}"))?
        .context("failed to receive UDP exit response")?;
    if source != SocketAddr::V4(target) {
        bail!("UDP exit response came from unexpected source {source}; expected {target}");
    }
    response.truncate(len);
    let response_packet = build_ipv4_udp_packet(
        parsed.destination_ip,
        parsed.source_ip,
        parsed.destination_port,
        parsed.source_port,
        &response,
    )?;
    Ok(Some((response_packet, target, len)))
}

async fn publish_events(keys: Keys, relay: &str, events: &[Event]) -> Result<()> {
    let relays = vec![relay.to_string()];
    let client = Client::new(keys);
    client
        .add_relay(relay)
        .await
        .with_context(|| format!("failed to add relay {relay}"))?;
    client.connect().await;
    for event in events {
        let output = client
            .send_event_to(relays.clone(), event)
            .await
            .context("failed to publish native approval event")?;
        if output.success.is_empty() {
            client.disconnect().await;
            bail!("native approval event was not accepted by the relay");
        }
    }
    client.disconnect().await;
    Ok(())
}

async fn start_native_endpoint(args: &Args, secret_hex: &str) -> Result<FipsEndpoint> {
    let mut config = Config::new();
    config.node.identity = IdentityConfig {
        nsec: Some(secret_hex.to_string()),
        persistent: false,
    };
    config.node.discovery.nostr.enabled = true;
    config.node.discovery.nostr.advertise = true;
    config.node.discovery.nostr.advert_relays = vec![args.relay.clone()];
    config.node.discovery.nostr.dm_relays = vec![args.relay.clone()];
    config.node.discovery.nostr.stun_servers = Vec::new();
    config.node.discovery.nostr.app = args.discovery_app.clone();
    config.node.discovery.nostr.policy = NostrDiscoveryPolicy::Open;
    config.transports.webrtc = TransportInstances::Single(WebRtcConfig {
        advertise_on_nostr: Some(true),
        auto_connect: Some(false),
        accept_connections: Some(true),
        connect_timeout_ms: Some(15_000),
        ice_gather_timeout_ms: Some(1_500),
        signal_relays: Some(vec![args.relay.clone()]),
        stun_servers: Some(Vec::new()),
        ..WebRtcConfig::default()
    });

    FipsEndpoint::builder()
        .config(config)
        .discovery_scope(args.discovery_app.clone())
        .without_system_tun()
        .bind()
        .await
        .context("failed to start native FIPS WebRTC endpoint")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse()?;
    let request = parse_nostr_identity_device_approval_request(
        args.join_request.trim(),
        &[JOIN_REQUEST_LINK_PREFIX],
    )
    .context("failed to parse join request")?
    .ok_or_else(|| anyhow!("join request is not a full Nostr VPN join request"))?;

    let admin_keys = Keys::generate();
    let admin_pubkey_hex = admin_keys.public_key().to_hex();
    if let Some(admin_app_key_pubkey) = &request.admin_app_key_pubkey
        && admin_app_key_pubkey.trim().to_lowercase() != admin_pubkey_hex
    {
        bail!("join request is addressed to a different admin app key");
    }
    if request.request_pubkey.trim().is_empty() {
        bail!("join request is missing request pubkey");
    }
    if request.device_app_key_pubkey.trim().is_empty() {
        bail!("join request is missing joiner app pubkey");
    }
    if request.request_secret.trim().is_empty() {
        bail!("join request is missing request secret");
    }
    print_json(json!({
        "type": "join-request-parsed",
        "requestPubkey": request.request_pubkey.clone(),
        "deviceAppKeyPubkey": request.device_app_key_pubkey.clone(),
        "hasRequestSecret": true,
    }))?;

    let approved_at = unix_timestamp();
    let profile_id = request
        .profile_id
        .clone()
        .unwrap_or_else(NostrIdentityId::new_v4);
    let sidecar = build_device_approval_sidecar(
        &admin_keys,
        NostrIdentityDeviceApprovalSidecarRequest {
            profile_id,
            network_name: args.network_name.clone(),
            request_pubkey: request.request_pubkey.clone(),
            device_app_key_pubkey: request.device_app_key_pubkey.clone(),
            request_secret: request.request_secret.clone(),
            parents: Vec::new(),
            actor_seq: None,
            approved_at,
        },
    )
    .context("failed to build native approval receipt")?;
    let context_event = build_nostr_vpn_join_approval_context_event(
        &admin_keys,
        NostrVpnJoinApprovalContextRequest {
            profile_id,
            request_pubkey: request.request_pubkey.clone(),
            device_app_key_pubkey: request.device_app_key_pubkey.clone(),
            request_secret: request.request_secret.clone(),
            mesh_network_id: args.mesh_network_id.clone(),
            network_name: args.network_name.clone(),
            roster_op_id: Some(sidecar.roster_op_event.id.to_hex()),
            approved_at,
        },
    )
    .context("failed to build Nostr VPN approval context")?;

    publish_events(
        admin_keys.clone(),
        &args.relay,
        &[
            sidecar.roster_op_event.clone(),
            sidecar.receipt_event.clone(),
            context_event,
        ],
    )
    .await?;
    print_json(json!({
        "type": "approval-published",
        "adminPubkeyHex": admin_pubkey_hex,
        "rosterOpId": sidecar.roster_op_event.id.to_hex(),
        "meshNetworkId": args.mesh_network_id,
    }))?;

    let secret_hex = admin_keys.secret_key().to_secret_hex();
    let identity = Identity::from_secret_str(&secret_hex)?;
    let endpoint_pubkey_hex = bytes_to_hex(&identity.pubkey_full().serialize());
    let endpoint = start_native_endpoint(&args, &secret_hex).await?;
    print_json(json!({
        "type": "ready",
        "adminPubkeyHex": admin_pubkey_hex,
        "endpointPubkeyHex": endpoint_pubkey_hex,
        "npub": endpoint.npub(),
        "meshNetworkId": args.mesh_network_id,
        "relay": args.relay,
    }))?;

    let mut messages = Vec::with_capacity(1);
    let received = timeout(
        Duration::from_millis(args.timeout_ms),
        endpoint.recv_batch_into(&mut messages, 1),
    )
    .await
    .context("timed out waiting for endpoint data from WebVM")?
    .ok_or_else(|| anyhow!("native endpoint closed before receiving endpoint data"))?;
    if received == 0 {
        bail!("native endpoint receive returned an empty endpoint-data batch");
    }
    let message = messages
        .pop()
        .ok_or_else(|| anyhow!("native endpoint receive returned no endpoint-data message"))?;
    let source_peer = message.source_peer;
    let source_peer_npub = source_peer.npub();
    let payload = message.data.into_vec();
    print_json(json!({
        "type": "endpoint-data",
        "sourcePeerNpub": source_peer_npub,
        "payloadHex": bytes_to_hex(&payload),
        "bytes": payload.len(),
    }))?;
    print_peer_snapshot(&endpoint, "after-endpoint-data").await?;
    if let Some((response_packet, target, response_bytes)) = try_udp_exit(&payload).await? {
        print_json(json!({
            "type": "exit-udp-response",
            "sourcePeerNpub": source_peer_npub,
            "target": target.to_string(),
            "responseBytes": response_bytes,
            "payloadHex": bytes_to_hex(&response_packet),
        }))?;
        for _ in 0..16 {
            endpoint
                .send_batch_to_peer(source_peer, vec![response_packet.clone()])
                .await
                .context("failed to send UDP exit response over FIPS endpoint data")?;
            sleep(Duration::from_millis(500)).await;
        }
        print_peer_snapshot(&endpoint, "after-exit-response-send").await?;
    }
    Ok(())
}
