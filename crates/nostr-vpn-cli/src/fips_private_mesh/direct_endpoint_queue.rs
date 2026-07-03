#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataSink {
    queue: Arc<FipsDirectEndpointQueue>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointDataRx {
    queue: Arc<FipsDirectEndpointQueue>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointQueue {
    state: Mutex<FipsDirectEndpointQueueState>,
    ready: Condvar,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct FipsDirectEndpointQueueState {
    batches: VecDeque<FipsDirectEndpointQueuedRuns>,
    waiting_consumer: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct FipsDirectEndpointQueuedRuns {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packets: usize,
    source_node_addr: Option<[u8; 16]>,
    enqueued_at: Option<Instant>,
    enqueued_while_waiting: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_direct_endpoint_queue_pair() -> (FipsDirectEndpointDataSink, FipsDirectEndpointDataRx) {
    let queue = Arc::new(FipsDirectEndpointQueue::new());
    (
        FipsDirectEndpointDataSink {
            queue: Arc::clone(&queue),
        },
        FipsDirectEndpointDataRx { queue },
    )
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
            enqueued_while_waiting: false,
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
        self.queue.push(batch.into_packet_runs())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointDataRx {
    fn recv_source_batch_timeout(
        &self,
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::RecvTimeoutError> {
        self.queue.recv_source_batch_timeout(timeout, limit)
    }

    fn try_recv(&self) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        self.queue.try_recv()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsDirectEndpointQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(FipsDirectEndpointQueueState::default()),
            ready: Condvar::new(),
        }
    }

    fn push(
        &self,
        runs: Vec<FipsEndpointDirectPacketRun>,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        let mut queued = FipsDirectEndpointQueuedRuns::new(runs);
        if queued.packets == 0 {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| FipsEndpointDirectDeliveryError::Unavailable)?;
        let queue_depth = state.batches.len().saturating_add(1);
        queued.enqueued_while_waiting = state.waiting_consumer;
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
            state.waiting_consumer = true;
            let (next_state, wait) = self
                .ready
                .wait_timeout_while(state, timeout, |state| state.batches.is_empty())
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
            state = next_state;
            state.waiting_consumer = false;
            if wait.timed_out() && state.batches.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
        }
        let mut queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::RecvTimeoutError::Timeout)?;
        record_direct_endpoint_queue_residence(&queued);
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
                .expect("front batch must remain present while queue lock is held");
            record_direct_endpoint_queue_residence(&next);
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
        record_direct_endpoint_queue_residence(&queued);
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
fn limit_queued_direct_endpoint_runs_to_remaining(
    queued: &mut FipsDirectEndpointQueuedRuns,
    remaining: usize,
    state: &mut FipsDirectEndpointQueueState,
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
        let mut tail_queued = FipsDirectEndpointQueuedRuns::with_enqueued_at(tail, enqueued_at);
        tail_queued.enqueued_while_waiting = false;
        state
            .batches
            .push_front(tail_queued);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn record_direct_endpoint_queue_residence(queued: &FipsDirectEndpointQueuedRuns) {
    crate::pipeline_profile::record_since(
        crate::pipeline_profile::Stage::DirectEndpointQueue,
        queued.enqueued_at,
    );
    if queued.enqueued_while_waiting {
        crate::pipeline_profile::record_since(
            crate::pipeline_profile::Stage::DirectEndpointWake,
            queued.enqueued_at,
        );
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
