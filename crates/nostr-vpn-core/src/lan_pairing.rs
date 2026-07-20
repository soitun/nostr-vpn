use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Timestamp};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, SockRef, Socket, Type};

use crate::config::{normalize_nostr_pubkey, npub_for_pubkey_hex};

const LAN_PAIRING_ANNOUNCEMENT_VERSION: u8 = 3;
const LAN_PAIRING_ANNOUNCEMENT_KIND: u16 = 37_389;
const LAN_PAIRING_MAX_FUTURE_SECS: u64 = 5;
pub const LAN_PAIRING_DURATION: Duration = Duration::from_mins(15);
pub const LAN_PAIRING_STALE_AFTER: Duration = Duration::from_secs(16);

const LAN_PAIRING_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 73, 73);
const LAN_PAIRING_PORT: u16 = 38_911;
const LAN_PAIRING_ANNOUNCE_EVERY: Duration = Duration::from_secs(3);
const LAN_PAIRING_READ_TIMEOUT: Duration = Duration::from_millis(250);
const LAN_PAIRING_BUFFER_BYTES: usize = 8_192;
const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPairingSignal {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub network_name: String,
    pub network_id: String,
    pub join_request: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPairingAnnouncement {
    pub npub: String,
    pub node_name: String,
    pub endpoint: String,
    pub join_request: String,
}

/// Shared control surface for the worker thread.
///
/// Both `broadcast_until` and `listen_until` are unix-second deadlines. The
/// worker thread stays alive until the owner stops it, while each loop tick
/// checks the deadlines independently to decide whether to send / accept.
struct LanPairingControl {
    stop: AtomicBool,
    signer: Keys,
    announcement: RwLock<LanPairingAnnouncement>,
    broadcast_until: AtomicU64,
    listen_until: AtomicU64,
}

impl LanPairingControl {
    fn new(announcement: LanPairingAnnouncement, signer: Keys) -> Self {
        Self {
            stop: AtomicBool::new(false),
            signer,
            announcement: RwLock::new(announcement),
            broadcast_until: AtomicU64::new(0),
            listen_until: AtomicU64::new(0),
        }
    }

    fn broadcast_active(&self, now_secs: u64) -> bool {
        self.broadcast_until.load(Ordering::Relaxed) > now_secs
    }

    fn listen_active(&self, now_secs: u64) -> bool {
        self.listen_until.load(Ordering::Relaxed) > now_secs
    }

    fn any_active(&self, now_secs: u64) -> bool {
        self.broadcast_active(now_secs) || self.listen_active(now_secs)
    }
}

impl std::fmt::Debug for LanPairingControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LanPairingControl")
            .field("signer", &self.signer.public_key().to_hex())
            .field("stop", &self.stop)
            .field("announcement", &self.announcement)
            .field("broadcast_until", &self.broadcast_until)
            .field("listen_until", &self.listen_until)
            .finish()
    }
}

