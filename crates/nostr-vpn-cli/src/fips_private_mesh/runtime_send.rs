impl FipsPrivateMeshRuntime {
    pub(crate) async fn bind(
        identity_nsec: impl Into<String>,
        network_id: impl AsRef<str>,
        peers: Vec<FipsMeshPeerConfig>,
    ) -> Result<Self> {
        let scope = fips_lan_discovery_scope(network_id.as_ref());
        let endpoint_peers = fips_endpoint_peers_from_mesh(&peers, Vec::new(), Vec::new());
        let config = fips_endpoint_config(
            &endpoint_peers,
            None,
            private_mesh_mtu_from_app(None),
            fips_nostr_discovery_policy_from_env(),
        );
        Self::bind_with_config(identity_nsec, scope, peers, config, Vec::new()).await
    }

    async fn bind_with_config(
        identity_nsec: impl Into<String>,
        scope: impl Into<String>,
        peers: Vec<FipsMeshPeerConfig>,
        config: Config,
        local_allowed_ips: Vec<String>,
    ) -> Result<Self> {
        let scope = scope.into();
        let endpoint = FipsEndpoint::builder()
            .config(config)
            .identity_nsec(identity_nsec)
            .discovery_scope(scope)
            .without_system_tun()
            .bind()
            .await
            .context("failed to bind embedded FIPS endpoint")?;
        let peer_identities = peer_identity_map(&peers);
        let mesh = FipsMeshRuntime::with_local_routes(peers, local_allowed_ips);
        let peer_activity = peer_activity_map(&mesh.peer_pubkeys(), None);

        Ok(Self {
            endpoint,
            mesh: ArcSwap::from_pointee(mesh),
            peer_activity: ArcSwap::from_pointee(peer_activity),
            peer_identities: ArcSwap::from_pointee(peer_identities),
            presence: RwLock::new(HashMap::new()),
            link_status: RwLock::new(HashMap::new()),
            other_link_status: RwLock::new(HashMap::new()),
            peer_capabilities: RwLock::new(HashMap::new()),
            control_fragments: Mutex::new(ControlFragmentBuffer::default()),
        })
    }

    pub(crate) fn npub(&self) -> &str {
        self.endpoint.npub()
    }

    pub(crate) async fn send_tunnel_packet(&self, packet: &[u8]) -> Result<bool> {
        let mesh = self.mesh.load();
        let Some(outgoing) = mesh.route_outbound_packet_with_peer(packet) else {
            return Ok(false);
        };
        let bytes_len = outgoing.bytes.len();

        self.send_endpoint_data(
            outgoing.participant_pubkey,
            outgoing.participant_pubkey_bytes,
            outgoing.endpoint_node_addr,
            outgoing.bytes,
        )
        .await
        .context("failed to send private packet over FIPS endpoint data")?;
        self.note_tx(
            Some(outgoing.participant_pubkey),
            outgoing.participant_pubkey_bytes,
            bytes_len,
        )?;
        Ok(true)
    }

    pub(crate) async fn send_tunnel_packet_owned(&self, packet: Vec<u8>) -> Result<bool> {
        let mesh = self.mesh.load();
        let Some(outgoing) = mesh.route_outbound_packet_owned_with_peer(packet) else {
            return Ok(false);
        };
        let bytes_len = outgoing.bytes.len();

        self.send_endpoint_data(
            outgoing.participant_pubkey,
            outgoing.participant_pubkey_bytes,
            outgoing.endpoint_node_addr,
            outgoing.bytes,
        )
        .await
        .context("failed to send private packet over FIPS endpoint data")?;
        self.note_tx(
            Some(outgoing.participant_pubkey),
            outgoing.participant_pubkey_bytes,
            bytes_len,
        )?;
        Ok(true)
    }

    pub(crate) async fn send_tunnel_packet_batch_owned(
        &self,
        packets: Vec<Vec<u8>>,
    ) -> Result<usize> {
        if packets.is_empty() {
            return Ok(0);
        }

        let mesh = self.mesh.load();
        let peer_identities = self.peer_identities.load();
        let mut runs = Vec::new();

        for packet in packets {
            let Some(outgoing) = mesh.route_outbound_packet_owned_with_peer(packet) else {
                continue;
            };
            Self::push_endpoint_send_run(
                &mut runs,
                &peer_identities,
                outgoing.participant_pubkey,
                outgoing.participant_pubkey_bytes.copied(),
                outgoing.endpoint_node_addr,
                FipsEndpointPayload::new(outgoing.bytes),
            );
        }
        drop(peer_identities);
        drop(mesh);

        self.send_endpoint_send_runs(runs).await
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn send_tun_pipeline_packet_batch(
        &self,
        packets: &mut Vec<TunPipelinePacket>,
    ) -> Result<usize> {
        if packets.is_empty() {
            return Ok(0);
        }

        let input_packets = packets.len();
        let mesh = self.mesh.load();
        let peer_identities = self.peer_identities.load();
        let mut runs = Vec::new();
        let mut routed_packets = 0usize;

        {
            let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshRoute);
            for packet in packets.drain(..) {
                let class = packet.class;
                let Some(outgoing) = mesh.route_outbound_packet_owned_with_peer(packet.bytes) else {
                    continue;
                };
                routed_packets += 1;
                let payload = FipsEndpointPayload::from_classified(outgoing.bytes, class);
                Self::push_endpoint_send_run(
                    &mut runs,
                    &peer_identities,
                    outgoing.participant_pubkey,
                    outgoing.participant_pubkey_bytes.copied(),
                    outgoing.endpoint_node_addr,
                    payload,
                );
            }
        }
        drop(peer_identities);
        drop(mesh);

        let run_count = runs.len();
        crate::pipeline_profile::record_mesh_send_batch(
            input_packets,
            routed_packets,
            run_count,
            FIPS_MESH_SEND_BURST,
        );

        let _t =
            crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshEndpointSend);
        self.send_endpoint_send_runs(runs).await
    }

    fn push_endpoint_send_run(
        runs: &mut Vec<FipsEndpointSendRun>,
        peer_identities: &FipsPeerIdentityMap,
        participant_pubkey: &str,
        participant_key: Option<ParticipantPubkeyBytes>,
        endpoint_node_addr: &[u8; 16],
        payload: FipsEndpointPayload,
    ) {
        let bytes_len = payload.len();

        if let Some(identity) = endpoint_identity_for_send(
            peer_identities,
            participant_key.as_ref(),
            endpoint_node_addr,
        ) {
            if let Some(FipsEndpointSendRun::Identity(run)) = runs.last_mut()
                && run.matches(identity, participant_key, participant_pubkey)
            {
                run.bytes_len += bytes_len;
                run.payloads.push(payload);
                return;
            }

            runs.push(FipsEndpointSendRun::Identity(FipsEndpointIdentitySendRun {
                participant_fallback: participant_key
                    .is_none()
                    .then(|| participant_pubkey.to_string()),
                participant_key,
                identity,
                payloads: vec![payload],
                bytes_len,
            }));
        }
    }

    async fn send_endpoint_send_runs(&self, runs: Vec<FipsEndpointSendRun>) -> Result<usize> {
        let mut sent = 0usize;
        for run in runs {
            match run {
                FipsEndpointSendRun::Identity(run) => {
                    let packet_count = run.payloads.len();
                    self.endpoint
                        .send_classified_batch_to_peer(run.identity, run.payloads)
                        .await
                        .with_context(|| {
                            format!(
                                "failed to send {packet_count} private packets over FIPS endpoint data"
                            )
                    })?;
                    self.note_tx(
                        run.participant_fallback.as_deref(),
                        run.participant_key.as_ref(),
                        run.bytes_len,
                    )?;
                    sent += packet_count;
                }
            }
        }

        Ok(sent)
    }

}
