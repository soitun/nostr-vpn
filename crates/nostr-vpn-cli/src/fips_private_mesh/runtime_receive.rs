impl FipsPrivateMeshRuntime {
    pub(crate) async fn recv_mesh_event(&self) -> Result<Option<FipsPrivateMeshEvent>> {
        loop {
            let Some(message) = self.endpoint.recv().await else {
                return Ok(None);
            };

            if let Some(event) = self.endpoint_message_to_mesh_event(message, None).await? {
                return Ok(Some(event));
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
            let Some(_) = self.endpoint.recv_batch_into(messages, limit).await else {
                return Ok(None);
            };
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
    fn recv_mesh_event_batch_blocking_into(
        &self,
        events: &mut Vec<FipsPrivateMeshEvent>,
        limit: usize,
        stop: &AtomicBool,
    ) -> Result<Option<usize>> {
        events.clear();
        self.recv_mesh_event_batch_blocking_for_each(limit, stop, |event| {
            events.push(event);
            true
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn recv_mesh_event_batch_blocking_for_each(
        &self,
        limit: usize,
        stop: &AtomicBool,
        mut handle_event: impl FnMut(FipsPrivateMeshEvent) -> bool,
    ) -> Result<Option<usize>> {
        let limit = limit.clamp(1, FIPS_MESH_EVENT_DRAIN_LIMIT);
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(None);
            }

            let now = Some(unix_timestamp());
            let mut events_drained = 0;
            let mut handler_stopped = false;
            let mut receive_error = None;
            let Some(_) = self
                .endpoint
                .blocking_recv_batch_for_each(limit, |message| {
                    if stop.load(Ordering::Acquire) {
                        handler_stopped = true;
                        return false;
                    }

                    match self.endpoint_message_to_mesh_event_blocking(message, now) {
                        Ok(Some(event)) => {
                            events_drained += 1;
                            if handle_event(event) {
                                true
                            } else {
                                handler_stopped = true;
                                false
                            }
                        }
                        Ok(None) => true,
                        Err(error) => {
                            receive_error = Some(error);
                            false
                        }
                    }
                })
            else {
                return Ok(None);
            };

            if let Some(error) = receive_error {
                return Err(error);
            }
            if handler_stopped || stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            if events_drained > 0 {
                return Ok(Some(events_drained));
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn wake_blocking_mesh_recv(&self) {
        let npub = self.endpoint.npub().to_string();
        let _ = self.endpoint.blocking_send(npub, Vec::new());
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

    fn endpoint_message_to_mesh_event_blocking(
        &self,
        message: FipsEndpointMessage,
        now: Option<u64>,
    ) -> Result<Option<FipsPrivateMeshEvent>> {
        let outcome = self.endpoint_message_to_mesh_event_outcome(message, now)?;
        if let Some(reply) = outcome.reply
            && let Err(error) = self.endpoint.blocking_send_to_peer(reply.peer, reply.data)
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
        if let Some(frame) = self.decode_endpoint_control_frame(&message)? {
            let source_pubkey = {
                let mesh = self.mesh.load();
                control_frame_source_pubkey(&mesh, message.source_peer, &frame)
            };
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
        let mesh = self.mesh.load();
        if let Some(packet) = mesh.receive_endpoint_data_owned_with_source_node_addr(
            source_node_addr.as_bytes(),
            message.data,
        ) {
            let now = now.unwrap_or_else(unix_timestamp);
            self.note_data_rx(
                packet.source_pubkey,
                packet.source_pubkey_bytes,
                data_len,
                now,
            )?;
            return Ok(FipsEndpointMessageOutcome::event(
                FipsPrivateMeshEvent::Packet(packet.bytes),
            ));
        }

        Ok(FipsEndpointMessageOutcome::none())
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

    #[cfg(test)]
    pub(crate) async fn recv_tunnel_packet(&self) -> Result<Option<Vec<u8>>> {
        loop {
            match self.recv_mesh_event().await? {
                Some(FipsPrivateMeshEvent::Packet(packet)) => return Ok(Some(packet)),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

}
