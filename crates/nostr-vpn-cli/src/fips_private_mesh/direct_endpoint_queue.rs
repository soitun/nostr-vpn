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
    runs: VecDeque<FipsDirectEndpointQueuedRun>,
    packets: usize,
    interrupt_pending: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
struct FipsDirectEndpointQueuedRun {
    run: FipsEndpointDirectPacketRun,
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
        debug_assert!(
            !runs.is_empty()
                && runs.iter().all(|run| !run.is_empty()
                    && run.len() <= FIPS_ENDPOINT_DIRECT_PACKET_RUN_MAX_PACKETS)
        );
        let run_count = runs.len();
        let packets = runs.iter().map(FipsEndpointDirectPacketRun::len).sum();
        let enqueued_at = crate::pipeline_profile::stamp();
        let mut state = self
            .state
            .lock()
            .map_err(|_| FipsEndpointDirectDeliveryError::Unavailable)?;
        let Some(queued_packets) = state.packets.checked_add(packets) else {
            drop(state);
            return Err(FipsEndpointDirectDeliveryError::Unavailable);
        };
        if queued_packets > FIPS_ENDPOINT_DIRECT_PACKET_QUEUE_MAX_PACKETS {
            drop(state);
            return Err(FipsEndpointDirectDeliveryError::Unavailable);
        }
        let wake_consumer = state.runs.is_empty();
        let queue_depth = state.runs.len().saturating_add(runs.len());
        state.packets = queued_packets;
        state.runs.extend(
            runs.into_iter()
                .map(|run| FipsDirectEndpointQueuedRun { run, enqueued_at }),
        );
        drop(state);
        if wake_consumer {
            self.ready.notify_one();
        }
        crate::pipeline_profile::record_direct_endpoint_sink_batch(
            run_count,
            packets,
            queue_depth,
        );
        Ok(())
    }

    fn interrupt(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.interrupt_pending = true;
        }
        self.ready.notify_all();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsDirectEndpointQueue {
    fn recv_source_batch_timeout(
        &self,
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::RecvTimeoutError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
        if state.runs.is_empty() {
            let (next_state, wait) = self
                .ready
                .wait_timeout_while(state, timeout, |state| {
                    state.runs.is_empty() && !state.interrupt_pending
                })
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)?;
            state = next_state;
            let interrupted = std::mem::take(&mut state.interrupt_pending);
            if interrupted && state.runs.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
            if wait.timed_out() && state.runs.is_empty() {
                return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
            }
        }
        let queued = state
            .runs
            .pop_front()
            .ok_or(std::sync::mpsc::RecvTimeoutError::Timeout)?;
        Ok(coalesce_direct_endpoint_runs(queued, limit, &mut state))
    }

    fn try_recv_limited(
        &self,
        limit: usize,
    ) -> Result<Vec<FipsEndpointDirectPacketRun>, std::sync::mpsc::TryRecvError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)?;
        let limit = limit
            .max(1)
            .min(FIPS_ENDPOINT_DIRECT_PACKET_RUN_MAX_PACKETS);
        if state
            .runs
            .front()
            .is_some_and(|queued| queued.run.len() > limit)
        {
            return Err(std::sync::mpsc::TryRecvError::Empty);
        }
        let queued = state
            .runs
            .pop_front()
            .ok_or(std::sync::mpsc::TryRecvError::Empty)?;
        Ok(coalesce_direct_endpoint_runs(queued, limit, &mut state))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn coalesce_direct_endpoint_runs(
    queued: FipsDirectEndpointQueuedRun,
    limit: usize,
    state: &mut FipsDirectEndpointQueueState,
) -> Vec<FipsEndpointDirectPacketRun> {
    let limit = limit
        .max(1)
        .min(FIPS_ENDPOINT_DIRECT_PACKET_RUN_MAX_PACKETS);
    let source_node_addr = *queued.run.source_peer().node_addr().as_bytes();
    let mut packet_count = queued.run.len();
    record_direct_endpoint_queue_residence(queued.enqueued_at);
    let mut runs = vec![queued.run];
    while packet_count < limit {
        let Some(next) = state.runs.front() else {
            break;
        };
        let next_packets = next.run.len();
        if next.run.source_peer().node_addr().as_bytes() != &source_node_addr
            || packet_count.saturating_add(next_packets) > limit
        {
            break;
        }
        let next = state
            .runs
            .pop_front()
            .expect("front run must remain present while queue lock is held");
        record_direct_endpoint_queue_residence(next.enqueued_at);
        packet_count = packet_count.saturating_add(next_packets);
        runs.push(next.run);
    }
    state.packets = state
        .packets
        .checked_sub(packet_count)
        .expect("queued packet count must cover every removed direct packet run");
    crate::pipeline_profile::record_direct_endpoint_rx_batch(runs.len(), packet_count);
    runs
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn record_direct_endpoint_queue_residence(enqueued_at: Option<Instant>) {
    crate::pipeline_profile::record_since(
        crate::pipeline_profile::Stage::DirectEndpointQueue,
        enqueued_at,
    );
}
