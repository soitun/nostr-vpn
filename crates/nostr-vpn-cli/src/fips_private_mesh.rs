#![allow(dead_code)]

use anyhow::{Context, Result, anyhow};
use arc_swap::ArcSwap;
use fips_endpoint::{
    Config, ConnectPolicy, FipsEndpoint, FipsEndpointError, FipsEndpointMessage,
    FipsEndpointPayload, FipsEndpointPeer, NostrDiscoveryPolicy, PeerAddress,
    PeerConfig as FipsPeerConfig, PeerIdentity, RoutingMode, TransportInstances, UdpConfig,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use fips_endpoint::{EndpointPayloadClass, EndpointPayloadLane, classify_endpoint_payload};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::{
    AppConfig, ConnectedUdpConfig, WireGuardExitConfig, derive_mesh_tunnel_ip,
    normalize_nostr_pubkey, split_peer_transport_addr,
};
use nostr_vpn_core::data_plane::MeshPeerStatus;
use nostr_vpn_core::fips_control::{
    FipsControlFragmentBuffer, FipsControlFrame, NetworkRoster, PeerCapabilities, PeerEndpointHint,
    SignedRoster, decode_fips_control_frame, encode_fips_control_frame,
    encode_fips_control_messages,
};
use nostr_vpn_core::fips_mesh::{FipsMeshPeerConfig, FipsMeshRuntime};
use nostr_vpn_core::join_requests::MeshJoinRequest;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Command as ProcessCommand;
use std::sync::Arc;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
#[cfg(target_os = "windows")]
use std::thread::{self, JoinHandle as ThreadJoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::io::Interest;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::io::unix::AsyncFd;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::sync::mpsc;

const FIPS_PEER_ONLINE_GRACE_SECS: u64 = 20;
const FIPS_PEER_MAX_FUTURE_SKEW_SECS: u64 = 2;
const FIPS_NOSTR_DISCOVERY_APP: &str = "fips-overlay-v1";
const FIPS_LAN_DISCOVERY_SCOPE_PREFIX: &str = "nostr-vpn";
const FIPS_PEER_CAPS_GRACE_SECS: u64 = 600;
const FIPS_RECONNECT_BACKOFF_BASE_SECS: u64 = 1;
const FIPS_RECONNECT_BACKOFF_MAX_SECS: u64 = 60;
const FIPS_DISCOVERY_BACKOFF_BASE_SECS: u64 = 30;
const FIPS_DISCOVERY_BACKOFF_MAX_SECS: u64 = 300;
const FIPS_DISCOVERY_FORWARD_MIN_INTERVAL_SECS: u64 = 30;
const FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING: usize = 8;
const FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS: usize = 4;
const FIPS_NOSTR_FAILURE_STREAK_THRESHOLD: u32 = 6;
const FIPS_NOSTR_EXTENDED_COOLDOWN_SECS: u64 = 60;
const FIPS_NOSTR_STARTUP_SWEEP_MAX_AGE_SECS: u64 = 300;
const FIPS_ENDPOINT_HEARTBEAT_INTERVAL_SECS: u64 = 2;
const FIPS_ENDPOINT_LINK_DEAD_TIMEOUT_SECS: u64 = 30;
const FIPS_ENDPOINT_FAST_LINK_DEAD_TIMEOUT_SECS: u64 = 5;
const FIPS_ENDPOINT_SESSION_IDLE_TIMEOUT_SECS: u64 = 0;
const FIPS_ENDPOINT_PENDING_PACKETS_PER_DEST: usize = 64;
const FIPS_ENDPOINT_REKEY_AFTER_SECS: u64 = 3600;
const FIPS_PEER_ACTIVE_PING_INTERVAL_SECS: u64 = 5;
const FIPS_PEER_LINK_PING_INTERVAL_SECS: u64 = 5;
const FIPS_PEER_DISCOVERY_PROBE_INTERVAL_SECS: u64 = 30;
const FIPS_CONTROL_RTT_MAX_ACCEPT_MS: u128 = 10_000;
const MESH_LAN_UNDERLAY_UDP_MTU: u16 = 1420;
const MESH_LAN_TUNNEL_MTU: u16 = 1290;
const MESH_MIN_UNDERLAY_UDP_MTU: u16 = 1280;
const MESH_MIN_TUNNEL_MTU: u16 = 576;
const MESH_MAX_MTU: u16 = 9000;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const FIPS_TUN_READ_BURST: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const FIPS_MESH_SEND_BURST: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const FIPS_MESH_RECV_BURST: usize = 128;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
const FIPS_MESH_EVENT_DRAIN_LIMIT: usize = 256;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const DEFAULT_FIPS_TUN_TO_MESH_QUEUE_CAP: usize = 4096;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MIN_FIPS_TUN_TO_MESH_QUEUE_CAP: usize = 1;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_FIPS_TUN_TO_MESH_QUEUE_CAP: usize = 65_536;
#[cfg(target_os = "macos")]
const DEFAULT_FIPS_TUN_BULK_COALESCE_MICROS: u64 = 250;
#[cfg(not(target_os = "macos"))]
const DEFAULT_FIPS_TUN_BULK_COALESCE_MICROS: u64 = 0;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_FIPS_TUN_BULK_COALESCE_MICROS: u64 = 5_000;
const MIN_FIPS_UDP_SEND_BUF_SIZE: usize = 64 * 1024;
const MAX_FIPS_UDP_SEND_BUF_SIZE: usize = 8 * 1024 * 1024;
#[cfg(target_os = "macos")]
const DEFAULT_FIPS_UDP_SEND_BUF_SIZE: Option<usize> = Some(512 * 1024);
#[cfg(not(target_os = "macos"))]
const DEFAULT_FIPS_UDP_SEND_BUF_SIZE: Option<usize> = None;
#[cfg(target_os = "windows")]
const WINDOWS_FIPS_TUN_READ_BURST: usize = 64;
#[cfg(target_os = "windows")]
const WINDOWS_FIPS_TUN_WRITE_BURST: usize = 128;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_tun_to_mesh_queue_cap() -> usize {
    static VALUE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_fips_tun_to_mesh_queue_cap(
            std::env::var("NVPN_FIPS_TUN_TO_MESH_QUEUE_CAP")
                .ok()
                .as_deref(),
            DEFAULT_FIPS_TUN_TO_MESH_QUEUE_CAP,
        )
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_fips_tun_to_mesh_queue_cap(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(default)
        .clamp(
            MIN_FIPS_TUN_TO_MESH_QUEUE_CAP,
            MAX_FIPS_TUN_TO_MESH_QUEUE_CAP,
        )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_tun_bulk_coalesce_delay() -> std::time::Duration {
    static VALUE: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::time::Duration::from_micros(parse_fips_tun_bulk_coalesce_micros(
            std::env::var("NVPN_FIPS_TUN_BULK_COALESCE_MICROS")
                .ok()
                .as_deref(),
            DEFAULT_FIPS_TUN_BULK_COALESCE_MICROS,
        ))
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_fips_tun_bulk_coalesce_micros(raw: Option<&str>, default: u64) -> u64 {
    raw.and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default)
        .min(MAX_FIPS_TUN_BULK_COALESCE_MICROS)
}

fn fips_udp_send_buf_size() -> Option<usize> {
    static VALUE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_fips_udp_send_buf_size(
            std::env::var("NVPN_FIPS_UDP_SEND_BUF_SIZE").ok().as_deref(),
            DEFAULT_FIPS_UDP_SEND_BUF_SIZE,
        )
    })
}

fn parse_fips_udp_send_buf_size(raw: Option<&str>, default: Option<usize>) -> Option<usize> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return default;
    };
    match raw.parse::<usize>().ok() {
        Some(0) => None,
        Some(value) => Some(value.clamp(MIN_FIPS_UDP_SEND_BUF_SIZE, MAX_FIPS_UDP_SEND_BUF_SIZE)),
        None => default,
    }
}

