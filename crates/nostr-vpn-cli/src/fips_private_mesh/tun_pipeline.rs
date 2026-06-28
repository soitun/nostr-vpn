#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn drain_event_batch(
    event_rx: &mut mpsc::Receiver<FipsPrivateMeshEvent>,
    limit: usize,
) -> Vec<FipsPrivateMeshEvent> {
    let mut events = Vec::new();
    for _ in 0..limit {
        let Ok(event) = event_rx.try_recv() else {
            break;
        };
        events.push(event);
    }
    events
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn submit_tun_packet_batch_to_mesh_queue(
    packet_tx: &TunPipelineQueueTx,
    batch: TunPipelineBatch,
) -> TunQueueSubmit {
    match tun_pipeline_split_batch_by_lane(batch) {
        TunPipelineSubmitBatches::Empty => TunQueueSubmit::Enqueued,
        TunPipelineSubmitBatches::Single { lane, batch } => {
            submit_tun_packet_batch_to_lane(packet_tx, lane, batch)
        }
        TunPipelineSubmitBatches::Split { priority, bulk } => {
            if matches!(
                submit_tun_packet_batch_to_lane(packet_tx, TunPipelineLane::Priority, priority),
                TunQueueSubmit::Closed
            ) {
                return TunQueueSubmit::Closed;
            }
            submit_tun_packet_batch_to_lane(packet_tx, TunPipelineLane::Bulk, bulk)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn submit_tun_packet_batch_to_mesh_queue_with_backpressure(
    packet_tx: &TunPipelineQueueTx,
    batch: TunPipelineBatch,
    max_bulk_chunk_packets: usize,
) -> TunQueueSubmit {
    match tun_pipeline_split_batch_by_lane(batch) {
        TunPipelineSubmitBatches::Empty => TunQueueSubmit::Enqueued,
        TunPipelineSubmitBatches::Single { lane, batch } => match lane {
            TunPipelineLane::Priority => submit_tun_packet_batch_to_lane(packet_tx, lane, batch),
            TunPipelineLane::Bulk => {
                submit_tun_bulk_batch_with_backpressure(
                    packet_tx,
                    batch,
                    max_bulk_chunk_packets,
                )
                .await
            }
        },
        TunPipelineSubmitBatches::Split { priority, bulk } => {
            if matches!(
                submit_tun_packet_batch_to_lane(packet_tx, TunPipelineLane::Priority, priority),
                TunQueueSubmit::Closed
            ) {
                return TunQueueSubmit::Closed;
            }
            submit_tun_bulk_batch_with_backpressure(packet_tx, bulk, max_bulk_chunk_packets).await
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum TunPipelineSubmitBatches {
    Empty,
    Single {
        lane: TunPipelineLane,
        batch: TunPipelineBatch,
    },
    Split {
        priority: TunPipelineBatch,
        bulk: TunPipelineBatch,
    },
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn tun_pipeline_split_batch_by_lane(batch: TunPipelineBatch) -> TunPipelineSubmitBatches {
    let packet_count = batch.len();
    if packet_count == 0 {
        return TunPipelineSubmitBatches::Empty;
    }

    let priority_count = batch
        .iter()
        .filter(|packet| packet.lane() == TunPipelineLane::Priority)
        .count();
    if priority_count == 0 || priority_count == packet_count {
        let lane = if priority_count == 0 {
            TunPipelineLane::Bulk
        } else {
            TunPipelineLane::Priority
        };
        return TunPipelineSubmitBatches::Single { lane, batch };
    }

    let mut priority = Vec::with_capacity(priority_count);
    let mut bulk = Vec::with_capacity(packet_count - priority_count);
    for packet in batch {
        match packet.lane() {
            TunPipelineLane::Priority => priority.push(packet),
            TunPipelineLane::Bulk => bulk.push(packet),
        }
    }
    TunPipelineSubmitBatches::Split { priority, bulk }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn submit_tun_packet_batch_to_lane(
    packet_tx: &TunPipelineQueueTx,
    lane: TunPipelineLane,
    batch: TunPipelineBatch,
) -> TunQueueSubmit {
    match lane {
        TunPipelineLane::Priority => packet_tx
            .priority
            .send(batch)
            .map(|_| TunQueueSubmit::Enqueued)
            .unwrap_or(TunQueueSubmit::Closed),
        TunPipelineLane::Bulk => submit_tun_bulk_batch(packet_tx, batch),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn submit_tun_bulk_batch(
    packet_tx: &TunPipelineQueueTx,
    batch: TunPipelineBatch,
) -> TunQueueSubmit {
    let packet_count = batch.len();
    if packet_tx.bulk.is_closed() {
        return TunQueueSubmit::Closed;
    }
    if !try_reserve_tun_bulk_packet_slots(packet_tx, packet_count) {
        if packet_tx.bulk.is_closed() {
            return TunQueueSubmit::Closed;
        }
        crate::pipeline_profile::record_tun_to_mesh_bulk_drop_packet_cap(packet_count);
        return TunQueueSubmit::DroppedBulk;
    }

    match packet_tx.bulk.try_send(batch) {
        Ok(()) => TunQueueSubmit::Enqueued,
        Err(mpsc::error::TrySendError::Full(_batch)) => {
            release_tun_bulk_packet_slots(&packet_tx.bulk_queued_packets, packet_count);
            packet_tx.bulk_available.notify_waiters();
            crate::pipeline_profile::record_tun_to_mesh_bulk_drop_channel_full(packet_count);
            TunQueueSubmit::DroppedBulk
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            release_tun_bulk_packet_slots(&packet_tx.bulk_queued_packets, packet_count);
            packet_tx.bulk_available.notify_waiters();
            TunQueueSubmit::Closed
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn submit_tun_bulk_batch_with_backpressure(
    packet_tx: &TunPipelineQueueTx,
    mut batch: TunPipelineBatch,
    max_chunk_packets: usize,
) -> TunQueueSubmit {
    let max_chunk_packets = packet_tx.tun_read_backpressure_min_slots(max_chunk_packets);
    while !batch.is_empty() {
        let chunk_len = batch.len().min(max_chunk_packets);
        if !packet_tx.wait_for_tun_read_bulk_slots(chunk_len).await {
            return TunQueueSubmit::Closed;
        }
        let rest = if batch.len() > chunk_len {
            batch.split_off(chunk_len)
        } else {
            Vec::new()
        };
        match submit_tun_bulk_batch(packet_tx, batch) {
            TunQueueSubmit::Enqueued => {
                batch = rest;
            }
            other => return other,
        }
    }
    TunQueueSubmit::Enqueued
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn try_reserve_tun_bulk_packet_slots(packet_tx: &TunPipelineQueueTx, packet_count: usize) -> bool {
    if packet_count == 0 {
        return true;
    }

    let capacity = packet_tx.bulk_packet_capacity;
    packet_tx
        .bulk_queued_packets
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current
                .checked_add(packet_count)
                .filter(|next| *next <= capacity)
        })
        .is_ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn release_tun_bulk_packet_slots(counter: &AtomicUsize, packet_count: usize) {
    if packet_count == 0 {
        return;
    }

    let previous = counter.fetch_sub(packet_count, Ordering::Relaxed);
    debug_assert!(
        previous >= packet_count,
        "TUN-to-mesh bulk queued packet accounting underflow"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn tun_pipeline_packet_lane(packet: &[u8]) -> TunPipelineLane {
    match classify_endpoint_payload(packet).lane() {
        EndpointPayloadLane::Priority => TunPipelineLane::Priority,
        EndpointPayloadLane::Bulk => TunPipelineLane::Bulk,
    }
}
