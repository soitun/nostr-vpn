use std::sync::OnceLock;
use std::sync::atomic::{
    AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};
use std::time::Instant;

const N_STAGES: usize = 13;
const N_COUNTERS: usize = 41;
const HIST_BUCKETS: usize = 48;

#[derive(Copy, Clone)]
#[repr(usize)]
pub(crate) enum Stage {
    TunRead = 0,
    MeshSend = 1,
    MeshRoute = 2,
    MeshEndpointSend = 3,
    TunWrite = 4,
    TunWriteBatch = 5,
    MeshRecv = 6,
    DirectEndpointQueue = 7,
    DirectEndpointWake = 8,
    DirectEndpointRecv = 9,
    DirectEndpointFinalize = 10,
    DirectEndpointBacklog = 11,
    DirectEndpointConsumerBusy = 12,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::TunRead => "nvpn_tun_read",
            Stage::MeshSend => "nvpn_mesh_send",
            Stage::MeshRoute => "nvpn_mesh_route",
            Stage::MeshEndpointSend => "nvpn_mesh_endpoint_send",
            Stage::TunWrite => "nvpn_tun_write",
            Stage::TunWriteBatch => "nvpn_tun_write_batch",
            Stage::MeshRecv => "nvpn_mesh_recv",
            Stage::DirectEndpointQueue => "nvpn_direct_endpoint_queue",
            Stage::DirectEndpointWake => "nvpn_direct_endpoint_wake",
            Stage::DirectEndpointRecv => "nvpn_direct_endpoint_recv",
            Stage::DirectEndpointFinalize => "nvpn_direct_endpoint_finalize",
            Stage::DirectEndpointBacklog => "nvpn_direct_endpoint_backlog",
            Stage::DirectEndpointConsumerBusy => "nvpn_direct_endpoint_consumer_busy",
        }
    }
}

#[derive(Copy, Clone)]
#[repr(usize)]
pub(crate) enum Counter {
    TunReadBatchFlush = 0,
    TunReadBatchPackets = 1,
    TunReadBatchFull = 2,
    TunReadBatchSingle = 3,
    TunReadPacketBytes = 4,
    MeshRecvBatchFlush = 5,
    MeshRecvBatchEvents = 6,
    MeshRecvBatchPackets = 7,
    MeshRecvPacketBytes = 8,
    MeshRecvBatchFull = 9,
    MeshRecvBatchSinglePacket = 10,
    MeshSendBatchFlush = 11,
    MeshSendBatchInputPackets = 12,
    MeshSendBatchRoutedPackets = 13,
    MeshSendBatchRuns = 14,
    MeshSendBatchFull = 15,
    TunWritePackets = 16,
    TunWritePacketBytes = 17,
    TunWriteWouldBlock = 18,
    TunWriteFrames = 19,
    TunWriteFrameBytes = 20,
    TunReadVnetGsoFrames = 21,
    TunReadVnetGsoSegments = 22,
    TunReadVnetGsoSegmentBytes = 23,
    MeshSendBulkTurnDecisions = 24,
    MeshSendBulkTurnBacklogBatches = 25,
    MeshSendBulkTurnCurrentBatchPackets = 26,
    TunWriteVnetGroVectoredFrames = 27,
    TunWriteVnetGroVectoredBytes = 28,
    DirectEndpointSinkBatches = 29,
    DirectEndpointSinkRuns = 30,
    DirectEndpointSinkPackets = 31,
    DirectEndpointSinkMaxQueueDepth = 32,
    DirectEndpointSinkWaitingBatches = 33,
    DirectEndpointSinkWaitingPackets = 34,
    DirectEndpointRxBatches = 35,
    DirectEndpointRxRuns = 36,
    DirectEndpointRxPackets = 37,
    DirectEndpointRxCoalescedBatches = 38,
    DirectEndpointRxLimitSplits = 39,
    DirectEndpointRxLimitTailPackets = 40,
}

