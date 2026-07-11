impl FipsPrivateMeshRuntime {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn recv_direct_endpoint_tun_batch_blocking(
        &self,
        stop: &AtomicBool,
        packet_outputs: &mut DirectTunWriteBatch,
        event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    ) -> Result<Option<usize>> {
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }

            let mut emitted = 0usize;
            let mut received = 0usize;
            let mut turn_start = None;
            let mut mesh = None;
            while received < FIPS_MESH_RECV_BURST {
                let runs = if received == 0 {
                    loop {
                        if stop.load(Ordering::Acquire) {
                            return Ok(None);
                        }
                        match self.direct_endpoint_rx.recv_source_batch_timeout(
                            Duration::from_millis(100),
                            FIPS_MESH_RECV_BURST,
                        ) {
                            Ok(runs) => break runs,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                return Ok(None);
                            }
                        }
                    }
                } else {
                    match self
                        .direct_endpoint_rx
                        .try_recv_limited(FIPS_MESH_RECV_BURST.saturating_sub(received))
                    {
                        Ok(runs) => runs,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(None),
                    }
                };

                if turn_start.is_none() {
                    turn_start = crate::pipeline_profile::stamp();
                }
                let mesh = mesh.get_or_insert_with(|| self.stable_mesh_snapshot().1);
                packet_outputs.set_admission_mesh(Arc::clone(mesh));

                let (received_packets, accepted_packets) = self
                    .admit_direct_endpoint_packet_runs_blocking(
                    mesh.as_ref(),
                    runs,
                    packet_outputs,
                    event_tx,
                )?;
                received = received.saturating_add(received_packets);
                if accepted_packets > 0 {
                    emitted = emitted.saturating_add(accepted_packets);
                }
                if should_flush_direct_endpoint_tun_batch_early(packet_outputs) {
                    break;
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

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn wake_blocking_mesh_recv(&self) {
        self.direct_endpoint_rx.interrupt();
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn finalize_direct_endpoint_tun_batch_blocking(
        &self,
        packet_outputs: &DirectTunWriteBatch,
    ) -> Result<()> {
        let _t = crate::pipeline_profile::Timer::start(
            crate::pipeline_profile::Stage::DirectEndpointFinalize,
        );
        if packet_outputs.is_empty() {
            return Ok(());
        }
        let mesh = packet_outputs
            .admission_mesh()
            .ok_or_else(|| anyhow!("FIPS direct TUN batch missing admission snapshot"))?;
        #[cfg(feature = "paid-exit")]
        self.note_paid_route_inbound_batch(mesh, packet_outputs)?;
        self.note_data_rx_batch(mesh, &packet_outputs.runs, None)
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
                    network_id: _,
                    roster: _,
                    signed_roster,
                } => {
                    return Ok(FipsEndpointMessageOutcome::event(
                        FipsPrivateMeshEvent::Roster {
                            sender_pubkey: source_pubkey,
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

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn note_data_rx_batch(
        &self,
        mesh: &FipsMeshRuntime,
        runs: &[FipsEndpointDirectPacketRun],
        now: Option<u64>,
    ) -> Result<()> {
        if runs.is_empty() {
            return Ok(());
        }
        let now = now.unwrap_or_else(unix_timestamp);
        let peer_activity = self.peer_activity.load();
        let mut presence_needed = false;
        for run in runs {
            let admitter = direct_run_admitter(mesh, run)?;
            if let Some(activity) = admitter
                .source_pubkey_bytes()
                .and_then(|key| peer_activity.get(key))
            {
                activity.note_rx(run.packet_bytes(), now, FipsPeerRxKind::Data);
            } else {
                presence_needed = true;
            }
        }
        if !presence_needed {
            return Ok(());
        }

        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        for run in runs {
            let admitter = direct_run_admitter(mesh, run)?;
            if admitter
                .source_pubkey_bytes()
                .is_some_and(|key| peer_activity.contains_key(key))
            {
                continue;
            }
            let participant = admitter.source_pubkey();
            let bytes = run.packet_bytes();
            if let Some(entry) = presence.get_mut(participant) {
                entry.last_seen_at = Some(now);
                entry.last_data_seen_at = Some(now);
                entry.rx_bytes = entry.rx_bytes.saturating_add(bytes as u64);
                entry.error = None;
            } else {
                presence.insert(
                    participant.to_string(),
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
        let Some(frame) = decode_fips_control_frame(message.data.as_slice())? else {
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

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn admit_direct_endpoint_packet_runs_blocking(
        &self,
        mesh: &FipsMeshRuntime,
        runs: Vec<FipsEndpointDirectPacketRun>,
        batch_outputs: &mut DirectTunWriteBatch,
        event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    ) -> Result<(usize, usize)> {
        let mut current_source_node_addr = None;
        let mut current_admitter = None;
        let mut current_admission_cache = FipsEndpointAdmissionCache::default();
        let mut received = 0usize;
        let mut accepted = 0usize;
        let mut control_events_open = true;
        for run in runs {
            received = received.saturating_add(run.len());
            let source_node_addr = *run.source_peer().node_addr().as_bytes();
            if current_source_node_addr != Some(source_node_addr) {
                current_source_node_addr = Some(source_node_addr);
                current_admitter = mesh.endpoint_source_admitter(&source_node_addr);
                current_admission_cache = FipsEndpointAdmissionCache::default();
            }
            let accepted_count = self.admit_direct_endpoint_packet_run_blocking(
                current_admitter.as_ref(),
                run,
                &mut current_admission_cache,
                batch_outputs,
                event_tx,
                &mut control_events_open,
            )?;
            if accepted_count == 0 {
                continue;
            }
            accepted = accepted.saturating_add(accepted_count);
        }
        Ok((received, accepted))
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn admit_direct_endpoint_packet_run_blocking(
        &self,
        admitter: Option<&FipsEndpointSourceAdmitter<'_>>,
        mut run: FipsEndpointDirectPacketRun,
        admission_cache: &mut FipsEndpointAdmissionCache,
        batch_outputs: &mut DirectTunWriteBatch,
        event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
        control_events_open: &mut bool,
    ) -> Result<usize> {
        let source_peer = *run.source_peer();
        let enqueued_at_ms = run.enqueued_at_ms();
        let mut control_now = None;
        let mut control_error = None;
        run.retain_packets(|_index, packet| {
            if control_error.is_some() {
                return false;
            }
            if is_fips_control_frame(packet) {
                match decode_fips_control_frame(packet) {
                    Ok(Some(_frame)) => {
                        if *control_events_open {
                            let message = FipsEndpointMessage {
                                source_peer,
                                data: FipsEndpointData::new(packet.to_vec()),
                                enqueued_at_ms,
                            };
                            let now = Some(*control_now.get_or_insert_with(unix_timestamp));
                            match self.endpoint_message_to_mesh_event_outcome(message, now) {
                                Ok(outcome) => {
                                    if let Some(reply) = outcome.reply
                                        && let Err(error) = self
                                            .endpoint
                                            .blocking_send_batch_to_peer(reply.peer, vec![reply.data])
                                    {
                                        eprintln!("fips: failed to reply to peer ping: {error}");
                                    }
                                    if let Some(event) = outcome.event
                                        && event_tx.blocking_send(event).is_err()
                                    {
                                        *control_events_open = false;
                                    }
                                }
                                Err(error) => control_error = Some(error),
                            }
                        }
                        return false;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        control_error = Some(error);
                        return false;
                    }
                }
            }

            let Some(admitter) = admitter else {
                return false;
            };
            if !admitter.admit_packet_cached(packet, admission_cache) {
                return false;
            }
            true
        });
        if let Some(error) = control_error {
            return Err(error);
        }
        let accepted_count = run.len();
        if accepted_count == 0 {
            return Ok(0);
        }

        if fips_tun_packet_debug_enabled() {
            for packet in run.packet_slices() {
                eprintln!(
                    "fips: mesh -> TUN {} bytes {}",
                    packet.len(),
                    describe_ip_packet(packet)
                );
            }
        }
        batch_outputs.push_run(run);
        Ok(accepted_count)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn direct_run_admitter<'a>(
    mesh: &'a FipsMeshRuntime,
    run: &FipsEndpointDirectPacketRun,
) -> Result<FipsEndpointSourceAdmitter<'a>> {
    let source_node_addr = run.source_peer().node_addr().as_bytes();
    mesh.endpoint_source_admitter(source_node_addr)
        .ok_or_else(|| anyhow!("accepted FIPS direct packet run lost admission source"))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn should_flush_direct_endpoint_tun_batch_early(packet_outputs: &DirectTunWriteBatch) -> bool {
    const LATENCY_BATCH_PACKETS: usize = 16;
    const LATENCY_PACKET_BYTES: usize = 256;

    packet_outputs.len() >= LATENCY_BATCH_PACKETS
        && packet_outputs.bytes() <= packet_outputs.len().saturating_mul(LATENCY_PACKET_BYTES)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_tun_packet_debug_enabled() -> bool {
    fips_unix_packet_debug_enabled()
}

#[cfg(target_os = "windows")]
fn fips_tun_packet_debug_enabled() -> bool {
    windows_fips_packet_debug_enabled()
}
