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
#[derive(Default)]
struct FipsDirectEndpointQueueState {
    batches: VecDeque<FipsDirectEndpointQueuedRuns>,
    waiting_consumer: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointQueuedRuns {
    runs: Vec<FipsEndpointDirectPacketRun>,
    packets: usize,
    source_node_addr: Option<[u8; 16]>,
    enqueued_at: Option<Instant>,
    arrival: DirectEndpointQueueArrival,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy)]
enum DirectEndpointQueueArrival {
    ConsumerBusy,
    Backlog,
    Wake,
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
    fn new(runs: Vec<FipsEndpointDirectPacketRun>) -> Self {
        Self::with_enqueued_at(runs, crate::pipeline_profile::stamp())
    }

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
            arrival: DirectEndpointQueueArrival::ConsumerBusy,
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
        let mut queued = FipsDirectEndpointQueuedRuns::new(runs);
        if queued.packets == 0 {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| FipsEndpointDirectDeliveryError::Unavailable)?;
        let wake_consumer = state.waiting_consumer;
        queued.arrival = if wake_consumer {
            DirectEndpointQueueArrival::Wake
        } else if state.batches.is_empty() {
            DirectEndpointQueueArrival::ConsumerBusy
        } else {
            DirectEndpointQueueArrival::Backlog
        };
        let queue_depth = state.batches.len().saturating_add(1);
        crate::pipeline_profile::record_direct_endpoint_sink_batch(
            queued.runs.len(),
            queued.packets,
            queue_depth,
            matches!(queued.arrival, DirectEndpointQueueArrival::Wake),
        );
        state.batches.push_back(queued);
        if wake_consumer {
            self.ready.notify_one();
        }
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
        coalesce_limited_direct_endpoint_runs(&mut queued, limit, &mut state);
        Ok(queued.runs)
    }

    fn try_recv_limited(
        &self,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        let mut queued = state
            .batches
            .pop_front()
            .ok_or(std::sync::mpsc::TryRecvError::Empty)?;
        coalesce_limited_direct_endpoint_runs(&mut queued, limit, &mut state);
        Ok(queued.runs)
    }

}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn coalesce_limited_direct_endpoint_runs(
    queued: &mut FipsDirectEndpointQueuedRuns,
    limit: usize,
    state: &mut FipsDirectEndpointQueueState,
) {
    let limit = limit.max(1);
    record_direct_endpoint_queue_residence(queued);
    limit_queued_direct_endpoint_runs_to_remaining(queued, limit, state);
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
        limit_queued_direct_endpoint_runs_to_remaining(
            &mut next,
            limit.saturating_sub(packet_count),
            state,
        );
        packet_count = packet_count.saturating_add(next.packets);
        coalesced_batches = coalesced_batches.saturating_add(1);
        queued.runs.append(&mut next.runs);
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
    state: &mut FipsDirectEndpointQueueState,
) {
    if queued.packets <= remaining {
        return;
    }
    let enqueued_at = queued.enqueued_at;
    let runs = std::mem::take(&mut queued.runs);
    let (head, tail) = split_direct_packet_runs_at_packet_limit(runs, remaining);
    queued.runs = head;
    (queued.packets, queued.source_node_addr) = direct_packet_runs_summary(&queued.runs);
    if !tail.is_empty() {
        let mut tail_queued = FipsDirectEndpointQueuedRuns::with_enqueued_at(tail, enqueued_at);
        crate::pipeline_profile::record_direct_endpoint_rx_limit_split(tail_queued.packets);
        tail_queued.arrival = DirectEndpointQueueArrival::Backlog;
        state
            .batches
            .push_front(tail_queued);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_direct_endpoint_queue_residence(queued: &FipsDirectEndpointQueuedRuns) {
    crate::pipeline_profile::record_since(
        crate::pipeline_profile::Stage::DirectEndpointQueue,
        queued.enqueued_at,
    );
    let stage = match queued.arrival {
        DirectEndpointQueueArrival::Wake => crate::pipeline_profile::Stage::DirectEndpointWake,
        DirectEndpointQueueArrival::Backlog => {
            crate::pipeline_profile::Stage::DirectEndpointBacklog
        }
        DirectEndpointQueueArrival::ConsumerBusy => {
            crate::pipeline_profile::Stage::DirectEndpointConsumerBusy
        }
    };
    crate::pipeline_profile::record_since(
        stage,
        queued.enqueued_at,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
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
    for mut run in runs {
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

        if let Some(tail_run) = run.split_off_packets(remaining) {
            if !run.is_empty() {
                head.push(run);
            }
            if !tail_run.is_empty() {
                tail.push(tail_run);
            }
        }
        remaining = 0;
    }

    (head, tail)
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