impl Counter {
    fn name(self) -> &'static str {
        match self {
            Counter::TunReadBatchFlush => "nvpn_tun_read_batch_flush",
            Counter::TunReadBatchPackets => "nvpn_tun_read_batch_packets",
            Counter::TunReadBatchFull => "nvpn_tun_read_batch_full",
            Counter::TunReadBatchSingle => "nvpn_tun_read_batch_single",
            Counter::TunReadPacketBytes => "nvpn_tun_read_packet_bytes",
            Counter::MeshRecvBatchFlush => "nvpn_mesh_recv_batch_flush",
            Counter::MeshRecvBatchEvents => "nvpn_mesh_recv_batch_events",
            Counter::MeshRecvBatchPackets => "nvpn_mesh_recv_batch_packets",
            Counter::MeshRecvPacketBytes => "nvpn_mesh_recv_packet_bytes",
            Counter::MeshRecvBatchFull => "nvpn_mesh_recv_batch_full",
            Counter::MeshRecvBatchSinglePacket => "nvpn_mesh_recv_batch_single_packet",
            Counter::MeshSendBatchFlush => "nvpn_mesh_send_batch_flush",
            Counter::MeshSendBatchInputPackets => "nvpn_mesh_send_batch_input_packets",
            Counter::MeshSendBatchRoutedPackets => "nvpn_mesh_send_batch_routed_packets",
            Counter::MeshSendBatchRuns => "nvpn_mesh_send_batch_runs",
            Counter::MeshSendBatchFull => "nvpn_mesh_send_batch_full",
            Counter::TunWritePackets => "nvpn_tun_write_packets",
            Counter::TunWritePacketBytes => "nvpn_tun_write_packet_bytes",
            Counter::TunWriteWouldBlock => "nvpn_tun_write_would_block",
            Counter::TunWriteFrames => "nvpn_tun_write_frames",
            Counter::TunWriteFrameBytes => "nvpn_tun_write_frame_bytes",
            Counter::TunReadVnetGsoFrames => "nvpn_tun_read_vnet_gso_frames",
            Counter::TunReadVnetGsoSegments => "nvpn_tun_read_vnet_gso_segments",
            Counter::TunReadVnetGsoSegmentBytes => "nvpn_tun_read_vnet_gso_segment_bytes",
            Counter::MeshSendBulkTurnDecisions => "nvpn_mesh_send_bulk_turn_decisions",
            Counter::MeshSendBulkTurnBacklogBatches => "nvpn_mesh_send_bulk_turn_backlog_batches",
            Counter::MeshSendBulkTurnCurrentBatchPackets => {
                "nvpn_mesh_send_bulk_turn_current_batch_packets"
            }
            Counter::TunWriteVnetGroVectoredFrames => "nvpn_tun_write_vnet_gro_vectored_frames",
            Counter::TunWriteVnetGroVectoredBytes => "nvpn_tun_write_vnet_gro_vectored_bytes",
            Counter::DirectEndpointSinkBatches => "nvpn_direct_endpoint_sink_batches",
            Counter::DirectEndpointSinkRuns => "nvpn_direct_endpoint_sink_runs",
            Counter::DirectEndpointSinkPackets => "nvpn_direct_endpoint_sink_packets",
            Counter::DirectEndpointSinkMaxQueueDepth => "nvpn_direct_endpoint_sink_max_queue_depth",
            Counter::DirectEndpointSinkWaitingBatches => {
                "nvpn_direct_endpoint_sink_waiting_batches"
            }
            Counter::DirectEndpointSinkWaitingPackets => {
                "nvpn_direct_endpoint_sink_waiting_packets"
            }
            Counter::DirectEndpointRxBatches => "nvpn_direct_endpoint_rx_batches",
            Counter::DirectEndpointRxRuns => "nvpn_direct_endpoint_rx_runs",
            Counter::DirectEndpointRxPackets => "nvpn_direct_endpoint_rx_packets",
            Counter::DirectEndpointRxCoalescedBatches => {
                "nvpn_direct_endpoint_rx_coalesced_batches"
            }
            Counter::DirectEndpointRxLimitSplits => "nvpn_direct_endpoint_rx_limit_splits",
            Counter::DirectEndpointRxLimitTailPackets => {
                "nvpn_direct_endpoint_rx_limit_tail_packets"
            }
        }
    }
}

