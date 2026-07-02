type ControlFragmentBuffer = FipsControlFragmentBuffer;
type ParticipantPubkeyBytes = [u8; 32];
type FipsPeerActivityMap = HashMap<ParticipantPubkeyBytes, Arc<FipsPeerActivity>>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataSink {
    lanes: Vec<Arc<FipsDirectEndpointDataLane>>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataRx {
    lane: Arc<FipsDirectEndpointDataLane>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataLane {
    state: Mutex<FipsDirectEndpointDataLaneState>,
    ready: Condvar,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct FipsDirectEndpointDataLaneState {
    batches: VecDeque<FipsDirectEndpointQueuedRuns>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointQueuedRuns {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packets: usize,
    enqueued_at: Option<Instant>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointQueuedRuns {
    fn new(runs: Vec<FipsEndpointDirectPacketRun>) -> Self {
        let packets = direct_packet_runs_len(&runs);
        Self {
            runs,
            packets,
            enqueued_at: crate::pipeline_profile::stamp(),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsEndpointDirectSink for FipsDirectEndpointDataSink {
    fn deliver_endpoint_packet_batch(
        &self,
        batch: FipsEndpointDirectPacketBatch,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        self.deliver_batch(batch)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointDataSink {
    fn deliver_batch(
        &self,
        batch: FipsEndpointDirectPacketBatch,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        if batch
            .packet_runs()
            .iter()
            .all(FipsEndpointDirectPacketRun::is_empty)
        {
            return Ok(());
        }
        if self.lanes.len() <= 1 {
            return self.deliver_packet_runs_to_lane(0, batch.into_packet_runs());
        }

        self.deliver_packet_runs_by_lane(batch.into_packet_runs())
    }

    fn deliver_packet_runs_by_lane(
        &self,
        runs: Vec<FipsEndpointDirectPacketRun>,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        let mut lane_runs: Vec<Vec<FipsEndpointDirectPacketRun>> =
            (0..self.lanes.len()).map(|_| Vec::new()).collect();
        for run in runs {
            if run.is_empty() {
                continue;
            }
            let source_node_addr = *run.source_node_addr().as_bytes();
            for (lane, lane_run) in run.partition_by_packet_lane(self.lanes.len(), |packet| {
                direct_endpoint_lane_key_for_packet(&source_node_addr, packet)
            }) {
                lane_runs[lane].push(lane_run);
            }
        }
        for (lane, runs) in lane_runs.into_iter().enumerate() {
            if !runs.is_empty() {
                self.deliver_packet_runs_to_lane(lane, runs)?;
            }
        }
        Ok(())
    }

    fn deliver_packet_runs_to_lane(
        &self,
        lane: usize,
        runs: Vec<FipsEndpointDirectPacketRun>,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        self.lanes[lane].push(runs)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointDataRx {
    fn new(lane: Arc<FipsDirectEndpointDataLane>) -> Self {
        Self { lane }
    }

    fn recv_source_batch_timeout(
        &self,
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::RecvTimeoutError> {
        self.lane.recv_source_batch_timeout(timeout, limit)
    }

    fn try_recv(&self) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        self.lane.try_recv()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointDataLane {
    fn new() -> Self {
        Self {
            state: Mutex::new(FipsDirectEndpointDataLaneState::default()),
            ready: Condvar::new(),
        }
    }

    fn push(
        &self,
        runs: Vec<FipsEndpointDirectPacketRun>,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        let queued = FipsDirectEndpointQueuedRuns::new(runs);
        if queued.packets == 0 {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| FipsEndpointDirectDeliveryError::Unavailable)?;
        let queue_depth = state.batches.len().saturating_add(1);
        crate::pipeline_profile::record_direct_endpoint_sink_batch(
            queued.runs.len(),
            queued.packets,
            queue_depth,
        );
        state.batches.push_back(queued);
        self.ready.notify_one();
        Ok(())
    }

    fn recv_source_batch_timeout(
        &self,
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::RecvTimeoutError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
        if state.batches.is_empty() {
            let (next_state, wait) = self
                .ready
                .wait_timeout_while(state, timeout, |state| state.batches.is_empty())
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
            state = next_state;
            if wait.timed_out() && state.batches.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
        }
        let mut queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::RecvTimeoutError::Timeout)?;
        crate::pipeline_profile::record_since(
            crate::pipeline_profile::Stage::DirectEndpointQueue,
            queued.enqueued_at,
        );
        let Some(source_node_addr) = direct_packet_runs_single_source_node_addr(&queued.runs)
        else {
            crate::pipeline_profile::record_direct_endpoint_rx_batch(1, queued.packets, 1);
            return Ok(queued.runs);
        };

        let mut packet_count = queued.packets;
        let mut coalesced_batches = 1usize;
        while packet_count < limit {
            let Some(next) = state.batches.front() else {
                break;
            };
            if direct_packet_runs_single_source_node_addr(&next.runs) != Some(source_node_addr) {
                break;
            }
            let mut next = state
                .batches
                .pop_front()
                .expect("front batch must remain present while lane lock is held");
            crate::pipeline_profile::record_since(
                crate::pipeline_profile::Stage::DirectEndpointQueue,
                next.enqueued_at,
            );
            packet_count = packet_count.saturating_add(next.packets);
            coalesced_batches = coalesced_batches.saturating_add(1);
            queued.runs.append(&mut next.runs);
        }

        crate::pipeline_profile::record_direct_endpoint_rx_batch(
            queued.runs.len(),
            packet_count,
            coalesced_batches,
        );
        Ok(queued.runs)
    }

    fn try_recv(&self) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        let queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::TryRecvError::Empty)?;
        crate::pipeline_profile::record_since(
            crate::pipeline_profile::Stage::DirectEndpointQueue,
            queued.enqueued_at,
        );
        crate::pipeline_profile::record_direct_endpoint_rx_batch(
            queued.runs.len(),
            queued.packets,
            1,
        );
        Ok(queued.runs)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_packet_runs_len(runs: &[FipsEndpointDirectPacketRun]) -> usize {
    runs.iter().map(FipsEndpointDirectPacketRun::len).sum()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_packet_runs_single_source_node_addr(
    runs: &[FipsEndpointDirectPacketRun],
) -> Option<[u8; 16]> {
    let first = *runs.first()?.source_node_addr().as_bytes();
    runs.iter()
        .all(|run| run.source_node_addr().as_bytes() == &first)
        .then_some(first)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_endpoint_lane_key_for_node_addr(source_node_addr: &[u8; 16]) -> usize {
    let mut lane = 2_166_136_261usize;
    for byte in source_node_addr {
        direct_endpoint_lane_key_mix(&mut lane, *byte);
    }
    lane
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_endpoint_lane_key_for_packet(source_node_addr: &[u8; 16], packet: &[u8]) -> usize {
    let mut key = direct_endpoint_lane_key_for_node_addr(source_node_addr);
    match packet.first().map(|byte| byte >> 4) {
        Some(4) => direct_endpoint_lane_key_mix_ipv4(&mut key, packet),
        Some(6) => direct_endpoint_lane_key_mix_ipv6(&mut key, packet),
        _ => {}
    }
    key
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_endpoint_lane_key_mix_ipv4(key: &mut usize, packet: &[u8]) {
    if packet.len() < 20 {
        return;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    if header_len < 20 || packet.len() < header_len {
        return;
    }
    let protocol = packet[9];
    direct_endpoint_lane_key_mix(key, protocol);
    for byte in &packet[12..20] {
        direct_endpoint_lane_key_mix(key, *byte);
    }
    let fragment = u16::from_be_bytes([packet[6], packet[7]]);
    let fragmented = fragment & 0x3fff != 0;
    if !fragmented && matches!(protocol, 6 | 17) && packet.len() >= header_len.saturating_add(4) {
        for byte in &packet[header_len..header_len + 4] {
            direct_endpoint_lane_key_mix(key, *byte);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_endpoint_lane_key_mix_ipv6(key: &mut usize, packet: &[u8]) {
    if packet.len() < 40 {
        return;
    }
    let next_header = packet[6];
    direct_endpoint_lane_key_mix(key, next_header);
    for byte in &packet[8..40] {
        direct_endpoint_lane_key_mix(key, *byte);
    }
    if matches!(next_header, 6 | 17) && packet.len() >= 44 {
        for byte in &packet[40..44] {
            direct_endpoint_lane_key_mix(key, *byte);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_endpoint_lane_key_mix(key: &mut usize, byte: u8) {
    *key = key.wrapping_mul(16_777_619) ^ usize::from(byte);
}

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
    destination: Option<IpAddr>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
// Enough for FSP header + inner header + tag, session-datagram header,
// and outer FMP header + timestamp + tag without another hot-path allocation.
const FIPS_ENDPOINT_PACKET_HEADROOM: usize = 128;

#[cfg(any(target_os = "linux", target_os = "macos"))]
type TunPipelineBatch = Vec<TunPipelinePacket>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TunWriteBatch {
    packets: Vec<FipsEndpointData>,
    bytes: usize,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TunWriteBatch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            packets: Vec::with_capacity(capacity),
            bytes: 0,
        }
    }

    fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }

    fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    fn len(&self) -> usize {
        self.packets.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn reserve(&mut self, additional: usize) {
        self.packets.reserve(additional);
    }

    fn push(&mut self, packet: FipsEndpointData) {
        self.bytes = self.bytes.saturating_add(packet.len());
        self.packets.push(packet);
    }

    fn packet_slice(&self, index: usize) -> Option<&[u8]> {
        self.packets.get(index).map(FipsEndpointData::as_slice)
    }

    #[cfg(test)]
    fn packet_slices_for_test(&self) -> Vec<&[u8]> {
        (0..self.len())
            .filter_map(|index| self.packet_slice(index))
            .collect()
    }

    fn drain_packets(&mut self) -> std::vec::Drain<'_, FipsEndpointData> {
        self.bytes = 0;
        self.packets.drain(..)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct DirectTunWriteBatch {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packet_ends: Vec<usize>,
    bytes: usize,
    mesh_generation: u64,
    data_rx_notes: FipsDataRxBatchNotes,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl DirectTunWriteBatch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            runs: Vec::with_capacity(capacity),
            packet_ends: Vec::with_capacity(capacity),
            bytes: 0,
            mesh_generation: 0,
            data_rx_notes: FipsDataRxBatchNotes::default(),
        }
    }

    fn clear(&mut self) {
        self.runs.clear();
        self.packet_ends.clear();
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

    fn reserve(&mut self, additional: usize) {
        self.runs.reserve(additional);
        self.packet_ends.reserve(additional);
    }

    fn push_run(&mut self, run: FipsEndpointDirectPacketRun) {
        if run.is_empty() {
            return;
        }
        self.bytes = self.bytes.saturating_add(run.packet_bytes());
        self.push_packet_end(run.len());
        self.runs.push(run);
    }

    fn push_packet_end(&mut self, packet_count: usize) {
        let previous = self.len();
        self.packet_ends
            .push(previous.saturating_add(packet_count));
    }

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

    fn run_slices(&self) -> impl Iterator<Item = &[u8]> {
        self.runs.iter().flat_map(|run| run.packet_slices())
    }

    #[cfg(test)]
    fn packet_slices_for_test(&self) -> Vec<&[u8]> {
        (0..self.len())
            .filter_map(|index| self.packet_slice(index))
            .collect()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum FipsMeshRecvWorker {
    Async(JoinHandle<()>),
    Blocking {
        stop: Arc<AtomicBool>,
        threads: Vec<std::thread::JoinHandle<()>>,
    },
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
