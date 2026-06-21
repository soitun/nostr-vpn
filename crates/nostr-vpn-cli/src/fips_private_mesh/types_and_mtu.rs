type ControlFragmentBuffer = FipsControlFragmentBuffer;
type ParticipantPubkeyBytes = [u8; 32];
type FipsPeerActivityMap = HashMap<ParticipantPubkeyBytes, Arc<FipsPeerActivity>>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct BorrowedTunFd {
    fd: RawFd,
    vnet_hdr: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BorrowedTunFd {
    fn new(fd: RawFd, vnet_hdr: bool) -> Self {
        Self { fd, vnet_hdr }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AsRawFd for BorrowedTunFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TunPipelinePacket {
    bytes: Vec<u8>,
    class: EndpointPayloadClass,
    queued_at: Option<std::time::Instant>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
type TunPipelineBatch = Vec<TunPipelinePacket>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TunWriteBatch {
    priority: Vec<Vec<u8>>,
    bulk: Vec<Vec<u8>>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TunPipelineQueueTx {
    priority: mpsc::UnboundedSender<TunPipelineBatch>,
    bulk: mpsc::Sender<TunPipelineBatch>,
    bulk_queued_packets: Arc<AtomicUsize>,
    bulk_packet_capacity: usize,
    bulk_available: Arc<Notify>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TunPipelineQueueRx {
    priority: mpsc::UnboundedReceiver<TunPipelineBatch>,
    bulk: mpsc::Receiver<TunPipelineBatch>,
    bulk_queued_packets: Arc<AtomicUsize>,
    bulk_packet_capacity: usize,
    bulk_available: Arc<Notify>,
    priority_closed: bool,
    bulk_closed: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TunPipelineLane {
    Priority,
    Bulk,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum FipsMeshRecvWorker {
    Async(JoinHandle<()>),
    Blocking {
        stop: Arc<AtomicBool>,
        thread: std::thread::JoinHandle<()>,
    },
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunPipelinePacket {
    fn new(bytes: Vec<u8>) -> Self {
        let class = classify_endpoint_payload(&bytes);
        Self {
            bytes,
            class,
            queued_at: crate::pipeline_profile::stamp(),
        }
    }

    fn lane(&self) -> TunPipelineLane {
        match self.class.lane() {
            EndpointPayloadLane::Priority => TunPipelineLane::Priority,
            EndpointPayloadLane::Bulk => TunPipelineLane::Bulk,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunWriteBatch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            priority: Vec::with_capacity(capacity.min(16)),
            bulk: Vec::with_capacity(capacity),
        }
    }

    fn clear(&mut self) {
        self.priority.clear();
        self.bulk.clear();
    }

    fn is_empty(&self) -> bool {
        self.priority.is_empty() && self.bulk.is_empty()
    }

    fn push(&mut self, packet: Vec<u8>) {
        match classify_endpoint_payload(&packet).lane() {
            EndpointPayloadLane::Priority => self.priority.push(packet),
            EndpointPayloadLane::Bulk => self.bulk.push(packet),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunPipelineQueueTx {
    fn channel(capacity: usize) -> (Self, TunPipelineQueueRx) {
        let capacity = capacity.max(1);
        let (priority_tx, priority_rx) = mpsc::unbounded_channel();
        let (bulk_tx, bulk_rx) = mpsc::channel(capacity);
        let bulk_queued_packets = Arc::new(AtomicUsize::new(0));
        let bulk_available = Arc::new(Notify::new());
        (
            Self {
                priority: priority_tx,
                bulk: bulk_tx,
                bulk_queued_packets: Arc::clone(&bulk_queued_packets),
                bulk_packet_capacity: capacity,
                bulk_available: Arc::clone(&bulk_available),
            },
            TunPipelineQueueRx {
                priority: priority_rx,
                bulk: bulk_rx,
                bulk_queued_packets,
                bulk_packet_capacity: capacity,
                bulk_available,
                priority_closed: false,
                bulk_closed: false,
            },
        )
    }

    fn bulk_available_packet_slots(&self) -> usize {
        self.bulk_packet_capacity.saturating_sub(
            self.bulk_queued_packets
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    fn tun_read_backpressure_ready(&self, read_burst: usize) -> bool {
        self.tun_read_backpressure_ready_for_slots(read_burst)
    }

    fn tun_read_backpressure_ready_for_slots(&self, slots: usize) -> bool {
        self.bulk.is_closed()
            || self.bulk_available_packet_slots()
                >= self.tun_read_backpressure_min_slots(slots)
    }

    fn tun_read_backpressure_min_slots(&self, read_burst: usize) -> usize {
        read_burst.max(1).min(self.bulk_packet_capacity)
    }

    async fn wait_for_tun_read_bulk_headroom(&self, read_burst: usize) -> bool {
        self.wait_for_tun_read_bulk_slots(read_burst).await
    }

    async fn wait_for_tun_read_bulk_slots(&self, slots: usize) -> bool {
        let needed = self.tun_read_backpressure_min_slots(slots);
        loop {
            if self.bulk.is_closed() {
                return false;
            }
            if self.bulk_available_packet_slots() >= needed {
                return true;
            }

            let notified = self.bulk_available.notified();
            if self.bulk.is_closed() {
                return false;
            }
            if self.bulk_available_packet_slots() >= needed {
                return true;
            }

            let _timer = crate::pipeline_profile::Timer::start(
                crate::pipeline_profile::Stage::TunToMeshBackpressureWait,
            );
            tokio::select! {
                _ = notified => {}
                _ = self.bulk.closed() => return false,
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunPipelineQueueRx {
    fn bulk_backlog_packets(&self) -> usize {
        self.bulk_queued_packets
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn has_bulk_backlog(&self) -> bool {
        self.bulk_backlog_packets() > 0
    }

    fn bulk_backlog_capacity(&self) -> usize {
        self.bulk_packet_capacity
    }

    async fn recv(&mut self) -> Option<TunPipelineBatch> {
        loop {
            match self.priority.try_recv() {
                Ok(batch) => return Some(batch),
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => self.priority_closed = true,
            }

            if self.priority_closed && self.bulk_closed {
                return None;
            }

            tokio::select! {
                biased;
                batch = self.priority.recv(), if !self.priority_closed => {
                    match batch {
                        Some(batch) => return Some(batch),
                        None => self.priority_closed = true,
                    }
                }
                batch = self.bulk.recv(), if !self.bulk_closed => {
                    match batch {
                        Some(batch) => {
                            release_tun_bulk_packet_slots(&self.bulk_queued_packets, batch.len());
                            self.bulk_available.notify_waiters();
                            return Some(batch);
                        }
                        None => self.bulk_closed = true,
                    }
                }
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TunQueueSubmit {
    Enqueued,
    DroppedBulk,
    Closed,
}

struct FipsEndpointControlReply {
    peer: PeerIdentity,
    data: Vec<u8>,
}

struct FipsEndpointMessageOutcome {
    event: Option<FipsPrivateMeshEvent>,
    reply: Option<FipsEndpointControlReply>,
}

impl FipsEndpointMessageOutcome {
    fn event(event: FipsPrivateMeshEvent) -> Self {
        Self {
            event: Some(event),
            reply: None,
        }
    }

    fn event_with_reply(event: FipsPrivateMeshEvent, peer: PeerIdentity, data: Vec<u8>) -> Self {
        Self {
            event: Some(event),
            reply: Some(FipsEndpointControlReply { peer, data }),
        }
    }

    fn none() -> Self {
        Self {
            event: None,
            reply: None,
        }
    }
}

#[derive(Debug)]
struct FipsEndpointIdentitySendRun {
    participant_fallback: Option<String>,
    participant_key: Option<ParticipantPubkeyBytes>,
    identity: PeerIdentity,
    payloads: Vec<FipsEndpointPayload>,
    bytes_len: usize,
}

impl FipsEndpointIdentitySendRun {
    fn matches(
        &self,
        identity: PeerIdentity,
        participant_key: Option<ParticipantPubkeyBytes>,
        participant: &str,
    ) -> bool {
        self.identity == identity && self.matches_participant(participant_key, participant)
    }

    fn matches_participant(
        &self,
        participant_key: Option<ParticipantPubkeyBytes>,
        participant: &str,
    ) -> bool {
        match (self.participant_key, participant_key) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.participant_fallback.as_deref() == Some(participant),
            _ => false,
        }
    }
}

#[derive(Debug)]
enum FipsEndpointSendRun {
    Identity(FipsEndpointIdentitySendRun),
}

#[derive(Debug, Clone, Default)]
struct FipsPeerIdentityMap {
    by_participant: HashMap<ParticipantPubkeyBytes, PeerIdentity>,
    by_endpoint_node_addr: HashMap<[u8; 16], PeerIdentity>,
}

impl FipsPeerIdentityMap {
    fn identity_for_send(
        &self,
        participant_pubkey: Option<&ParticipantPubkeyBytes>,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<PeerIdentity> {
        self.by_endpoint_node_addr
            .get(endpoint_node_addr)
            .or_else(|| participant_pubkey.and_then(|pubkey| self.by_participant.get(pubkey)))
            .copied()
    }

    fn identity_for_participant(&self, participant_pubkey: &str) -> Option<PeerIdentity> {
        let participant_pubkey = participant_pubkey_bytes(participant_pubkey)?;
        self.identity_for_participant_bytes(&participant_pubkey)
    }

    fn identity_for_participant_bytes(
        &self,
        participant_pubkey: &ParticipantPubkeyBytes,
    ) -> Option<PeerIdentity> {
        self.by_participant.get(participant_pubkey).copied()
    }
}

#[derive(Debug, Clone, Default)]
struct FipsPeerPresence {
    last_seen_at: Option<u64>,
    last_control_seen_at: Option<u64>,
    last_data_seen_at: Option<u64>,
    last_ping_sent_at: Option<u64>,
    last_ping_started_at: Option<Instant>,
    rtt_ms: Option<u64>,
    tx_bytes: u64,
    rx_bytes: u64,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FipsPeerRxKind {
    Control,
    Data,
}

#[derive(Debug, Default)]
struct FipsPeerActivity {
    last_seen_at: AtomicU64,
    last_control_seen_at: AtomicU64,
    last_data_seen_at: AtomicU64,
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FipsPeerActivitySnapshot {
    last_seen_at: Option<u64>,
    last_control_seen_at: Option<u64>,
    last_data_seen_at: Option<u64>,
    tx_bytes: u64,
    rx_bytes: u64,
}

impl FipsPeerActivity {
    fn note_tx(&self, len: usize) {
        saturating_atomic_add(&self.tx_bytes, len as u64);
    }

    fn note_rx(&self, len: usize, now: u64, kind: FipsPeerRxKind) {
        self.last_seen_at.store(now, Ordering::Relaxed);
        match kind {
            FipsPeerRxKind::Control => self.last_control_seen_at.store(now, Ordering::Relaxed),
            FipsPeerRxKind::Data => self.last_data_seen_at.store(now, Ordering::Relaxed),
        }
        saturating_atomic_add(&self.rx_bytes, len as u64);
    }

    fn snapshot(&self) -> FipsPeerActivitySnapshot {
        FipsPeerActivitySnapshot {
            last_seen_at: self.last_seen_at(),
            last_control_seen_at: self.last_control_seen_at(),
            last_data_seen_at: self.last_data_seen_at(),
            tx_bytes: self.tx_bytes.load(Ordering::Relaxed),
            rx_bytes: self.rx_bytes.load(Ordering::Relaxed),
        }
    }

    fn last_seen_at(&self) -> Option<u64> {
        nonzero_timestamp(self.last_seen_at.load(Ordering::Relaxed))
    }

    fn last_control_seen_at(&self) -> Option<u64> {
        nonzero_timestamp(self.last_control_seen_at.load(Ordering::Relaxed))
    }

    fn last_data_seen_at(&self) -> Option<u64> {
        nonzero_timestamp(self.last_data_seen_at.load(Ordering::Relaxed))
    }
}

fn peer_activity_map(
    participants: &[String],
    previous: Option<&FipsPeerActivityMap>,
) -> FipsPeerActivityMap {
    participants
        .iter()
        .filter_map(|participant| {
            let participant = participant_pubkey_bytes(participant)?;
            let activity = previous
                .and_then(|previous| previous.get(&participant))
                .cloned()
                .unwrap_or_default();
            Some((participant, activity))
        })
        .collect()
}

fn peer_identity_map(peers: &[FipsMeshPeerConfig]) -> FipsPeerIdentityMap {
    let mut identities = FipsPeerIdentityMap::default();
    for peer in peers {
        let endpoint_npub = normalize_fips_endpoint_npub(&peer.endpoint_npub);
        let Ok(identity) = PeerIdentity::from_npub(&endpoint_npub) else {
            continue;
        };

        if let Some(participant) = participant_pubkey_bytes(&peer.participant_pubkey) {
            identities.by_participant.insert(participant, identity);
        }
        identities
            .by_endpoint_node_addr
            .insert(*identity.node_addr().as_bytes(), identity);
    }
    identities
}

fn endpoint_identity_for_send(
    peer_identities: &FipsPeerIdentityMap,
    participant_pubkey: Option<&ParticipantPubkeyBytes>,
    endpoint_node_addr: &[u8; 16],
) -> Option<PeerIdentity> {
    peer_identities.identity_for_send(participant_pubkey, endpoint_node_addr)
}

fn saturating_atomic_add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

fn nonzero_timestamp(value: u64) -> Option<u64> {
    (value != 0).then_some(value)
}

fn fips_timestamp_within_grace(now: u64, timestamp: u64, grace_secs: u64) -> bool {
    if timestamp > now {
        return timestamp - now <= FIPS_PEER_MAX_FUTURE_SKEW_SECS;
    }
    now - timestamp <= grace_secs
}

fn fips_elapsed_at_least(now: u64, timestamp: u64, interval_secs: u64) -> bool {
    if timestamp > now {
        return timestamp - now > FIPS_PEER_MAX_FUTURE_SKEW_SECS;
    }
    now - timestamp >= interval_secs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MeshMtu {
    underlay_udp: u16,
    tunnel: u16,
}

fn private_mesh_mtu_from_app(app: Option<&AppConfig>) -> MeshMtu {
    let env_profile_raw = std::env::var("NVPN_MESH_MTU_PROFILE").ok();
    let env_profile = env_profile_raw.as_deref().and_then(non_empty_str);
    let env_underlay = parse_mtu_env("NVPN_MESH_UNDERLAY_UDP_MTU");
    let env_tunnel = parse_mtu_env("NVPN_MESH_TUNNEL_MTU");

    resolve_private_mesh_mtu_from_sources(app, env_profile, env_underlay, env_tunnel)
}

fn resolve_private_mesh_mtu_from_sources(
    app: Option<&AppConfig>,
    env_profile: Option<&str>,
    env_underlay: Option<u16>,
    env_tunnel: Option<u16>,
) -> MeshMtu {
    let app_profile = app.and_then(|app| non_empty_str(&app.mesh_mtu_profile));
    let app_underlay =
        app.and_then(|app| (app.mesh_underlay_udp_mtu > 0).then_some(app.mesh_underlay_udp_mtu));
    let app_tunnel = app.and_then(|app| (app.mesh_tunnel_mtu > 0).then_some(app.mesh_tunnel_mtu));

    resolve_private_mesh_mtu(
        env_profile.or(app_profile),
        env_underlay.or(app_underlay),
        env_tunnel.or(app_tunnel),
    )
}

fn resolve_private_mesh_mtu(
    profile: Option<&str>,
    underlay_override: Option<u16>,
    tunnel_override: Option<u16>,
) -> MeshMtu {
    let mut mtu = match normalized_mtu_profile(profile).as_deref() {
        Some("lan") => MeshMtu {
            underlay_udp: MESH_LAN_UNDERLAY_UDP_MTU,
            tunnel: MESH_LAN_TUNNEL_MTU,
        },
        _ => MeshMtu {
            underlay_udp: nostr_vpn_core::MESH_UNDERLAY_UDP_MTU,
            tunnel: nostr_vpn_core::MESH_TUNNEL_MTU,
        },
    };

    if let Some(underlay_udp) = clamp_mtu(underlay_override, MESH_MIN_UNDERLAY_UDP_MTU) {
        mtu.underlay_udp = underlay_udp;
        if tunnel_override.is_none() {
            mtu.tunnel = tunnel_mtu_for_underlay(underlay_udp);
        }
    }
    if let Some(tunnel) = clamp_mtu(tunnel_override, MESH_MIN_TUNNEL_MTU) {
        mtu.tunnel = tunnel;
    }

    let max_tunnel = tunnel_mtu_for_underlay(mtu.underlay_udp);
    if mtu.tunnel > max_tunnel {
        mtu.tunnel = max_tunnel;
    }
    mtu
}

fn normalized_mtu_profile(profile: Option<&str>) -> Option<String> {
    let profile = profile?.trim();
    if profile.is_empty() {
        return None;
    }
    Some(profile.to_ascii_lowercase())
}

fn parse_mtu_env(name: &str) -> Option<u16> {
    std::env::var(name).ok()?.trim().parse::<u16>().ok()
}

fn fips_nostr_discovery_policy_from_env() -> NostrDiscoveryPolicy {
    std::env::var("NVPN_FIPS_NOSTR_DISCOVERY_POLICY")
        .ok()
        .as_deref()
        .and_then(parse_fips_nostr_discovery_policy)
        .unwrap_or(NostrDiscoveryPolicy::ConfiguredOnly)
}

fn fips_nostr_discovery_policy_from_app(app: &AppConfig) -> NostrDiscoveryPolicy {
    std::env::var("NVPN_FIPS_NOSTR_DISCOVERY_POLICY")
        .ok()
        .as_deref()
        .and_then(parse_fips_nostr_discovery_policy)
        .unwrap_or(if app.connect_to_non_roster_fips_peers {
            NostrDiscoveryPolicy::Open
        } else {
            NostrDiscoveryPolicy::ConfiguredOnly
        })
}

fn parse_fips_nostr_discovery_policy(value: &str) -> Option<NostrDiscoveryPolicy> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "disabled" | "off" | "false" | "0" => Some(NostrDiscoveryPolicy::Disabled),
        "configured_only" | "configuredonly" | "configured" => {
            Some(NostrDiscoveryPolicy::ConfiguredOnly)
        }
        "open" | "true" | "1" => Some(NostrDiscoveryPolicy::Open),
        _ => None,
    }
}

fn non_empty_str(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn clamp_mtu(value: Option<u16>, min: u16) -> Option<u16> {
    value.map(|mtu| mtu.clamp(min, MESH_MAX_MTU))
}

fn tunnel_mtu_for_underlay(underlay_udp_mtu: u16) -> u16 {
    let tunnel_headroom =
        nostr_vpn_core::MESH_UNDERLAY_UDP_MTU.saturating_sub(nostr_vpn_core::MESH_TUNNEL_MTU);
    underlay_udp_mtu
        .saturating_sub(tunnel_headroom)
        .max(MESH_MIN_TUNNEL_MTU)
}

fn exit_node_ipv4_mss_clamp(tunnel_mtu: u16) -> u16 {
    tunnel_mtu.saturating_sub(40).max(536)
}