fn counter_from_index(idx: usize) -> Counter {
    match idx {
        0 => Counter::TunReadBatchFlush,
        1 => Counter::TunReadBatchPackets,
        2 => Counter::TunReadBatchFull,
        3 => Counter::TunReadBatchSingle,
        4 => Counter::TunReadPacketBytes,
        5 => Counter::MeshRecvBatchFlush,
        6 => Counter::MeshRecvBatchEvents,
        7 => Counter::MeshRecvBatchPackets,
        8 => Counter::MeshRecvPacketBytes,
        9 => Counter::MeshRecvBatchFull,
        10 => Counter::MeshRecvBatchSinglePacket,
        11 => Counter::MeshSendBatchFlush,
        12 => Counter::MeshSendBatchInputPackets,
        13 => Counter::MeshSendBatchRoutedPackets,
        14 => Counter::MeshSendBatchRuns,
        15 => Counter::MeshSendBatchFull,
        16 => Counter::TunWritePackets,
        17 => Counter::TunWritePacketBytes,
        18 => Counter::TunWriteWouldBlock,
        19 => Counter::TunWriteFrames,
        20 => Counter::TunWriteFrameBytes,
        21 => Counter::TunReadVnetGsoFrames,
        22 => Counter::TunReadVnetGsoSegments,
        23 => Counter::TunReadVnetGsoSegmentBytes,
        24 => Counter::MeshSendBulkTurnDecisions,
        25 => Counter::MeshSendBulkTurnBacklogBatches,
        26 => Counter::MeshSendBulkTurnCurrentBatchPackets,
        27 => Counter::TunWriteVnetGroVectoredFrames,
        28 => Counter::TunWriteVnetGroVectoredBytes,
        29 => Counter::DirectEndpointSinkBatches,
        30 => Counter::DirectEndpointSinkRuns,
        31 => Counter::DirectEndpointSinkPackets,
        32 => Counter::DirectEndpointSinkMaxQueueDepth,
        33 => Counter::DirectEndpointSinkWaitingBatches,
        34 => Counter::DirectEndpointSinkWaitingPackets,
        35 => Counter::DirectEndpointRxBatches,
        36 => Counter::DirectEndpointRxRuns,
        37 => Counter::DirectEndpointRxPackets,
        38 => Counter::DirectEndpointRxCoalescedBatches,
        39 => Counter::DirectEndpointRxLimitSplits,
        40 => Counter::DirectEndpointRxLimitTailPackets,
        _ => unreachable!(),
    }
}

fn stage_from_index(idx: usize) -> Stage {
    match idx {
        0 => Stage::TunRead,
        1 => Stage::MeshSend,
        2 => Stage::MeshRoute,
        3 => Stage::MeshEndpointSend,
        4 => Stage::TunWrite,
        5 => Stage::TunWriteBatch,
        6 => Stage::MeshRecv,
        7 => Stage::DirectEndpointQueue,
        8 => Stage::DirectEndpointWake,
        9 => Stage::DirectEndpointRecv,
        10 => Stage::DirectEndpointFinalize,
        11 => Stage::DirectEndpointBacklog,
        12 => Stage::DirectEndpointConsumerBusy,
        _ => unreachable!(),
    }
}

static TOTAL_NS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static COUNT: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static MAX_NS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static HIST: [AtomicU64; N_STAGES * HIST_BUCKETS] =
    [const { AtomicU64::new(0) }; N_STAGES * HIST_BUCKETS];