#[derive(Debug)]
pub struct LanPairingWorker {
    receiver: Receiver<LanPairingSignal>,
    control: Arc<LanPairingControl>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct LanPairingAnnouncementPayload {
    v: u8,
    event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LanPairingAnnouncementContent {
    #[serde(default)]
    node_name: String,
    #[serde(default)]
    endpoint: String,
    join_request: String,
}

impl LanPairingWorker {
    pub fn drain(&mut self) -> Vec<LanPairingSignal> {
        let mut signals = Vec::new();
        while let Ok(signal) = self.receiver.try_recv() {
            signals.push(signal);
        }
        signals
    }

    /// Mark the worker as broadcasting our join request until `expires_at`.
    pub fn set_broadcast_until(&self, expires_at: SystemTime) {
        self.control
            .broadcast_until
            .store(unix_seconds(expires_at), Ordering::Relaxed);
    }

    /// Mark the worker as listening for nearby join requests until `expires_at`.
    pub fn set_listen_until(&self, expires_at: SystemTime) {
        self.control
            .listen_until
            .store(unix_seconds(expires_at), Ordering::Relaxed);
    }

    pub fn clear_broadcast(&self) {
        self.control.broadcast_until.store(0, Ordering::Relaxed);
    }

    pub fn clear_listen(&self) {
        self.control.listen_until.store(0, Ordering::Relaxed);
    }

    pub fn update_announcement(&self, announcement: LanPairingAnnouncement) {
        if let Ok(mut current) = self.control.announcement.write() {
            *current = announcement;
        }
    }

    pub fn stop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LanPairingWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn_lan_pairing_worker(
    announcement: LanPairingAnnouncement,
    signer: Keys,
) -> Result<LanPairingWorker> {
    validate_lan_pairing_signer(&announcement, &signer)?;
    let socket = bind_lan_pairing_socket()?;
    let interfaces = lan_pairing_interfaces();
    join_multicast_on_interfaces(&socket, &interfaces);
    let send_plan = LanPairingSendPlan::production(interfaces);

    let (sender, receiver) = mpsc::channel();
    let control = Arc::new(LanPairingControl::new(announcement, signer));
    let own_npub = to_npub_for_filter(&control);
    let thread_control = Arc::clone(&control);
    let handle = thread::spawn(move || {
        run_lan_pairing_loop(&socket, &thread_control, &send_plan, &own_npub, &sender);
    });

    Ok(LanPairingWorker {
        receiver,
        control,
        handle: Some(handle),
    })
}

fn to_npub_for_filter(control: &LanPairingControl) -> String {
    control
        .announcement
        .read()
        .map(|guard| guard.npub.clone())
        .unwrap_or_default()
}

fn bind_lan_pairing_socket() -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create LAN pairing UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to configure LAN pairing socket reuse")?;
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin")
    ))]
    socket
        .set_reuse_port(true)
        .context("failed to configure LAN pairing port reuse")?;
    socket
        .set_broadcast(true)
        .context("failed to enable broadcast on LAN pairing socket")?;
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LAN_PAIRING_PORT).into())
        .context("failed to bind LAN pairing UDP socket")?;

    let socket: UdpSocket = socket.into();
    // Default-interface join — preserved as a baseline so single-interface hosts
    // and the loopback test still work without depending on netdev.
    let _ = socket.join_multicast_v4(&LAN_PAIRING_ADDR, &Ipv4Addr::UNSPECIFIED);
    socket
        .set_read_timeout(Some(LAN_PAIRING_READ_TIMEOUT))
        .context("failed to configure LAN pairing socket timeout")?;
    socket
        .set_multicast_loop_v4(true)
        .context("failed to configure LAN pairing multicast loopback")?;
    socket
        .set_multicast_ttl_v4(1)
        .context("failed to configure LAN pairing multicast TTL")?;
    Ok(socket)
}

#[derive(Debug, Clone)]
struct LanInterface {
    addr: Ipv4Addr,
    broadcast: Option<Ipv4Addr>,
}

/// Enumerate non-loopback IPv4 interfaces with link addresses.
///
/// On Windows, the routing-table-derived `INADDR_ANY` multicast join often
/// picks a virtual adapter (Hyper-V vEthernet, WSL, Tailscale, etc.) instead
/// of the physical Wi-Fi/Ethernet, which is why a Mac on the same LAN can see
/// the Windows announcements (the Mac's join is correct) but not vice versa.
/// Joining on every real interface fixes that asymmetry.
fn lan_pairing_interfaces() -> Vec<LanInterface> {
    let mut out = Vec::new();
    for iface in netdev::get_interfaces() {
        if iface.is_loopback() {
            continue;
        }
        for net in &iface.ipv4 {
            let addr = net.addr();
            if addr.is_loopback() || addr.is_unspecified() || addr.is_link_local() {
                continue;
            }
            let broadcast = directed_broadcast(addr, net.prefix_len());
            out.push(LanInterface { addr, broadcast });
        }
    }
    out
}

fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if prefix_len == 0 || prefix_len >= 32 {
        return None;
    }
    let host_bits = 32 - u32::from(prefix_len);
    let mask = u32::MAX.checked_shl(host_bits).unwrap_or(0);
    let bcast = u32::from(addr) | !mask;
    Some(Ipv4Addr::from(bcast))
}

fn join_multicast_on_interfaces(socket: &UdpSocket, interfaces: &[LanInterface]) {
    for iface in interfaces {
        // Duplicate joins (already covered by the INADDR_ANY join) return
        // EADDRINUSE on some platforms — harmless, swallow.
        let _ = socket.join_multicast_v4(&LAN_PAIRING_ADDR, &iface.addr);
    }
}

#[derive(Debug, Clone)]
struct LanPairingSendPlan {
    multicast_target: Option<SocketAddrV4>,
    interfaces: Vec<LanInterface>,
    unicast_targets: Vec<SocketAddrV4>,
    global_broadcast: bool,
}

