#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataSink {
    lanes: Vec<Arc<FipsDirectEndpointDataLane>>,
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
    source_node_addr: Option<[u8; 16]>,
    enqueued_at: Option<Instant>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointQueuedRuns {
    fn new(runs: Vec<FipsEndpointDirectPacketRun>) -> Self {
        Self::with_enqueued_at(runs, crate::pipeline_profile::stamp())
    }

    fn with_enqueued_at(
        runs: Vec<FipsEndpointDirectPacketRun>,
        enqueued_at: Option<Instant>,
    ) -> Self {
        let packets = direct_packet_runs_len(&runs);
        let source_node_addr = direct_packet_runs_single_source_node_addr(&runs);
        Self {
            runs,
            packets,
            source_node_addr,
            enqueued_at,
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
            push_direct_packet_run_by_lane(run, self.lanes.len(), &mut lane_runs);
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
        let Some(source_node_addr) = queued.source_node_addr else {
            crate::pipeline_profile::record_direct_endpoint_rx_batch(1, queued.packets, 1);
            return Ok(queued.runs);
        };

        let mut packet_count = queued.packets;
        let mut coalesced_batches = 1usize;
        while packet_count < limit {
            let Some(next) = state.batches.front() else {
                break;
            };
            if next.source_node_addr != Some(source_node_addr) {
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
            limit_queued_direct_endpoint_runs_to_remaining(
                &mut next,
                limit.saturating_sub(packet_count),
                &mut state,
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
fn push_direct_packet_run_by_lane(
    run: FipsEndpointDirectPacketRun,
    lane_count: usize,
    lane_runs: &mut [Vec<FipsEndpointDirectPacketRun>],
) {
    if lane_count == 0 || run.is_empty() {
        return;
    }

    let source_node_addr = *run.source_node_addr().as_bytes();
    for (lane, lane_run) in run.partition_by_packet_lane(lane_count, |packet| {
        direct_endpoint_lane_key_for_packet(&source_node_addr, packet)
    }) {
        if !lane_run.is_empty() {
            lane_runs[lane].push(lane_run);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn direct_packet_runs_len(runs: &[FipsEndpointDirectPacketRun]) -> usize {
    runs.iter().map(FipsEndpointDirectPacketRun::len).sum()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn limit_queued_direct_endpoint_runs_to_remaining(
    queued: &mut FipsDirectEndpointQueuedRuns,
    remaining: usize,
    state: &mut FipsDirectEndpointDataLaneState,
) {
    if queued.packets <= remaining {
        return;
    }
    let enqueued_at = queued.enqueued_at;
    let runs = std::mem::take(&mut queued.runs);
    let (head, tail) = split_direct_packet_runs_at_packet_limit(runs, remaining);
    queued.runs = head;
    queued.packets = direct_packet_runs_len(&queued.runs);
    queued.source_node_addr = direct_packet_runs_single_source_node_addr(&queued.runs);
    if !tail.is_empty() {
        state
            .batches
            .push_front(FipsDirectEndpointQueuedRuns::with_enqueued_at(
                tail,
                enqueued_at,
            ));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn split_direct_packet_runs_at_packet_limit(
    runs: Vec<FipsEndpointDirectPacketRun>,
    limit: usize,
) -> (
    Vec<FipsEndpointDirectPacketRun>,
    Vec<FipsEndpointDirectPacketRun>,
) {
    if limit == 0 {
        return (Vec::new(), runs);
    }

    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut remaining = limit;
    for run in runs {
        if remaining == 0 {
            tail.push(run);
            continue;
        }

        let run_len = run.len();
        if run_len <= remaining {
            remaining -= run_len;
            head.push(run);
            continue;
        }

        let mut head_run = run.clone();
        head_run.retain_packets(|index, _| index < remaining);
        if !head_run.is_empty() {
            head.push(head_run);
        }

        let mut tail_run = run;
        tail_run.retain_packets(|index, _| index >= remaining);
        if !tail_run.is_empty() {
            tail.push(tail_run);
        }
        remaining = 0;
    }

    (head, tail)
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