fn fips_lan_discovery_scope(network_id: &str) -> String {
    let digest = Sha256::digest(network_id.trim().as_bytes());
    format!(
        "{FIPS_LAN_DISCOVERY_SCOPE_PREFIX}:{}",
        hex::encode(&digest[..16])
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
use boringtun::device::{Error as TunError, tun::TunSocket};
#[cfg(target_os = "windows")]
use nostr_vpn_wintun::load_wintun;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::task::JoinHandle;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use tokio::time::{Duration, sleep};
#[cfg(target_os = "windows")]
use wintun::{Adapter, MAX_RING_CAPACITY, Session};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::fips_host_tunnel::FipsHostTunnelConfig;

pub(crate) struct FipsPrivateMeshRuntime {
    endpoint: FipsEndpoint,
    mesh: ArcSwap<FipsMeshRuntime>,
    peer_activity: ArcSwap<FipsPeerActivityMap>,
    peer_identities: ArcSwap<FipsPeerIdentityMap>,
    presence: RwLock<HashMap<String, FipsPeerPresence>>,
    link_status: RwLock<HashMap<String, FipsEndpointPeer>>,
    other_link_status: RwLock<HashMap<String, FipsEndpointPeer>>,
    peer_capabilities: RwLock<HashMap<String, PeerCapabilitiesEntry>>,
    control_fragments: Mutex<ControlFragmentBuffer>,
}

include!("fips_private_mesh/types_and_mtu.rs");
include!("fips_private_mesh/peer_status_and_events.rs");
include!("fips_private_mesh/tun_pipeline.rs");
include!("fips_private_mesh/runtime_send.rs");
include!("fips_private_mesh/runtime_receive.rs");
include!("fips_private_mesh/runtime_status.rs");
include!("fips_private_mesh/runtime_control.rs");
include!("fips_private_mesh/control_frame.rs");
include!("fips_private_mesh/endpoint_config.rs");
include!("fips_private_mesh/tunnel_config.rs");
include!("fips_private_mesh/tunnel_runtime_unix_core.rs");
include!("fips_private_mesh/tunnel_runtime_linux.rs");
include!("fips_private_mesh/unix_tun.rs");
include!("fips_private_mesh/tunnel_runtime_windows.rs");
include!("fips_private_mesh/windows_tun.rs");
include!("fips_private_mesh/tunnel_runtime_unsupported.rs");
include!("fips_private_mesh/time.rs");

#[cfg(test)]
mod tests {
    include!("fips_private_mesh/tests_core.rs");
    include!("fips_private_mesh/tests_status.rs");
    include!("fips_private_mesh/tests_runtime.rs");
    include!("fips_private_mesh/tests_config.rs");
}
