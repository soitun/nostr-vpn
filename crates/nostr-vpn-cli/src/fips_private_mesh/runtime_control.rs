impl FipsPrivateMeshRuntime {
    pub(crate) async fn ping_peers(&self, network_id: &str, now: u64) -> Result<usize> {
        let frame = FipsControlFrame::Ping {
            network_id: network_id.to_string(),
            sent_at: now,
        };
        let participants = self.ping_due_participants(now)?;
        let mut sent = 0usize;
        for participant in participants {
            self.note_ping_attempt(&participant, now)?;
            if self.send_control_frame(&participant, &frame).await.is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    pub(crate) async fn send_join_request(
        &self,
        participant: &str,
        requested_at: u64,
        request: MeshJoinRequest,
    ) -> Result<()> {
        self.send_control_frame(
            participant,
            &FipsControlFrame::JoinRequest {
                requested_at,
                request,
            },
        )
        .await
    }

    pub(crate) async fn send_roster(
        &self,
        participant: &str,
        signed_roster: SignedRoster,
    ) -> Result<()> {
        let network_id = signed_roster.network_id()?;
        let roster = signed_roster.roster()?;
        self.send_control_frame(
            participant,
            &FipsControlFrame::Roster {
                network_id,
                roster,
                signed_roster: Some(Box::new(signed_roster)),
            },
        )
        .await
    }

    pub(crate) async fn send_capabilities(
        &self,
        participant: &str,
        network_id: &str,
        capabilities: PeerCapabilities,
    ) -> Result<()> {
        self.send_control_frame(
            participant,
            &FipsControlFrame::Capabilities {
                network_id: network_id.to_string(),
                capabilities,
            },
        )
        .await
    }

    pub(crate) async fn broadcast_capabilities(
        &self,
        network_id: &str,
        capabilities: PeerCapabilities,
    ) -> Result<usize> {
        let frame = FipsControlFrame::Capabilities {
            network_id: network_id.to_string(),
            capabilities,
        };
        self.broadcast_control_frame(&frame).await
    }

    async fn broadcast_control_frame(&self, frame: &FipsControlFrame) -> Result<usize> {
        let participants = self.mesh.load().peer_pubkeys();
        let mut sent = 0usize;
        for participant in participants {
            if self.send_control_frame(&participant, frame).await.is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }

    async fn send_control_frame(&self, participant: &str, frame: &FipsControlFrame) -> Result<()> {
        let participant_key = participant_pubkey_bytes(participant);
        let destination = {
            let mesh = self.mesh.load();
            let peer_identities = self.peer_identities.load();
            control_frame_destination_peer(&mesh, &peer_identities, participant)?
        };
        let messages = encode_fips_control_messages(frame)?;
        let mut sent_len = 0usize;
        for encoded in messages {
            sent_len += encoded.len();
            self.endpoint
                .send_to_peer(destination, encoded)
                .await
                .with_context(|| format!("failed to send FIPS control frame to {participant}"))?;
        }
        self.note_tx(Some(participant), participant_key.as_ref(), sent_len)?;
        Ok(())
    }

    async fn send_endpoint_data(
        &self,
        participant: &str,
        participant_key: Option<&ParticipantPubkeyBytes>,
        endpoint_node_addr: &[u8; 16],
        data: Vec<u8>,
    ) -> Result<()> {
        let peer_identities = self.peer_identities.load();
        let identity =
            endpoint_identity_for_send(&peer_identities, participant_key, endpoint_node_addr);
        drop(peer_identities);
        let identity = identity.ok_or_else(|| {
            anyhow!(
                "missing FIPS endpoint identity for participant {participant} node_addr {}",
                hex::encode(endpoint_node_addr)
            )
        })?;
        let payload = FipsEndpointPayload::new(data);
        self.endpoint
            .send_classified_batch_to_peer(identity, vec![payload])
            .await
            .context("failed to send private packet over FIPS endpoint data")
    }

    fn note_tx(
        &self,
        participant: Option<&str>,
        participant_key: Option<&ParticipantPubkeyBytes>,
        len: usize,
    ) -> Result<()> {
        // Hot path. Packet movers pass the already-parsed participant key
        // from FipsMeshRuntime, avoiding per-packet pubkey parsing or
        // string-key hashing for configured peers.
        let parsed_participant_key = participant_key
            .is_none()
            .then(|| participant.and_then(participant_pubkey_bytes))
            .flatten();
        let participant_key = participant_key.or(parsed_participant_key.as_ref());
        let peer_activity = self.peer_activity.load();
        if let Some(activity) = participant_key.and_then(|key| peer_activity.get(key)) {
            activity.note_tx(len);
            return Ok(());
        }
        drop(peer_activity);
        let participant = participant
            .map(str::to_owned)
            .or_else(|| participant_key.map(hex::encode))
            .ok_or_else(|| anyhow!("missing FIPS participant identity for tx accounting"))?;
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        if let Some(entry) = presence.get_mut(&participant) {
            entry.tx_bytes = entry.tx_bytes.saturating_add(len as u64);
        } else {
            let entry = FipsPeerPresence {
                tx_bytes: len as u64,
                ..Default::default()
            };
            presence.insert(participant, entry);
        }
        Ok(())
    }

    fn note_ping_attempt(&self, participant: &str, now: u64) -> Result<()> {
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        if let Some(entry) = presence.get_mut(participant) {
            entry.last_ping_sent_at = Some(now);
            entry.last_ping_started_at = Some(Instant::now());
        } else {
            let entry = FipsPeerPresence {
                last_ping_sent_at: Some(now),
                last_ping_started_at: Some(Instant::now()),
                ..Default::default()
            };
            presence.insert(participant.to_string(), entry);
        }
        Ok(())
    }

    fn note_pong(&self, participant: &str, sent_at: u64) -> Result<()> {
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        let Some(entry) = presence.get_mut(participant) else {
            return Ok(());
        };
        if entry.last_ping_sent_at == Some(sent_at)
            && let Some(started_at) = entry.last_ping_started_at.take()
        {
            let elapsed_ms = started_at.elapsed().as_millis();
            if elapsed_ms <= FIPS_CONTROL_RTT_MAX_ACCEPT_MS {
                entry.rtt_ms = Some(elapsed_ms.min(u128::from(u64::MAX)) as u64);
            } else {
                entry.last_ping_sent_at = None;
            }
        }
        Ok(())
    }

    fn note_control_rx(&self, participant: &str, len: usize, now: u64) -> Result<()> {
        self.note_rx(participant, None, len, now, FipsPeerRxKind::Control)
    }

    fn note_data_rx(
        &self,
        participant: &str,
        participant_key: Option<&ParticipantPubkeyBytes>,
        len: usize,
        now: u64,
    ) -> Result<()> {
        self.note_rx(participant, participant_key, len, now, FipsPeerRxKind::Data)
    }

    fn note_rx(
        &self,
        participant: &str,
        participant_key: Option<&ParticipantPubkeyBytes>,
        len: usize,
        now: u64,
        kind: FipsPeerRxKind,
    ) -> Result<()> {
        let parsed_participant_key = participant_key
            .is_none()
            .then(|| participant_pubkey_bytes(participant))
            .flatten();
        let participant_key = participant_key.or(parsed_participant_key.as_ref());
        let peer_activity = self.peer_activity.load();
        if let Some(activity) = participant_key.and_then(|key| peer_activity.get(key)) {
            activity.note_rx(len, now, kind);
            return Ok(());
        }
        drop(peer_activity);
        let mut presence = self
            .presence
            .write()
            .map_err(|_| anyhow!("FIPS mesh presence lock poisoned"))?;
        if let Some(entry) = presence.get_mut(participant) {
            entry.last_seen_at = Some(now);
            match kind {
                FipsPeerRxKind::Control => entry.last_control_seen_at = Some(now),
                FipsPeerRxKind::Data => entry.last_data_seen_at = Some(now),
            }
            entry.rx_bytes = entry.rx_bytes.saturating_add(len as u64);
            entry.error = None;
        } else {
            let mut entry = FipsPeerPresence {
                last_seen_at: Some(now),
                rx_bytes: len as u64,
                error: None,
                ..Default::default()
            };
            match kind {
                FipsPeerRxKind::Control => entry.last_control_seen_at = Some(now),
                FipsPeerRxKind::Data => entry.last_data_seen_at = Some(now),
            }
            presence.insert(participant.to_string(), entry);
        }
        Ok(())
    }

    pub(crate) async fn shutdown(self) -> Result<(), FipsEndpointError> {
        self.endpoint.shutdown().await
    }

    /// Hand the latest peer roster to fips without restarting the endpoint.
    ///
    /// The wrapper translates nvpn's intermediate hint shape
    /// ([`FipsEndpointPeerTransportConfig`]) into `fips_endpoint::PeerConfig`
    /// (carrying `seen_at_ms` per address) and calls
    /// [`fips_endpoint::FipsEndpoint::update_peers`]. fips diffs new vs old,
    /// initiates connections for fresh npubs, drops retry entries for
    /// removed ones, and refreshes address hints in place for the rest.
    pub(crate) async fn update_peers(
        &self,
        endpoint_peers: &[FipsEndpointPeerTransportConfig],
    ) -> Result<fips_endpoint::UpdatePeersOutcome> {
        let peers: Vec<FipsPeerConfig> = endpoint_peers
            .iter()
            .map(|peer| FipsPeerConfig {
                npub: peer.npub.clone(),
                alias: None,
                addresses: peer
                    .addresses
                    .iter()
                    .map(fips_peer_address_from_hint)
                    .collect(),
                connect_policy: ConnectPolicy::AutoConnect,
                auto_reconnect: peer.auto_reconnect,
                discovery_fallback_transit: peer.discovery_fallback_transit,
            })
            .collect();
        self.endpoint
            .update_peers(peers)
            .await
            .context("fips: update_peers rejected by endpoint")
    }

    pub(crate) async fn refresh_peer_paths(
        &self,
        endpoint_peers: &[FipsEndpointPeerTransportConfig],
    ) -> Result<usize> {
        let peers = endpoint_peers
            .iter()
            .map(|peer| {
                PeerIdentity::from_npub(&peer.npub)
                    .with_context(|| format!("invalid FIPS endpoint peer npub {}", peer.npub))
            })
            .collect::<Result<Vec<_>>>()?;
        self.endpoint
            .refresh_peer_paths(peers)
            .await
            .context("fips: refresh_peer_paths rejected by endpoint")
    }
}