impl LanPairingSendPlan {
    fn production(interfaces: Vec<LanInterface>) -> Self {
        Self {
            multicast_target: Some(SocketAddrV4::new(LAN_PAIRING_ADDR, LAN_PAIRING_PORT)),
            interfaces,
            unicast_targets: Vec::new(),
            global_broadcast: true,
        }
    }
}
fn decode_lan_pairing_payload(payload: &[u8], own_npub: &str) -> Result<Option<LanPairingSignal>> {
    decode_lan_pairing_payload_at(payload, own_npub, unix_timestamp())
}

fn decode_lan_pairing_payload_at(
    payload: &[u8],
    own_npub: &str,
    now: u64,
) -> Result<Option<LanPairingSignal>> {
    let signed = match serde_json::from_slice::<LanPairingAnnouncementPayload>(payload) {
        Ok(signed) => signed,
        Err(_) => return Ok(None),
    };
    if signed.v != LAN_PAIRING_ANNOUNCEMENT_VERSION
        || signed.event.kind.as_u16() != LAN_PAIRING_ANNOUNCEMENT_KIND
        || signed.event.verify().is_err()
    {
        return Ok(None);
    }
    let timestamp = signed.event.created_at.as_secs();
    if timestamp > now.saturating_add(LAN_PAIRING_MAX_FUTURE_SECS)
        || now.saturating_sub(timestamp) > LAN_PAIRING_STALE_AFTER.as_secs()
    {
        return Ok(None);
    }
    let announcement =
        match serde_json::from_str::<LanPairingAnnouncementContent>(&signed.event.content) {
            Ok(announcement) => announcement,
            Err(_) => return Ok(None),
        };

    let sender_npub = npub_for_pubkey_hex(&signed.event.pubkey.to_hex());
    if sender_npub == own_npub.trim() {
        return Ok(None);
    }

    let join_request = announcement.join_request.trim();
    if !is_join_request_link(join_request) {
        return Ok(None);
    }

    Ok(Some(LanPairingSignal {
        npub: sender_npub.clone(),
        node_name: announcement.node_name.trim().to_string(),
        endpoint: announcement.endpoint.trim().to_string(),
        network_name: "Join request".to_string(),
        network_id: sender_npub,
        join_request: join_request.to_string(),
    }))
}

fn is_join_request_link(value: &str) -> bool {
    value
        .get(..JOIN_REQUEST_LINK_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(JOIN_REQUEST_LINK_PREFIX))
}

fn run_lan_pairing_loop(
    socket: &UdpSocket,
    control: &Arc<LanPairingControl>,
    send_plan: &LanPairingSendPlan,
    own_npub: &str,
    sender: &mpsc::Sender<LanPairingSignal>,
) {
    let mut next_announcement = SystemTime::UNIX_EPOCH;
    let mut buffer = [0_u8; LAN_PAIRING_BUFFER_BYTES];

    while !control.stop.load(Ordering::Relaxed) {
        let now = SystemTime::now();
        let now_secs = unix_seconds(now);
        if !control.any_active(now_secs) {
            thread::sleep(LAN_PAIRING_READ_TIMEOUT);
            continue;
        }

        if control.broadcast_active(now_secs) && now >= next_announcement {
            let snapshot = control.announcement.read().ok().map(|guard| guard.clone());
            if let Some(announcement) = snapshot {
                let _ = send_lan_pairing_announcement(
                    socket,
                    send_plan,
                    &announcement,
                    &control.signer,
                );
            }
            next_announcement = now
                .checked_add(LAN_PAIRING_ANNOUNCE_EVERY)
                .unwrap_or_else(SystemTime::now);
        }

        match socket.recv_from(&mut buffer) {
            Ok((len, _)) => {
                if !control.listen_active(unix_seconds(SystemTime::now())) {
                    continue;
                }
                if let Ok(Some(signal)) = decode_lan_pairing_payload(&buffer[..len], own_npub) {
                    let _ = sender.send(signal);
                }
            }
            Err(error)
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::TimedOut => {
            }
            Err(_) => break,
        }
    }
}

