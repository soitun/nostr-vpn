#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointDataSink {
    queue: Arc<FipsDirectEndpointQueue>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
type FipsDirectEndpointDataRx = Arc<FipsDirectEndpointQueue>;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointQueue {
    state: Mutex<FipsDirectEndpointQueueState>,
    ready: Condvar,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointRxCursor {
    queue: FipsDirectEndpointDataRx,
    pending: Option<FipsDirectEndpointQueuedRuns>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Default)]
struct FipsDirectEndpointQueueState {
    batches: VecDeque<FipsDirectEndpointQueuedRuns>,
    interrupt_pending: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointQueuedRuns {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packets: usize,
    source_node_addr: Option<[u8; 16]>,
    enqueued_at: Option<Instant>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn fips_direct_endpoint_queue_pair() -> (FipsDirectEndpointDataSink, FipsDirectEndpointDataRx) {
    let queue = Arc::new(FipsDirectEndpointQueue::new());
    (
        FipsDirectEndpointDataSink {
            queue: Arc::clone(&queue),
        },
        queue,
    )
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsDirectEndpointQueuedRuns {
    fn with_enqueued_at(
        runs: Vec<FipsEndpointDirectPacketRun>,
        enqueued_at: Option<Instant>,
    ) -> Self {
        let (packets, source_node_addr) = direct_packet_runs_summary(&runs);
        Self {
            runs,
            packets,
            source_node_addr,
            enqueued_at,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsEndpointDirectSink for FipsDirectEndpointDataSink {
    fn deliver_endpoint_packet_batch(
        &self,
        batch: FipsEndpointDirectPacketBatch,
    ) -> Result<(), FipsEndpointDirectDeliveryError> {
        self.queue.push(batch.into_packet_runs())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
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
        let queued =
            FipsDirectEndpointQueuedRuns::with_enqueued_at(runs, crate::pipeline_profile::stamp());
        if queued.packets == 0 {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| FipsEndpointDirectDeliveryError::Unavailable)?;
        let wake_consumer = state.batches.is_empty();
        let queue_depth = state.batches.len().saturating_add(1);
        crate::pipeline_profile::record_direct_endpoint_sink_batch(
            queued.runs.len(),
            queued.packets,
            queue_depth,
        );
        state.batches.push_back(queued);
        if wake_consumer {
            self.ready.notify_one();
        }
        Ok(())
    }

    fn cursor(self: &Arc<Self>) -> FipsDirectEndpointRxCursor {
        FipsDirectEndpointRxCursor {
            queue: Arc::clone(self),
            pending: None,
        }
    }

    fn interrupt(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.interrupt_pending = true;
        }
        self.ready.notify_all();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsDirectEndpointRxCursor {
    fn recv_source_batch_timeout(
        &mut self,
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::RecvTimeoutError> {
        if let Some(runs) = self.take_pending_limited(limit) {
            return Ok(runs);
        }

        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
        if state.batches.is_empty() {
            let (next_state, wait) = self
                .queue
                .ready
                .wait_timeout_while(state, timeout, |state| {
                    state.batches.is_empty() && !state.interrupt_pending
                })
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
            state = next_state;
            let interrupted = std::mem::take(&mut state.interrupt_pending);
            if interrupted && state.batches.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
            if wait.timed_out() && state.batches.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
        }
        let mut queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::RecvTimeoutError::Timeout)?;
        coalesce_limited_direct_endpoint_runs(&mut queued, limit, &mut state, &mut self.pending);
        Ok(queued.runs)
    }

    fn try_recv_limited(
        &mut self,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        if let Some(runs) = self.take_pending_limited(limit) {
            return Ok(runs);
        }

        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        let mut queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::TryRecvError::Empty)?;
        coalesce_limited_direct_endpoint_runs(&mut queued, limit, &mut state, &mut self.pending);
        Ok(queued.runs)
    }

    fn take_pending_limited(&mut self, limit: usize) -> Option<Vec<FipsEndpointDirectPacketRun>> {
        let mut queued = self.pending.take()?;
        self.pending = limit_queued_direct_endpoint_runs_to_remaining(&mut queued, limit.max(1));
        crate::pipeline_profile::record_direct_endpoint_rx_batch(
            queued.runs.len(),
            queued.packets,
            1,
        );
        Some(queued.runs)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Drop for FipsDirectEndpointRxCursor {
    fn drop(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if let Ok(mut state) = self.queue.state.lock() {
            let wake_consumer = state.batches.is_empty();
            state.batches.push_front(pending);
            if wake_consumer {
                self.queue.ready.notify_one();
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn coalesce_limited_direct_endpoint_runs(
    queued: &mut FipsDirectEndpointQueuedRuns,
    limit: usize,
    state: &mut FipsDirectEndpointQueueState,
    pending: &mut Option<FipsDirectEndpointQueuedRuns>,
) {
    let limit = limit.max(1);
    record_direct_endpoint_queue_residence(queued);
    *pending = limit_queued_direct_endpoint_runs_to_remaining(queued, limit);
    let Some(source_node_addr) = queued.source_node_addr else {
        crate::pipeline_profile::record_direct_endpoint_rx_batch(1, queued.packets, 1);
        return;
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
        *pending = limit_queued_direct_endpoint_runs_to_remaining(&mut next, limit - packet_count);
        packet_count = packet_count.saturating_add(next.packets);
        coalesced_batches = coalesced_batches.saturating_add(1);
        queued.runs.append(&mut next.runs);
        if pending.is_some() {
            break;
        }
    }
    queued.packets = packet_count;

    crate::pipeline_profile::record_direct_endpoint_rx_batch(
        queued.runs.len(),
        packet_count,
        coalesced_batches,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn limit_queued_direct_endpoint_runs_to_remaining(
    queued: &mut FipsDirectEndpointQueuedRuns,
    remaining: usize,
) -> Option<FipsDirectEndpointQueuedRuns> {
    if queued.packets <= remaining {
        return None;
    }
    let enqueued_at = queued.enqueued_at;
    let packet_count = queued.packets;
    let source_node_addr = queued.source_node_addr;
    let tail = split_queued_direct_packet_runs_at_packet_limit(&mut queued.runs, remaining);
    if let Some(source_node_addr) = source_node_addr {
        queued.packets = remaining;
        queued.source_node_addr = (remaining > 0).then_some(source_node_addr);
    } else {
        (queued.packets, queued.source_node_addr) = direct_packet_runs_summary(&queued.runs);
    }
    if !tail.is_empty() {
        let tail_queued = if let Some(source_node_addr) = source_node_addr {
            FipsDirectEndpointQueuedRuns {
                runs: tail,
                packets: packet_count.saturating_sub(remaining),
                source_node_addr: Some(source_node_addr),
                enqueued_at,
            }
        } else {
            FipsDirectEndpointQueuedRuns::with_enqueued_at(tail, enqueued_at)
        };
        crate::pipeline_profile::record_direct_endpoint_rx_limit_split(tail_queued.packets);
        return Some(tail_queued);
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_direct_endpoint_queue_residence(queued: &FipsDirectEndpointQueuedRuns) {
    crate::pipeline_profile::record_since(
        crate::pipeline_profile::Stage::DirectEndpointQueue,
        queued.enqueued_at,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn split_queued_direct_packet_runs_at_packet_limit(
    runs: &mut Vec<FipsEndpointDirectPacketRun>,
    limit: usize,
) -> Vec<FipsEndpointDirectPacketRun> {
    if limit == 0 {
        return std::mem::take(runs);
    }

    let mut consumed = 0usize;
    let mut index = 0usize;
    while index < runs.len() {
        let next = consumed.saturating_add(runs[index].len());
        if next < limit {
            consumed = next;
            index = index.saturating_add(1);
            continue;
        }
        if next == limit {
            return runs.split_off(index.saturating_add(1));
        }

        let split_at = limit - consumed;
        let mut tail = Vec::with_capacity(runs.len().saturating_sub(index));
        if let Some(tail_run) = runs[index].split_off_packets(split_at)
            && !tail_run.is_empty()
        {
            tail.push(tail_run);
        }
        tail.extend(runs.split_off(index.saturating_add(1)));
        return tail;
    }

    Vec::new()
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn direct_packet_runs_summary(runs: &[FipsEndpointDirectPacketRun]) -> (usize, Option<[u8; 16]>) {
    let Some((first, rest)) = runs.split_first() else {
        return (0, None);
    };

    let first_source = *first.source_peer().node_addr().as_bytes();
    let mut packets = first.len();
    let mut source_node_addr = Some(first_source);
    for run in rest {
        packets += run.len();
        if source_node_addr.is_some() && run.source_peer().node_addr().as_bytes() != &first_source
        {
            source_node_addr = None;
        }
    }

    (packets, source_node_addr)
}