static COUNTERS: [AtomicU64; N_COUNTERS] = [const { AtomicU64::new(0) }; N_COUNTERS];

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if cfg!(debug_assertions) || option_env!("NVPN_FORCE_PIPELINE_TRACE").is_some() {
            return true;
        }
        ["NVPN_PIPELINE_TRACE", "FIPS_PIPELINE_TRACE"]
            .into_iter()
            .any(|key| {
                std::env::var(key)
                    .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
            })
    })
}

#[inline]
pub(crate) fn stamp() -> Option<Instant> {
    enabled().then(Instant::now)
}

#[inline]
pub(crate) fn record_since(stage: Stage, start: Option<Instant>) {
    if let Some(start) = start {
        record(stage, start.elapsed().as_nanos() as u64);
    }
}

pub(crate) fn record(stage: Stage, elapsed_ns: u64) {
    if !enabled() {
        return;
    }
    let idx = stage as usize;
    let elapsed_ns = elapsed_ns.max(1);
    TOTAL_NS[idx].fetch_add(elapsed_ns, Relaxed);
    MAX_NS[idx].fetch_max(elapsed_ns, Relaxed);
    HIST[(idx * HIST_BUCKETS) + bucket_for_ns(elapsed_ns)].fetch_add(1, Relaxed);
    COUNT[idx].fetch_add(1, Release);
}

pub(crate) fn increment_counter_by(counter: Counter, amount: u64) {
    if amount > 0 && enabled() {
        COUNTERS[counter as usize].fetch_add(amount, Relaxed);
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn max_counter(counter: Counter, value: u64) {
    if value > 0 && enabled() {
        COUNTERS[counter as usize].fetch_max(value, Relaxed);
    }
}

pub(crate) fn record_mesh_send_bulk_turn(backlog_batches: usize, current_batch_packets: usize) {
    if !enabled() {
        return;
    }
    increment_counter_by(Counter::MeshSendBulkTurnDecisions, 1);
    increment_counter_by(
        Counter::MeshSendBulkTurnBacklogBatches,
        backlog_batches as u64,
    );
    increment_counter_by(
        Counter::MeshSendBulkTurnCurrentBatchPackets,
        current_batch_packets as u64,
    );
}

pub(crate) fn record_tun_read_batch(packets: usize, bytes: usize, max_batch: usize) {
    if packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::TunReadBatchFlush, 1);
    increment_counter_by(Counter::TunReadBatchPackets, packets as u64);
    increment_counter_by(Counter::TunReadPacketBytes, bytes as u64);
    if packets >= max_batch.max(1) {
        increment_counter_by(Counter::TunReadBatchFull, 1);
    }
    if packets == 1 {
        increment_counter_by(Counter::TunReadBatchSingle, 1);
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn record_tun_read_vnet_gso_split(segments: usize, segment_bytes: usize) {
    if segments == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::TunReadVnetGsoFrames, 1);
    increment_counter_by(Counter::TunReadVnetGsoSegments, segments as u64);
    increment_counter_by(Counter::TunReadVnetGsoSegmentBytes, segment_bytes as u64);
}

pub(crate) fn record_mesh_recv_batch(
    events: usize,
    packets: usize,
    packet_bytes: usize,
    max_batch: usize,
) {
    if events == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::MeshRecvBatchFlush, 1);
    increment_counter_by(Counter::MeshRecvBatchEvents, events as u64);
    increment_counter_by(Counter::MeshRecvBatchPackets, packets as u64);
    increment_counter_by(Counter::MeshRecvPacketBytes, packet_bytes as u64);
    if events >= max_batch.max(1) {
        increment_counter_by(Counter::MeshRecvBatchFull, 1);
    }
    if packets == 1 {
        increment_counter_by(Counter::MeshRecvBatchSinglePacket, 1);
    }
}

pub(crate) fn record_mesh_send_batch(
    input_packets: usize,
    routed_packets: usize,
    runs: usize,
    max_batch: usize,
) {
    if input_packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::MeshSendBatchFlush, 1);
    increment_counter_by(Counter::MeshSendBatchInputPackets, input_packets as u64);
    increment_counter_by(Counter::MeshSendBatchRoutedPackets, routed_packets as u64);
    increment_counter_by(Counter::MeshSendBatchRuns, runs as u64);
    if input_packets >= max_batch.max(1) {
        increment_counter_by(Counter::MeshSendBatchFull, 1);
    }
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn record_tun_write_packet(bytes: usize) {
    record_tun_write_packets(1, bytes);
}

pub(crate) fn record_tun_write_packets(packets: usize, bytes: usize) {
    if packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::TunWritePackets, packets as u64);
    increment_counter_by(Counter::TunWritePacketBytes, bytes as u64);
}