fn send_lan_pairing_announcement(
    socket: &UdpSocket,
    send_plan: &LanPairingSendPlan,
    announcement: &LanPairingAnnouncement,
    signer: &Keys,
) -> Result<()> {
    let encoded = encode_signed_lan_pairing_announcement(announcement, signer, unix_timestamp())?;

    if let Some(multicast_target) = send_plan.multicast_target {
        let sock_ref = SockRef::from(socket);

        // Always send via the OS-default interface — covers single-NIC hosts
        // and platforms where the per-interface fan-out below adds nothing.
        let _ = sock_ref.set_multicast_if_v4(&Ipv4Addr::UNSPECIFIED);
        let _ = socket.send_to(&encoded, multicast_target);

        // Fan out to every real interface so multi-homed hosts (Windows with
        // Hyper-V/WSL/Tailscale, Macs on Wi-Fi + Ethernet, etc.) actually reach
        // peers on every L2 segment they're connected to. Multicast is tried
        // first; a directed broadcast is the fallback when the LAN suppresses
        // multicast.
        for iface in &send_plan.interfaces {
            if sock_ref.set_multicast_if_v4(&iface.addr).is_ok() {
                let _ = socket.send_to(&encoded, multicast_target);
            }
            if let Some(broadcast) = iface.broadcast {
                let target = SocketAddrV4::new(broadcast, LAN_PAIRING_PORT);
                let _ = socket.send_to(&encoded, target);
            }
        }

        if send_plan.global_broadcast {
            // Global limited broadcast — last-resort fallback for hosts whose
            // interface enumeration picked up nothing useful (locked-down VMs,
            // captive portals).
            let _ = socket.send_to(
                &encoded,
                SocketAddrV4::new(Ipv4Addr::BROADCAST, LAN_PAIRING_PORT),
            );
        }
    }

    for target in &send_plan.unicast_targets {
        let _ = socket.send_to(&encoded, target);
    }

    Ok(())
}

fn encode_signed_lan_pairing_announcement(
    announcement: &LanPairingAnnouncement,
    signer: &Keys,
    timestamp: u64,
) -> Result<Vec<u8>> {
    validate_lan_pairing_signer(announcement, signer)?;
    let content = serde_json::to_string(&LanPairingAnnouncementContent {
        node_name: announcement.node_name.clone(),
        endpoint: announcement.endpoint.clone(),
        join_request: announcement.join_request.clone(),
    })
    .context("failed to encode LAN announcement content")?;
    let event = EventBuilder::new(Kind::Custom(LAN_PAIRING_ANNOUNCEMENT_KIND), content)
        .custom_created_at(Timestamp::from(timestamp))
        .sign_with_keys(signer)
        .map_err(|error| anyhow::anyhow!("failed to sign LAN announcement: {error}"))?;
    serde_json::to_vec(&LanPairingAnnouncementPayload {
        v: LAN_PAIRING_ANNOUNCEMENT_VERSION,
        event,
    })
    .context("failed to encode signed LAN announcement")
}

fn validate_lan_pairing_signer(announcement: &LanPairingAnnouncement, signer: &Keys) -> Result<()> {
    let announced = normalize_nostr_pubkey(&announcement.npub)?;
    anyhow::ensure!(
        announced == signer.public_key().to_hex(),
        "LAN pairing announcement identity does not match signer"
    );
    Ok(())
}

