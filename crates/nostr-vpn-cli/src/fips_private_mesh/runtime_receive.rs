impl FipsPrivateMeshRuntime {
    pub(crate) async fn recv_mesh_event(&self) -> Result<Option<FipsPrivateMeshEvent>> {
        loop {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.drain_direct_endpoint_mesh_events(1).await?;
                if let Some(event) = self.pop_direct_endpoint_mesh_event()? {
                    return Ok(Some(event));
                }

                match tokio::time::timeout(Duration::from_millis(10), self.endpoint.recv()).await {
                    Ok(Some(message)) => {
                        if let Some(event) =
                            self.endpoint_message_to_mesh_event(message, None).await?
                        {
                            return Ok(Some(event));
                        }
                    }
                    Ok(None) => return Ok(None),
                    Err(_) => continue,
                }
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let Some(message) = self.endpoint.recv().await else {
                    return Ok(None);
                };

                if let Some(event) = self.endpoint_message_to_mesh_event(message, None).await? {
                    return Ok(Some(event));
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub(crate) async fn recv_mesh_event_batch(
        &self,
        limit: usize,
    ) -> Result<Option<Vec<FipsPrivateMeshEvent>>> {
        let limit = limit.clamp(1, FIPS_MESH_EVENT_DRAIN_LIMIT);
        let mut messages = Vec::with_capacity(limit);
        let mut events = Vec::with_capacity(limit);
        if self
            .recv_mesh_event_batch_into(&mut messages, &mut events, limit)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(events))
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub(crate) async fn recv_mesh_event_batch_into(
        &self,
        messages: &mut Vec<FipsEndpointMessage>,
        events: &mut Vec<FipsPrivateMeshEvent>,
        limit: usize,
    ) -> Result<Option<usize>> {
        let limit = limit.clamp(1, FIPS_MESH_EVENT_DRAIN_LIMIT);
        events.clear();
        loop {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.drain_direct_endpoint_mesh_events(limit).await?;
                if self.pop_direct_endpoint_mesh_events_into(events, limit)? > 0 {
                    return Ok(Some(events.len()));
                }

                let Some(_) =
                    (match tokio::time::timeout(
                        Duration::from_millis(10),
                        self.endpoint.recv_batch_into(messages, limit),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => continue,
                    })
                else {
                    return Ok(None);
                };
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let Some(_) = self.endpoint.recv_batch_into(messages, limit).await else {
                    return Ok(None);
                };
            }
            let now = Some(unix_timestamp());
            events.reserve(messages.len());
            for message in messages.drain(..) {
                if let Some(event) = self.endpoint_message_to_mesh_event(message, now).await? {
                    events.push(event);
                }
            }
            if !events.is_empty() {
                return Ok(Some(events.len()));
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn recv_direct_endpoint_tun_batch_blocking(
        &self,
        limit: usize,
        stop: &AtomicBool,
        packet_outputs: &mut DirectTunWriteBatch,
        event_tx: Option<&mpsc::Sender<FipsPrivateMeshEvent>>,
    ) -> Result<Option<usize>> {
        let limit = limit.clamp(1, FIPS_MESH_EVENT_DRAIN_LIMIT);
        let rx = &self.direct_endpoint_rx;

        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }

            let mut emitted = 0usize;
            let mut received = 0usize;
            let mut turn_start = None;
            let (mesh_generation, mesh) = self.stable_mesh_snapshot();
            packet_outputs.set_mesh_generation(mesh_generation);
            while received < limit {
                let runs = if received == 0 {
                    loop {
                        if stop.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        match rx.recv_source_batch_timeout(Duration::from_millis(100), limit) {
                            Ok(runs) => break runs,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                return Ok(None);
                            }
                        }
                    }
                } else {
                    match rx.try_recv_limited(limit.saturating_sub(received)) {
                        Ok(runs) => runs,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(None),
                    }
                };

                if runs.is_empty() {
                    continue;
                }
                if turn_start.is_none() {
                    turn_start = crate::pipeline_profile::stamp();
                }

                self.forward_direct_endpoint_control_events_blocking(&runs, event_tx)?;
                let mut admitted =
                    admit_direct_endpoint_packet_runs_with_mesh(&mesh, runs, packet_outputs);
                received = received.saturating_add(admitted.received);
                packet_outputs.append_data_rx_notes(&mut admitted.data_rx_notes);
                if admitted.accepted > 0 {
                    emitted = emitted.saturating_add(admitted.accepted);
                }
            }

            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            if emitted > 0 {
                crate::pipeline_profile::record_since(
                    crate::pipeline_profile::Stage::DirectEndpointRecv,
                    turn_start,
                );
                return Ok(Some(emitted));
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn wake_blocking_mesh_recv(&self) {
        let npub = self.endpoint.npub().to_string();
        let _ = self.endpoint.blocking_send(npub, Vec::new());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn finalize_direct_endpoint_tun_batch_blocking(
        &self,
        packet_outputs: &mut DirectTunWriteBatch,
    ) -> Result<()> {
        let _t = crate::pipeline_profile::Timer::start(
            crate::pipeline_profile::Stage::DirectEndpointFinalize,
        );
        if packet_outputs.is_empty() {
            packet_outputs.clear();
            return Ok(());
        }
        let mesh_generation = self.mesh_generation();
        if packet_outputs.mesh_generation() != mesh_generation || mesh_generation & 1 != 0 {
            let (mesh_generation, mesh) = self.stable_mesh_snapshot();
            revalidate_direct_endpoint_tun_batch_with_mesh(
                &mesh,
                mesh_generation,
                packet_outputs,
            );
        }
        #[cfg(feature = "paid-exit")]
        self.note_paid_route_inbound_batch(packet_outputs)?;
        self.note_data_rx_batch(packet_outputs.data_rx_notes_mut(), None)
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    pub(crate) async fn try_recv_mesh_event(&self) -> Result<Option<FipsPrivateMeshEvent>> {
        self.try_recv_mesh_event_with_timestamp(None).await
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    async fn try_recv_mesh_event_with_timestamp(
        &self,
        now: Option<u64>,
    ) -> Result<Option<FipsPrivateMeshEvent>> {
        loop {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                self.drain_direct_endpoint_mesh_events(1).await?;
                if let Some(event) = self.pop_direct_endpoint_mesh_event()? {
                    return Ok(Some(event));
                }
            }

            let Some(message) = self.endpoint.try_recv() else {
                return Ok(None);
            };

            if let Some(event) = self.endpoint_message_to_mesh_event(message, now).await? {
                return Ok(Some(event));
            }
        }
    }

    async fn endpoint_message_to_mesh_event(
        &self,
        message: FipsEndpointMessage,
        now: Option<u64>,
    ) -> Result<Option<FipsPrivateMeshEvent>> {
        let outcome = self.endpoint_message_to_mesh_event_outcome(message, now)?;
        if let Some(reply) = outcome.reply
            && let Err(error) = self.endpoint.send_to_peer(reply.peer, reply.data).await
        {
            eprintln!("fips: failed to reply to peer ping: {error}");
        }
        Ok(outcome.event)
    }

    fn endpoint_message_to_mesh_event_outcome(
        &self,
        message: FipsEndpointMessage,
        now: Option<u64>,
    ) -> Result<FipsEndpointMessageOutcome> {
        let mesh = self.mesh.load();
        self.endpoint_message_to_mesh_event_outcome_inner_with_mesh(message, now, &mesh)
    }

    fn endpoint_message_to_mesh_event_outcome_inner_with_mesh(
        &self,
        message: FipsEndpointMessage,
        now: Option<u64>,
        mesh: &FipsMeshRuntime,
    ) -> Result<FipsEndpointMessageOutcome> {
        if let Some(frame) = self.decode_endpoint_control_frame(&message)? {
            let source_pubkey = control_frame_source_pubkey(mesh, message.source_peer, &frame);
            let Some(source_pubkey) = source_pubkey else {
                return Ok(FipsEndpointMessageOutcome::none());
            };
            let now = now.unwrap_or_else(unix_timestamp);
            self.note_control_rx(&source_pubkey, message.data.len(), now)?;
            match frame {
                FipsControlFrame::Ping {
                    network_id,
                    sent_at,
                } => {
                    let reply = FipsControlFrame::Pong {
                        network_id,
                        sent_at,
                        replied_at: now,
                    };
                    let encoded = encode_fips_control_frame(&reply)?;
                    return Ok(FipsEndpointMessageOutcome::event_with_reply(
                        FipsPrivateMeshEvent::Presence {
                            participant_pubkey: source_pubkey,
                            last_seen_at: now,
                        },
                        message.source_peer,
                        encoded,
                    ));
                }
                FipsControlFrame::Pong { sent_at, .. } => {
                    self.note_pong(&source_pubkey, sent_at)?;
                    return Ok(FipsEndpointMessageOutcome::event(
                        FipsPrivateMeshEvent::Presence {
                            participant_pubkey: source_pubkey,
                            last_seen_at: now,
                        },
                    ));
                }
                FipsControlFrame::JoinRequest {
                    requested_at,
                    request,
                } => {
                    return Ok(FipsEndpointMessageOutcome::event(
                        FipsPrivateMeshEvent::JoinRequest {
                            sender_pubkey: source_pubkey,
                            requested_at,
                            request,
                        },
                    ));
                }
                FipsControlFrame::Roster {
                    network_id,
                    roster,
                    signed_roster,
                } => {
                    return Ok(FipsEndpointMessageOutcome::event(
                        FipsPrivateMeshEvent::Roster {
                            sender_pubkey: source_pubkey,
                            network_id,
                            roster,
                            signed_roster,
                        },
                    ));
                }
                FipsControlFrame::Capabilities {
                    network_id,
                    capabilities,
                } => {
                    self.record_peer_capabilities(&source_pubkey, &capabilities, now)?;
                    return Ok(FipsEndpointMessageOutcome::event(
                        FipsPrivateMeshEvent::Capabilities {
                            sender_pubkey: source_pubkey,
                            network_id,
                            capabilities,
                        },
                    ));
                }
                FipsControlFrame::Fragment { .. } => {
                    return Ok(FipsEndpointMessageOutcome::none());
                }
            }
        }

        let data_len = message.data.len();
        let source_node_addr = *message.source_peer.node_addr();
        if let Some(packet) = mesh.receive_endpoint_data_owned_with_source_node_addr(
            source_node_addr.as_bytes(),
            message.data,
        ) {
            let now = now.unwrap_or_else(unix_timestamp);
            #[cfg(feature = "paid-exit")]
            self.note_paid_route_inbound_packet(
                Some(packet.source_pubkey),
                packet.source_pubkey_bytes,
                packet.bytes.as_ref(),
            )?;
            self.note_data_rx(packet.source_pubkey, packet.source_pubkey_bytes, data_len, now)?;
            return Ok(FipsEndpointMessageOutcome::event(
                FipsPrivateMeshEvent::Packet(packet.bytes),
            ));
        }

        Ok(FipsEndpointMessageOutcome::none())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn note_data_rx_batch(
        &self,
        notes: &mut FipsDataRxBatchNotes,
        now: Option<u64>,
    ) -> Result<()> {
        if notes.is_empty() {
            return Ok(());
        }
        let now = now.unwrap_or_else(unix_timestamp);
        let peer_activity = self.peer_activity.load();
        let mut presence_notes = Vec::new();
        for note in notes.drain() {
            if let Some(participant_key) = note.participant_key {
                if let Some(activity) = peer_activity.get(&participant_key) {
                    activity.note_rx(note.bytes, now, FipsPeerRxKind::Data);
                    continue;
                }
                presence_notes.push((hex::encode(participant_key), note.bytes));
                continue;
            }
            if let Some(participant) = note.participant {
                presence_notes.push((participant, note.bytes));
            }
        }
        drop(peer_activity);
        if presence_notes.is_empty() {
            return Ok(());
        }

        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        for (participant, bytes) in presence_notes {
            if let Some(entry) = presence.get_mut(&participant) {
                entry.last_seen_at = Some(now);
                entry.last_data_seen_at = Some(now);
                entry.rx_bytes = entry.rx_bytes.saturating_add(bytes as u64);
                entry.error = None;
            } else {
                presence.insert(
                    participant,
                    FipsPeerPresence {
                        last_seen_at: Some(now),
                        last_data_seen_at: Some(now),
                        rx_bytes: bytes as u64,
                        error: None,
                        ..Default::default()
                    },
                );
            }
        }
        Ok(())
    }

    fn decode_endpoint_control_frame(
        &self,
        message: &FipsEndpointMessage,
    ) -> Result<Option<FipsControlFrame>> {
        let Some(frame) = decode_fips_control_frame(&message.data)? else {
            return Ok(None);
        };
        let FipsControlFrame::Fragment {
            id,
            index,
            total,
            data,
        } = frame
        else {
            return Ok(Some(frame));
        };

        let source_key = *message.source_peer.node_addr().as_bytes();
        let Some(reassembled) = self
            .control_fragments
            .lock()
            .map_err(|_| anyhow!("FIPS control fragment buffer lock poisoned"))?
            .push(source_key, id, index, total, data, unix_timestamp())?
        else {
            return Ok(None);
        };
        decode_fips_control_frame(&reassembled)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn drain_direct_endpoint_mesh_events(&self, limit: usize) -> Result<usize> {
        let mut events = Vec::new();
        while events.len() < limit {
            let runs = match self.direct_endpoint_rx.try_recv() {
                Ok(runs) => runs,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(0),
            };
            self.direct_endpoint_packet_runs_to_mesh_events(
                runs,
                Some(unix_timestamp()),
                &mut events,
            )
            .await?;
        }

        let drained = events.len();
        if drained > 0 {
            self.direct_endpoint_pending_events
                .lock()
                .map_err(|_| anyhow!("FIPS direct endpoint event queue lock poisoned"))?
                .extend(events);
        }
        Ok(drained)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn direct_endpoint_packet_runs_to_mesh_events(
        &self,
        runs: Vec<FipsEndpointDirectPacketRun>,
        now: Option<u64>,
        events: &mut Vec<FipsPrivateMeshEvent>,
    ) -> Result<()> {
        for run in runs {
            let source_peer = *run.source_peer();
            let enqueued_at_ms = run.enqueued_at_ms();
            for packet in run.packet_slices() {
                let message = FipsEndpointMessage {
                    source_peer,
                    data: FipsEndpointData::from(packet.to_vec()),
                    enqueued_at_ms,
                };
                if let Some(event) = self.endpoint_message_to_mesh_event(message, now).await? {
                    events.push(event);
                }
            }
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn pop_direct_endpoint_mesh_event(&self) -> Result<Option<FipsPrivateMeshEvent>> {
        Ok(self
            .direct_endpoint_pending_events
            .lock()
            .map_err(|_| anyhow!("FIPS direct endpoint event queue lock poisoned"))?
            .pop_front())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn pop_direct_endpoint_mesh_events_into(
        &self,
        events: &mut Vec<FipsPrivateMeshEvent>,
        limit: usize,
    ) -> Result<usize> {
        let mut pending = self
            .direct_endpoint_pending_events
            .lock()
            .map_err(|_| anyhow!("FIPS direct endpoint event queue lock poisoned"))?;
        while events.len() < limit {
            let Some(event) = pending.pop_front() else {
                break;
            };
            events.push(event);
        }
        Ok(events.len())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn push_direct_endpoint_mesh_event(&self, event: FipsPrivateMeshEvent) -> Result<()> {
        self.direct_endpoint_pending_events
            .lock()
            .map_err(|_| anyhow!("FIPS direct endpoint event queue lock poisoned"))?
            .push_back(event);
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn forward_direct_endpoint_control_events_blocking(
        &self,
        runs: &[FipsEndpointDirectPacketRun],
        event_tx: Option<&mpsc::Sender<FipsPrivateMeshEvent>>,
    ) -> Result<()> {
        let now = Some(unix_timestamp());
        for run in runs {
            let source_peer = *run.source_peer();
            let enqueued_at_ms = run.enqueued_at_ms();
            for packet in run.packet_slices() {
                if decode_fips_control_frame(packet)?.is_none() {
                    continue;
                }
                let message = FipsEndpointMessage {
                    source_peer,
                    data: FipsEndpointData::from(packet.to_vec()),
                    enqueued_at_ms,
                };
                let outcome = self.endpoint_message_to_mesh_event_outcome(message, now)?;
                if let Some(reply) = outcome.reply
                    && let Err(error) = self.endpoint.blocking_send_to_peer(reply.peer, reply.data)
                {
                    eprintln!("fips: failed to reply to peer ping: {error}");
                }
                let Some(event) = outcome.event else {
                    continue;
                };
                if let Some(event_tx) = event_tx {
                    if event_tx.blocking_send(event).is_err() {
                        return Ok(());
                    }
                } else {
                    self.push_direct_endpoint_mesh_event(event)?;
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn recv_tunnel_packet(&self) -> Result<Option<Vec<u8>>> {
        loop {
            match self.recv_mesh_event().await? {
                Some(FipsPrivateMeshEvent::Packet(packet)) => return Ok(Some(packet.into_vec())),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct DirectEndpointPacketRunAdmission {
    received: usize,
    accepted: usize,
    data_rx_notes: FipsDataRxBatchNotes,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn admit_direct_endpoint_packet_runs_with_mesh(
    mesh: &FipsMeshRuntime,
    runs: Vec<FipsEndpointDirectPacketRun>,
    batch_outputs: &mut DirectTunWriteBatch,
) -> DirectEndpointPacketRunAdmission {
    let mut current_source_node_addr = None;
    let mut current_admitter = None;
    let mut received = 0usize;
    let mut accepted = 0usize;
    let mut data_rx_notes = FipsDataRxBatchNotes::default();
    for run in runs {
        let run_packets = run.len();
        received = received.saturating_add(run_packets);
        if run_packets == 0 {
            continue;
        }

        let source_node_addr = *run.source_node_addr().as_bytes();
        if current_source_node_addr != Some(source_node_addr) {
            current_source_node_addr = Some(source_node_addr);
            current_admitter = mesh.endpoint_source_admitter(&source_node_addr);
        }
        let Some(admitter) = current_admitter else {
            continue;
        };

        let (accepted_count, endpoint_bytes) =
            admit_direct_endpoint_packet_run_with_admitter(&admitter, run, batch_outputs);
        if accepted_count == 0 {
            continue;
        }

        accepted = accepted.saturating_add(accepted_count);
        data_rx_notes.push(FipsDataRxNote::new(
            admitter.source_pubkey(),
            admitter.source_pubkey_bytes(),
            endpoint_bytes,
        ));
    }
    DirectEndpointPacketRunAdmission {
        received,
        accepted,
        data_rx_notes,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn revalidate_direct_endpoint_tun_batch_with_mesh(
    mesh: &FipsMeshRuntime,
    mesh_generation: u64,
    batch_outputs: &mut DirectTunWriteBatch,
) {
    let runs = std::mem::take(&mut batch_outputs.runs);
    batch_outputs.clear();
    batch_outputs.set_mesh_generation(mesh_generation);
    let mut admitted = admit_direct_endpoint_packet_runs_with_mesh(mesh, runs, batch_outputs);
    batch_outputs.append_data_rx_notes(&mut admitted.data_rx_notes);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn admit_direct_endpoint_packet_run_with_admitter(
    admitter: &FipsEndpointSourceAdmitter<'_>,
    mut run: FipsEndpointDirectPacketRun,
    batch_outputs: &mut DirectTunWriteBatch,
) -> (usize, usize) {
    let mut admission_cache = FipsEndpointAdmissionCache::default();
    let mut accepted_count = 0usize;
    let mut endpoint_bytes = 0usize;
    run.retain_packets(|_index, packet| {
        if !admitter.admit_packet_cached(packet, &mut admission_cache) {
            return false;
        }
        accepted_count = accepted_count.saturating_add(1);
        endpoint_bytes = endpoint_bytes.saturating_add(packet.len());
        true
    });
    if accepted_count == 0 {
        return (0, 0);
    }

    if fips_unix_packet_debug_enabled() {
        for packet in run.packet_slices() {
            eprintln!(
                "fips: mesh -> TUN {} bytes {}",
                packet.len(),
                describe_ip_packet(packet)
            );
        }
    }
    batch_outputs.push_run(
        run,
        FipsPacketSource::new(admitter.source_pubkey_bytes()),
    );
    (accepted_count, endpoint_bytes)
}