pub(crate) fn record_tun_write_frame(bytes: usize) {
    if bytes == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::TunWriteFrames, 1);
    increment_counter_by(Counter::TunWriteFrameBytes, bytes as u64);
}

#[cfg(target_os = "linux")]
pub(crate) fn record_tun_write_vnet_gro_vectored_frame(segments: usize, bytes: usize) {
    if segments == 0 || bytes == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::TunWriteVnetGroVectoredFrames, 1);
    increment_counter_by(Counter::TunWriteVnetGroVectoredBytes, bytes as u64);
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn record_tun_write_would_block() {
    increment_counter_by(Counter::TunWriteWouldBlock, 1);
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) fn record_direct_endpoint_sink_batch(
    runs: usize,
    packets: usize,
    queue_depth: usize,
    waiting_consumer: bool,
) {
    if packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::DirectEndpointSinkBatches, 1);
    increment_counter_by(Counter::DirectEndpointSinkRuns, runs as u64);
    increment_counter_by(Counter::DirectEndpointSinkPackets, packets as u64);
    max_counter(Counter::DirectEndpointSinkMaxQueueDepth, queue_depth as u64);
    if waiting_consumer {
        increment_counter_by(Counter::DirectEndpointSinkWaitingBatches, 1);
        increment_counter_by(Counter::DirectEndpointSinkWaitingPackets, packets as u64);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) fn record_direct_endpoint_rx_batch(
    runs: usize,
    packets: usize,
    coalesced_batches: usize,
) {
    if packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::DirectEndpointRxBatches, 1);
    increment_counter_by(Counter::DirectEndpointRxRuns, runs as u64);
    increment_counter_by(Counter::DirectEndpointRxPackets, packets as u64);
    increment_counter_by(
        Counter::DirectEndpointRxCoalescedBatches,
        coalesced_batches.saturating_sub(1) as u64,
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) fn record_direct_endpoint_rx_limit_split(tail_packets: usize) {
    if tail_packets == 0 || !enabled() {
        return;
    }
    increment_counter_by(Counter::DirectEndpointRxLimitSplits, 1);
    increment_counter_by(
        Counter::DirectEndpointRxLimitTailPackets,
        tail_packets as u64,
    );
}

pub(crate) struct Timer {
    stage: Stage,
    start: Option<Instant>,
}

