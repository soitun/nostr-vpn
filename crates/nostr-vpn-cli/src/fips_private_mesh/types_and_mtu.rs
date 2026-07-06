type ControlFragmentBuffer = FipsControlFragmentBuffer;
pub(crate) type ParticipantPubkeyBytes = [u8; 32];
type FipsPeerActivityMap = HashMap<ParticipantPubkeyBytes, Arc<FipsPeerActivity>>;


#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
struct BorrowedTunFd {
    fd: RawFd,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl BorrowedTunFd {
    fn new(fd: RawFd) -> Self {
        Self { fd }
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
    destination: Option<IpAddr>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
// Enough for FSP header + inner header + tag, session-datagram header,
// and outer FMP header + timestamp + tag without another hot-path allocation.
const FIPS_ENDPOINT_PACKET_HEADROOM: usize = 128;

#[cfg(any(target_os = "linux", target_os = "macos"))]
type TunPipelineBatch = Vec<TunPipelinePacket>;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct DirectTunWriteBatch {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packet_ends: Vec<usize>,
    #[cfg(feature = "paid-exit")]
    packet_sources: Vec<FipsPacketSource>,
    bytes: usize,
    mesh_generation: u64,
    data_rx_notes: FipsDataRxBatchNotes,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl DirectTunWriteBatch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            runs: Vec::with_capacity(capacity),
            packet_ends: Vec::with_capacity(capacity),
            #[cfg(feature = "paid-exit")]
            packet_sources: Vec::with_capacity(capacity),
            bytes: 0,
            mesh_generation: 0,
            data_rx_notes: FipsDataRxBatchNotes::default(),
        }
    }

    fn clear(&mut self) {
        self.runs.clear();
        self.packet_ends.clear();
        #[cfg(feature = "paid-exit")]
        self.packet_sources.clear();
        self.bytes = 0;
        self.mesh_generation = 0;
        self.data_rx_notes.clear();
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        self.packet_ends.last().copied().unwrap_or(0)
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn mesh_generation(&self) -> u64 {
        self.mesh_generation
    }

    fn set_mesh_generation(&mut self, generation: u64) {
        self.mesh_generation = generation;
    }

    fn append_data_rx_notes(&mut self, notes: &mut FipsDataRxBatchNotes) {
        self.data_rx_notes.append(notes);
    }

    fn data_rx_notes_mut(&mut self) -> &mut FipsDataRxBatchNotes {
        &mut self.data_rx_notes
    }

    fn push_run(&mut self, run: FipsEndpointDirectPacketRun, source: FipsPacketSource) {
        if run.is_empty() {
            return;
        }
        let packet_count = run.len();
        self.runs.reserve(1);
        self.packet_ends.reserve(1);
        #[cfg(feature = "paid-exit")]
        self.packet_sources.reserve(packet_count);
        self.bytes = self.bytes.saturating_add(run.packet_bytes());
        self.push_packet_end(packet_count);
        #[cfg(feature = "paid-exit")]
        self.packet_sources
            .extend(std::iter::repeat_n(source, packet_count));
        #[cfg(not(feature = "paid-exit"))]
        let _ = source;
        self.runs.push(run);
    }

    fn push_packet_end(&mut self, packet_count: usize) {
        let previous = self.len();
        self.packet_ends
            .push(previous.saturating_add(packet_count));
    }

    #[cfg(any(feature = "paid-exit", target_os = "linux"))]
    fn packet_slice(&self, index: usize) -> Option<&[u8]> {
        if index >= self.len() {
            return None;
        }
        let run_index = self.packet_ends.partition_point(|end| *end <= index);
        let previous_end = run_index
            .checked_sub(1)
            .and_then(|previous| self.packet_ends.get(previous).copied())
            .unwrap_or(0);
        self.runs
            .get(run_index)
            .and_then(|run| run.packet_slice(index - previous_end))
    }

    #[cfg(feature = "paid-exit")]
    fn packet_source(&self, index: usize) -> Option<FipsPacketSource> {
        self.packet_sources.get(index).copied()
    }

    fn run_slices(&self) -> impl Iterator<Item = &[u8]> {
        self.runs.iter().flat_map(|run| run.packet_slices())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
#[cfg(feature = "paid-exit")]
struct FipsPacketSource {
    participant_key: Option<ParticipantPubkeyBytes>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
#[cfg(not(feature = "paid-exit"))]
struct FipsPacketSource;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsPacketSource {
    fn new(participant_key: Option<&ParticipantPubkeyBytes>) -> Self {
        #[cfg(feature = "paid-exit")]
        {
            return Self {
                participant_key: participant_key.copied(),
            };
        }
        #[cfg(not(feature = "paid-exit"))]
        {
            let _ = participant_key;
            Self
        }
    }
}

#[cfg(feature = "paid-exit")]
#[derive(Default)]
struct FipsPaidRouteAccounting {
    peers: HashMap<ParticipantPubkeyBytes, FipsPaidRoutePeerAccounting>,
}

#[cfg(feature = "paid-exit")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FipsPaidRouteAccountingPeer {
    pub(crate) participant_pubkey: ParticipantPubkeyBytes,
    pub(crate) role: FipsPaidRouteAccountingRole,
}

#[cfg(feature = "paid-exit")]
impl FipsPaidRouteAccountingPeer {
    pub(crate) fn parse(
        participant_pubkey: &str,
        role: FipsPaidRouteAccountingRole,
    ) -> Option<Self> {
        participant_pubkey_bytes(participant_pubkey).map(|participant_pubkey| Self {
            participant_pubkey,
            role,
        })
    }
}

#[cfg(feature = "paid-exit")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FipsPaidRouteAccountingRole {
    LocalBuyer,
    LocalSeller,
}

#[cfg(feature = "paid-exit")]
#[derive(Default)]
struct FipsPaidRoutePeerAccounting {
    role: Option<FipsPaidRouteAccountingRole>,
    accountant: PaidRouteTrafficAccountant,
    pending: PaidRouteUsage,
}

#[cfg(feature = "paid-exit")]
impl FipsPaidRouteAccounting {
    fn replace_peers<I>(&mut self, participants: I)
    where
        I: IntoIterator<Item = FipsPaidRouteAccountingPeer>,
    {
        let mut next_peers = HashMap::new();
        for peer in participants {
            if next_peers.contains_key(&peer.participant_pubkey) {
                continue;
            }
            let mut state = self
                .peers
                .remove(&peer.participant_pubkey)
                .unwrap_or_default();
            state.role = Some(peer.role);
            next_peers.insert(peer.participant_pubkey, state);
        }
        self.peers = next_peers;
    }

    fn record_outbound(
        &mut self,
        participant: Option<&str>,
        participant_key: Option<&ParticipantPubkeyBytes>,
        packet: &[u8],
    ) {
        let Some(peer) = self.peer_mut(participant, participant_key) else {
            return;
        };
        let delta = match peer.role {
            Some(FipsPaidRouteAccountingRole::LocalBuyer) => {
                peer.accountant.record_outbound_packet(packet)
            }
            Some(FipsPaidRouteAccountingRole::LocalSeller) => {
                peer.accountant.record_inbound_packet(packet)
            }
            None => return,
        };
        peer.pending.add_assign(&delta);
    }

    fn record_inbound(
        &mut self,
        participant: Option<&str>,
        participant_key: Option<&ParticipantPubkeyBytes>,
        packet: &[u8],
    ) {
        let Some(peer) = self.peer_mut(participant, participant_key) else {
            return;
        };
        let delta = match peer.role {
            Some(FipsPaidRouteAccountingRole::LocalBuyer) => {
                peer.accountant.record_inbound_packet(packet)
            }
            Some(FipsPaidRouteAccountingRole::LocalSeller) => {
                peer.accountant.record_outbound_packet(packet)
            }
            None => return,
        };
        peer.pending.add_assign(&delta);
    }

    fn drain(&mut self, participant: &str) -> PaidRouteUsage {
        if let Some(key) = participant_pubkey_bytes(participant)
            && let Some(peer) = self.peers.get_mut(&key)
        {
            return std::mem::take(&mut peer.pending);
        }
        PaidRouteUsage::default()
    }

    fn peer_mut(
        &mut self,
        participant: Option<&str>,
        participant_key: Option<&ParticipantPubkeyBytes>,
    ) -> Option<&mut FipsPaidRoutePeerAccounting> {
        let key = participant_key
            .copied()
            .or_else(|| participant.and_then(participant_pubkey_bytes))?;
        self.peers.get_mut(&key)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsMeshRecvWorker {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsTunSendWorker {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunPipelinePacket {
    fn new(bytes: Vec<u8>) -> Self {
        let destination = packet_destination(&bytes);
        Self::from_destination(bytes, destination)
    }

    fn from_destination(mut bytes: Vec<u8>, destination: Option<IpAddr>) -> Self {
        reserve_fips_endpoint_headroom(&mut bytes);
        Self {
            bytes,
            destination,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn vec_with_fips_endpoint_headroom(len: usize) -> Vec<u8> {
    Vec::with_capacity(len.saturating_add(FIPS_ENDPOINT_PACKET_HEADROOM))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn copy_with_fips_endpoint_headroom(bytes: &[u8]) -> Vec<u8> {
    let mut owned = vec_with_fips_endpoint_headroom(bytes.len());
    owned.extend_from_slice(bytes);
    owned
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reserve_fips_endpoint_headroom(bytes: &mut Vec<u8>) {
    let needed = bytes.len().saturating_add(FIPS_ENDPOINT_PACKET_HEADROOM);
    if bytes.capacity() < needed {
        bytes.reserve(needed - bytes.capacity());
    }
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

struct FipsDataRxNote {
    participant: Option<String>,
    participant_key: Option<ParticipantPubkeyBytes>,
    bytes: usize,
}

impl FipsDataRxNote {
    fn new(
        participant: &str,
        participant_key: Option<&ParticipantPubkeyBytes>,
        bytes: usize,
    ) -> Self {
        let participant_key = participant_key.copied();
        Self {
            participant: participant_key.is_none().then(|| participant.to_string()),
            participant_key,
            bytes,
        }
    }
}

#[derive(Default)]
struct FipsDataRxBatchNotes {
    entries: Vec<FipsDataRxNote>,
}

impl FipsDataRxBatchNotes {
    fn push(&mut self, note: FipsDataRxNote) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            match (entry.participant_key, note.participant_key) {
                (Some(left), Some(right)) => left == right,
                (None, None) => entry.participant == note.participant,
                _ => false,
            }
        }) {
            entry.bytes = entry.bytes.saturating_add(note.bytes);
            return;
        }
        self.entries.push(note);
    }

    fn append(&mut self, other: &mut Self) {
        for note in other.drain() {
            self.push(note);
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn drain(&mut self) -> impl Iterator<Item = FipsDataRxNote> + '_ {
        self.entries.drain(..)
    }
}

#[derive(Debug)]
struct FipsEndpointIdentitySendRun {
    participant_fallback: Option<String>,
    participant_key: Option<ParticipantPubkeyBytes>,
    identity: PeerIdentity,
    payloads: Vec<Vec<u8>>,
    packet_count: usize,
    bytes_len: usize,
}

impl FipsEndpointIdentitySendRun {
    fn new(
        participant_fallback: Option<String>,
        participant_key: Option<ParticipantPubkeyBytes>,
        identity: PeerIdentity,
        payload: Vec<u8>,
    ) -> Self {
        let mut run = Self {
            participant_fallback,
            participant_key,
            identity,
            payloads: Vec::new(),
            packet_count: 0,
            bytes_len: 0,
        };
        run.push_payload(payload);
        run
    }

    fn push_payload(&mut self, payload: Vec<u8>) {
        let bytes_len = payload.len();
        self.payloads.push(payload);
        self.packet_count = self.packet_count.saturating_add(1);
        self.bytes_len = self.bytes_len.saturating_add(bytes_len);
    }

    fn into_send_parts(
        self,
    ) -> (
        Option<String>,
        Option<ParticipantPubkeyBytes>,
        PeerIdentity,
        Vec<Vec<u8>>,
        usize,
        usize,
    ) {
        (
            self.participant_fallback,
            self.participant_key,
            self.identity,
            self.payloads,
            self.packet_count,
            self.bytes_len,
        )
    }

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

    fn matches_endpoint(
        &self,
        endpoint_node_addr: &[u8; 16],
        participant_key: Option<ParticipantPubkeyBytes>,
        participant: &str,
    ) -> bool {
        self.identity.node_addr().as_bytes() == endpoint_node_addr
            && self.matches_participant(participant_key, participant)
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
