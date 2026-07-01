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
    record_tun_pipeline_submit_batch(&batch);
    if batch.is_empty() {
        TunQueueSubmit::Enqueued
    } else {
        submit_tun_bulk_batch(packet_tx, batch)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn record_tun_pipeline_submit_batch(batch: &[TunPipelinePacket]) {
    if batch.is_empty() || !crate::pipeline_profile::enabled() {
        return;
    }

    crate::pipeline_profile::record_tun_to_mesh_submit_batch(batch.len());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn submit_tun_bulk_batch(
    packet_tx: &TunPipelineQueueTx,
    batch: TunPipelineBatch,
) -> TunQueueSubmit {
    let packet_count = batch.len();
    crate::pipeline_profile::record_tun_to_mesh_bulk_admission(packet_count);
    if packet_tx.bulk.is_closed() {
        return TunQueueSubmit::Closed;
    }
    if !packet_tx.try_reserve_bulk_packets(packet_count) {
        crate::pipeline_profile::record_tun_to_mesh_bulk_drop_channel_full(packet_count);
        return TunQueueSubmit::DroppedBulk;
    }

    match packet_tx.bulk.try_send(batch) {
        Ok(()) => TunQueueSubmit::Enqueued,
        Err(mpsc::error::TrySendError::Full(_batch)) => {
            packet_tx.release_bulk_packets(packet_count);
            crate::pipeline_profile::record_tun_to_mesh_bulk_drop_channel_full(packet_count);
            TunQueueSubmit::DroppedBulk
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            packet_tx.release_bulk_packets(packet_count);
            TunQueueSubmit::Closed
        }
    }
}