impl Timer {
    #[inline]
    pub(crate) fn start(stage: Stage) -> Self {
        Self {
            stage,
            start: stamp(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        record_since(self.stage, self.start);
    }
}

pub(crate) fn maybe_spawn_reporter() {
    if !enabled() {
        return;
    }
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let interval = std::env::var("NVPN_PIPELINE_INTERVAL_SECS")
        .ok()
        .or_else(|| std::env::var("FIPS_PERF_INTERVAL_SECS").ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5)
        .max(1);
    tokio::spawn(async move {
        let mut prev_total = [0u64; N_STAGES];
        let mut prev_count = [0u64; N_STAGES];
        let mut prev_hist = [0u64; N_STAGES * HIST_BUCKETS];
        let mut prev_counters = [0u64; N_COUNTERS];
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            let mut line = format!("[nvpn-pipe {}s]", interval);
            for i in 0..N_STAGES {
                let count = COUNT[i].load(Acquire);
                let dc = count.saturating_sub(prev_count[i]);
                if dc == 0 {
                    continue;
                }
                let total = TOTAL_NS[i].load(Relaxed);
                let dt = total.saturating_sub(prev_total[i]);
                prev_total[i] = total;
                prev_count[i] = count;

                let base = i * HIST_BUCKETS;
                let mut hist_delta = [0u64; HIST_BUCKETS];
                for (bucket, slot) in hist_delta.iter_mut().enumerate() {
                    let idx = base + bucket;
                    let current = HIST[idx].load(Relaxed);
                    *slot = current.saturating_sub(prev_hist[idx]);
                    prev_hist[idx] = current;
                }

                let stage = stage_from_index(i);
                let avg_ns = dt / dc;
                let pps = dc / interval;
                let p50 = percentile_ns(&hist_delta, dc, 50);
                let p95 = percentile_ns(&hist_delta, dc, 95);
                let p99 = percentile_ns(&hist_delta, dc, 99);
                let approx_max = interval_max_ns(&hist_delta);
                let lifetime_max = MAX_NS[i].load(Relaxed);
                line.push_str(&format!(
                    " {}={}/s avg={} p50<={} p95<={} p99<={} max<={} allmax={}",
                    stage.name(),
                    pps,
                    fmt_ns(avg_ns),
                    fmt_ns(p50),
                    fmt_ns(p95),
                    fmt_ns(p99),
                    fmt_ns(approx_max),
                    fmt_ns(lifetime_max),
                ));
            }
            for i in 0..N_COUNTERS {
                let current = COUNTERS[i].load(Relaxed);
                let delta = current.saturating_sub(prev_counters[i]);
                prev_counters[i] = current;
                if delta == 0 {
                    continue;
                }
                line.push_str(&format!(
                    " {}={}/s total={}",
                    counter_from_index(i).name(),
                    delta / interval,
                    current,
                ));
            }
            eprintln!("{line}");
        }
    });
}

fn bucket_for_ns(ns: u64) -> usize {
    if ns <= 1 {
        return 0;
    }
    ((u64::BITS - (ns - 1).leading_zeros()) as usize).min(HIST_BUCKETS - 1)
}

fn bucket_upper_ns(bucket: usize) -> u64 {
    if bucket == 0 {
        1
    } else if bucket >= 63 {
        u64::MAX
    } else {
        1u64 << bucket
    }
}

fn percentile_ns(hist_delta: &[u64; HIST_BUCKETS], total: u64, pct: u64) -> u64 {
    let observed_total = hist_delta.iter().copied().sum::<u64>();
    let total = total.min(observed_total);
    if total == 0 {
        return 0;
    }
    let target = total.saturating_mul(pct).saturating_add(99) / 100;
    let mut seen = 0u64;
    for (idx, count) in hist_delta.iter().enumerate() {
        seen = seen.saturating_add(*count);
        if seen >= target {
            return bucket_upper_ns(idx);
        }
    }
    interval_max_ns(hist_delta)
}

fn interval_max_ns(hist_delta: &[u64; HIST_BUCKETS]) -> u64 {
    for idx in (0..HIST_BUCKETS).rev() {
        if hist_delta[idx] != 0 {
            return bucket_upper_ns(idx);
        }
    }
    0
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.1}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Counter, HIST_BUCKETS, N_COUNTERS, Stage, bucket_upper_ns, counter_from_index,
        percentile_ns, stage_from_index,
    };

    #[test]
    fn percentile_uses_observed_histogram_count_when_stage_count_leads() {
        let mut hist = [0u64; HIST_BUCKETS];
        hist[10] = 1;

        assert_eq!(percentile_ns(&hist, 2, 99), bucket_upper_ns(10));
        assert_eq!(percentile_ns(&[0u64; HIST_BUCKETS], 1, 99), 0);
    }

    #[test]
    fn mesh_send_pipeline_names_are_stable() {
        assert_eq!(N_COUNTERS, 41);
        assert_eq!(Stage::TunRead.name(), "nvpn_tun_read");
        assert_eq!(Stage::MeshRecv.name(), "nvpn_mesh_recv");
        assert_eq!(Stage::MeshRoute.name(), "nvpn_mesh_route");
        assert_eq!(Stage::MeshEndpointSend.name(), "nvpn_mesh_endpoint_send");
        assert_eq!(
            Stage::DirectEndpointQueue.name(),
            "nvpn_direct_endpoint_queue"
        );
        assert_eq!(Stage::TunWriteBatch.name(), "nvpn_tun_write_batch");
        assert_eq!(
            Stage::DirectEndpointWake.name(),
            "nvpn_direct_endpoint_wake"
        );
        assert_eq!(
            Stage::DirectEndpointBacklog.name(),
            "nvpn_direct_endpoint_backlog"
        );
        assert_eq!(
            Stage::DirectEndpointConsumerBusy.name(),
            "nvpn_direct_endpoint_consumer_busy"
        );
        assert_eq!(
            Stage::DirectEndpointRecv.name(),
            "nvpn_direct_endpoint_recv"
        );
        assert_eq!(
            Stage::DirectEndpointFinalize.name(),
            "nvpn_direct_endpoint_finalize"
        );
        assert_eq!(
            Counter::MeshSendBatchInputPackets.name(),
            "nvpn_mesh_send_batch_input_packets"
        );
        assert_eq!(
            Counter::TunReadVnetGsoSegments.name(),
            "nvpn_tun_read_vnet_gso_segments"
        );
        assert_eq!(
            Counter::MeshSendBulkTurnCurrentBatchPackets.name(),
            "nvpn_mesh_send_bulk_turn_current_batch_packets"
        );
        assert_eq!(
            Counter::MeshSendBatchRoutedPackets.name(),
            "nvpn_mesh_send_batch_routed_packets"
        );
        assert_eq!(
            Counter::TunWriteVnetGroVectoredBytes.name(),
            "nvpn_tun_write_vnet_gro_vectored_bytes"
        );
        assert_eq!(
            Counter::DirectEndpointSinkMaxQueueDepth.name(),
            "nvpn_direct_endpoint_sink_max_queue_depth"
        );
        assert_eq!(
            Counter::DirectEndpointSinkWaitingBatches.name(),
            "nvpn_direct_endpoint_sink_waiting_batches"
        );
        assert_eq!(
            Counter::DirectEndpointSinkWaitingPackets.name(),
            "nvpn_direct_endpoint_sink_waiting_packets"
        );
        assert_eq!(
            Counter::DirectEndpointRxCoalescedBatches.name(),
            "nvpn_direct_endpoint_rx_coalesced_batches"
        );
        assert_eq!(
            Counter::DirectEndpointRxLimitSplits.name(),
            "nvpn_direct_endpoint_rx_limit_splits"
        );
        assert_eq!(
            Counter::DirectEndpointRxLimitTailPackets.name(),
            "nvpn_direct_endpoint_rx_limit_tail_packets"
        );
        assert_eq!(
            stage_from_index(Stage::MeshRoute as usize).name(),
            "nvpn_mesh_route"
        );
        assert_eq!(
            stage_from_index(Stage::DirectEndpointQueue as usize).name(),
            "nvpn_direct_endpoint_queue"
        );
        assert_eq!(
            counter_from_index(Counter::MeshSendBatchRuns as usize).name(),
            "nvpn_mesh_send_batch_runs"
        );
    }
}