fn unix_timestamp() -> u64 {
    unix_seconds(SystemTime::now())
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::ToBech32;
    use serde_json::json;
    use std::time::Instant;

    use super::*;

    fn bind_loopback_socket() -> UdpSocket {
        let socket =
            UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind loopback UDP");
        socket
            .set_read_timeout(Some(LAN_PAIRING_READ_TIMEOUT))
            .expect("set loopback read timeout");
        socket
    }

    fn local_addr_v4(socket: &UdpSocket) -> SocketAddrV4 {
        match socket.local_addr().expect("local addr") {
            std::net::SocketAddr::V4(addr) => addr,
            std::net::SocketAddr::V6(_) => unreachable!("loopback helper binds IPv4"),
        }
    }

    fn spawn_loopback_lan_pairing_worker(
        announcement: LanPairingAnnouncement,
        signer: Keys,
        socket: UdpSocket,
        unicast_targets: Vec<SocketAddrV4>,
    ) -> LanPairingWorker {
        let send_plan = LanPairingSendPlan {
            multicast_target: None,
            interfaces: Vec::new(),
            unicast_targets,
            global_broadcast: false,
        };
        let (sender, receiver) = mpsc::channel();
        let control = Arc::new(LanPairingControl::new(announcement, signer));
        let own_npub = to_npub_for_filter(&control);
        let thread_control = Arc::clone(&control);
        let handle = thread::spawn(move || {
            run_lan_pairing_loop(&socket, &thread_control, &send_plan, &own_npub, &sender);
        });

        LanPairingWorker {
            receiver,
            control,
            handle: Some(handle),
        }
    }

    #[test]
    fn decodes_lan_announcement_with_join_request() {
        let requester = nostr_sdk::Keys::generate();
        let requester_npub = requester.public_key().to_bech32().expect("requester npub");
        let own_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("own npub");
        let join_request = "nvpn://join-request/payload";
        let payload = encode_signed_lan_pairing_announcement(
            &LanPairingAnnouncement {
                npub: requester_npub,
                node_name: "Pixel Phone".to_string(),
                endpoint: String::new(),
                join_request: join_request.to_string(),
            },
            &requester,
            unix_timestamp(),
        )
        .expect("signed payload");

        let signal = decode_lan_pairing_payload(&payload, &own_npub)
            .expect("decode")
            .expect("peer");

        assert_eq!(signal.node_name, "Pixel Phone");
        assert_eq!(signal.network_name, "Join request");
        assert_eq!(signal.join_request, join_request);
    }

    #[test]
    fn unsigned_lan_announcement_is_rejected() {
        let admin_npub = nostr_sdk::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("npub");
        let payload = json!({
            "v": 2,
            "npub": admin_npub,
            "nodeName": "Spoofed peer",
            "endpoint": "192.0.2.10:51820",
            "joinRequest": "nvpn://join-request/spoofed",
            "timestamp": unix_timestamp()
        })
        .to_string();

        assert!(
            decode_lan_pairing_payload(payload.as_bytes(), "npub1other")
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn stale_signed_lan_announcement_is_rejected() {
        let keys = nostr_sdk::Keys::generate();
        let npub = keys.public_key().to_bech32().expect("npub");
        let announcement = LanPairingAnnouncement {
            npub: npub.clone(),
            node_name: "Alice".to_string(),
            endpoint: "192.0.2.10:51820".to_string(),
            join_request: "nvpn://join-request/alice".to_string(),
        };
        let now = unix_timestamp();
        let stale = now.saturating_sub(LAN_PAIRING_STALE_AFTER.as_secs() + 1);
        let payload = encode_signed_lan_pairing_announcement(&announcement, &keys, stale)
            .expect("signed payload");

        assert!(
            decode_lan_pairing_payload_at(&payload, "npub1other", now)
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn tampered_signed_lan_announcement_is_rejected() {
        let keys = nostr_sdk::Keys::generate();
        let npub = keys.public_key().to_bech32().expect("npub");
        let announcement = LanPairingAnnouncement {
            npub: npub.clone(),
            node_name: "Alice".to_string(),
            endpoint: "192.0.2.10:51820".to_string(),
            join_request: "nvpn://join-request/alice".to_string(),
        };
        let payload =
            encode_signed_lan_pairing_announcement(&announcement, &keys, unix_timestamp())
                .expect("signed payload");
        let mut tampered = serde_json::from_slice::<serde_json::Value>(&payload).expect("JSON");
        tampered["event"]["content"] = json!({
            "nodeName": "Mallory",
            "endpoint": "192.0.2.99:51820",
            "joinRequest": announcement.join_request,
        })
        .to_string()
        .into();
        let tampered = serde_json::to_vec(&tampered).expect("tampered payload");

        assert!(
            decode_lan_pairing_payload(&tampered, "npub1other")
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn signed_lan_announcement_rejects_non_join_request_content() {
        let signer = nostr_sdk::Keys::generate();
        let signer_npub = signer.public_key().to_bech32().expect("npub");
        let announcement = LanPairingAnnouncement {
            npub: signer_npub,
            node_name: "Mallory".to_string(),
            endpoint: "192.0.2.10:51820".to_string(),
            join_request: "https://obsolete.example".to_string(),
        };
        let now = unix_timestamp();
        let payload = encode_signed_lan_pairing_announcement(&announcement, &signer, now)
            .expect("signed payload");

        assert!(
            decode_lan_pairing_payload_at(&payload, "npub1other", now)
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn lan_pairing_workers_exchange_join_requests_over_loopback_transport() {
        let alice_keys = nostr_sdk::Keys::generate();
        let alice_npub = alice_keys.public_key().to_bech32().expect("alice npub");
        let bob_keys = nostr_sdk::Keys::generate();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let alice_request = "nvpn://join-request/alice".to_string();
        let bob_request = "nvpn://join-request/bob".to_string();
        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(7))
            .expect("expiry");

        let alice_socket = bind_loopback_socket();
        let bob_socket = bind_loopback_socket();
        let alice_addr = local_addr_v4(&alice_socket);
        let bob_addr = local_addr_v4(&bob_socket);

        let mut alice = spawn_loopback_lan_pairing_worker(
            LanPairingAnnouncement {
                npub: alice_npub.clone(),
                node_name: "Alice".to_string(),
                endpoint: "192.0.2.10:51820".to_string(),
                join_request: alice_request,
            },
            alice_keys,
            alice_socket,
            vec![bob_addr],
        );
        alice.set_broadcast_until(expires_at);
        alice.set_listen_until(expires_at);
        let mut bob = spawn_loopback_lan_pairing_worker(
            LanPairingAnnouncement {
                npub: bob_npub.clone(),
                node_name: "Bob".to_string(),
                endpoint: "192.0.2.11:51820".to_string(),
                join_request: bob_request,
            },
            bob_keys,
            bob_socket,
            vec![alice_addr],
        );
        bob.set_broadcast_until(expires_at);
        bob.set_listen_until(expires_at);

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut alice_saw_bob = false;
        let mut bob_saw_alice = false;
        while Instant::now() < deadline && !(alice_saw_bob && bob_saw_alice) {
            alice_saw_bob |= alice.drain().into_iter().any(|signal| {
                signal.npub == bob_npub
                    && signal.node_name == "Bob"
                    && signal.network_name == "Join request"
            });
            bob_saw_alice |= bob.drain().into_iter().any(|signal| {
                signal.npub == alice_npub
                    && signal.node_name == "Alice"
                    && signal.network_name == "Join request"
            });
            thread::sleep(Duration::from_millis(100));
        }

        alice.stop();
        bob.stop();

        assert!(
            alice_saw_bob,
            "alice did not receive bob's LAN join request"
        );
        assert!(
            bob_saw_alice,
            "bob did not receive alice's LAN join request"
        );
    }

    #[test]
    fn listen_only_worker_receives_without_advertising() {
        let alice_keys = nostr_sdk::Keys::generate();
        let alice_npub = alice_keys.public_key().to_bech32().expect("alice npub");
        let bob_keys = nostr_sdk::Keys::generate();
        let bob_npub = bob_keys.public_key().to_bech32().expect("bob npub");
        let alice_request = "nvpn://join-request/alice".to_string();
        let bob_request = "nvpn://join-request/bob".to_string();
        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(7))
            .expect("expiry");

        // Alice broadcasts only — she should never surface bob's request.
        let alice_socket = bind_loopback_socket();
        let bob_socket = bind_loopback_socket();
        let bob_addr = local_addr_v4(&bob_socket);

        let mut alice = spawn_loopback_lan_pairing_worker(
            LanPairingAnnouncement {
                npub: alice_npub.clone(),
                node_name: "Alice".to_string(),
                endpoint: "192.0.2.10:51820".to_string(),
                join_request: alice_request,
            },
            alice_keys,
            alice_socket,
            vec![bob_addr],
        );
        alice.set_broadcast_until(expires_at);

        // Bob listens only — he should still see alice.
        let mut bob = spawn_loopback_lan_pairing_worker(
            LanPairingAnnouncement {
                npub: bob_npub.clone(),
                node_name: "Bob".to_string(),
                endpoint: "192.0.2.11:51820".to_string(),
                join_request: bob_request,
            },
            bob_keys,
            bob_socket,
            Vec::new(),
        );
        bob.set_listen_until(expires_at);

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut bob_saw_alice = false;
        let mut alice_saw_bob = false;
        while Instant::now() < deadline && !bob_saw_alice {
            bob_saw_alice |= bob
                .drain()
                .into_iter()
                .any(|signal| signal.npub == alice_npub && signal.network_name == "Join request");
            alice_saw_bob |= alice
                .drain()
                .into_iter()
                .any(|signal| signal.npub == bob_npub && signal.network_name == "Join request");
            thread::sleep(Duration::from_millis(100));
        }

        alice.stop();
        bob.stop();

        assert!(
            bob_saw_alice,
            "listen-only worker did not receive broadcast"
        );
        assert!(
            !alice_saw_bob,
            "broadcast-only worker should not surface peers (listen disabled)"
        );
    }

    #[test]
    fn directed_broadcast_for_common_prefixes() {
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 1, 17), 24),
            Some(Ipv4Addr::new(192, 168, 1, 255))
        );
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 8),
            Some(Ipv4Addr::new(10, 255, 255, 255))
        );
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 32), None);
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 0), None);
    }
}
