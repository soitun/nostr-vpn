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
        Self::bind_with_config(identity_nsec, scope, peers, config, Vec::new(), Vec::new()).await
    }

    async fn bind_with_config(
        identity_nsec: impl Into<String>,
        scope: impl Into<String>,
        peers: Vec<FipsMeshPeerConfig>,
        config: Config,
        local_allowed_ips: Vec<String>,
        paid_route_admissions: Vec<FipsPaidRouteAdmission>,
    ) -> Result<Self> {
        let scope = scope.into();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (direct_endpoint_sink, direct_endpoint_rx) = {
            // This queue carries authenticated packet-runs; live IP policy is applied on drain.
            let lane_count = FIPS_DIRECT_ENDPOINT_RX_LANES.max(1);
            let mut lanes = Vec::with_capacity(lane_count);
            let mut receivers = Vec::with_capacity(lane_count);
            for _ in 0..lane_count {
                let lane = Arc::new(FipsDirectEndpointDataLane::new());
                lanes.push(Arc::clone(&lane));
                receivers.push(FipsDirectEndpointDataRx::new(lane));
            }
            (FipsDirectEndpointDataSink { lanes }, Some(receivers))
        };

        let endpoint_builder = FipsEndpoint::builder()
            .config(config)
            .identity_nsec(identity_nsec)
            .discovery_scope(scope)
            .without_system_tun();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let endpoint = endpoint_builder
            .bind_with_direct_sink(direct_endpoint_sink)
            .await
            .context("failed to bind embedded FIPS endpoint")?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let endpoint = endpoint_builder
            .bind()
            .await
            .context("failed to bind embedded FIPS endpoint")?;
        let peer_identities = peer_identity_map(&peers);
        let mesh = FipsMeshRuntime::with_local_routes_and_paid_route_admissions(
            peers,
            local_allowed_ips,
            paid_route_admissions,
        );
        let peer_activity = peer_activity_map(&mesh.peer_pubkeys(), None);

        Ok(Self {
            endpoint,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            direct_endpoint_rx,
            mesh: ArcSwap::from_pointee(mesh),
            mesh_generation: AtomicU64::new(0),
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
            let participant_key = outgoing.participant_pubkey_bytes.copied();
            Self::push_endpoint_send_run(
                &mut runs,
                &peer_identities,
                outgoing.participant_pubkey,
                participant_key,
                outgoing.endpoint_node_addr,
                outgoing.bytes,
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
        let input_packets = packets.len();
        let mut runs = Vec::new();
        self.send_tun_pipeline_packet_turn(
            packets.drain(..),
            input_packets,
            input_packets,
            &mut runs,
        )
        .await
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn send_tun_pipeline_packet_turn<I>(
        &self,
        packets: I,
        input_packets: usize,
        turn_capacity: usize,
        runs: &mut Vec<FipsEndpointSendRun>,
    ) -> Result<usize>
    where
        I: IntoIterator<Item = TunPipelinePacket>,
    {
        self.build_tun_pipeline_send_runs(packets, input_packets, turn_capacity, runs)?;
        let _t =
            crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshEndpointSend);
        self.send_endpoint_send_runs(runs.drain(..)).await
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn blocking_send_tun_pipeline_packet_turn<I>(
        &self,
        packets: I,
        input_packets: usize,
        turn_capacity: usize,
        runs: &mut Vec<FipsEndpointSendRun>,
    ) -> Result<usize>
    where
        I: IntoIterator<Item = TunPipelinePacket>,
    {
        self.build_tun_pipeline_send_runs(packets, input_packets, turn_capacity, runs)?;
        let _t =
            crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshEndpointSend);
        self.blocking_send_endpoint_send_runs(runs.drain(..))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn build_tun_pipeline_send_runs<I>(
        &self,
        packets: I,
        input_packets: usize,
        turn_capacity: usize,
        runs: &mut Vec<FipsEndpointSendRun>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = TunPipelinePacket>,
    {
        if input_packets == 0 {
            return Ok(());
        }

        debug_assert!(runs.is_empty());
        let mesh = self.mesh.load();
        let peer_identities = self.peer_identities.load();
        let mut routed_packets = 0usize;
        let mut cached_destination_peer = None;

        {
            let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshRoute);
            for packet in packets {
                let destination = packet.destination;
                let packet_debug = fips_unix_packet_debug_enabled();
                let debug_description = packet_debug.then(|| describe_ip_packet(&packet.bytes));
                let routed = match destination {
                    Some(destination) => cached_tun_destination_route(
                        &mesh,
                        &mut cached_destination_peer,
                        destination,
                        packet.bytes,
                    ),
                    None => mesh
                        .route_outbound_packet_owned_with_peer(packet.bytes)
                        .map(RoutedTunPipelinePacket::from),
                };
                let Some(routed) = routed else {
                    if let Some(description) = debug_description {
                        eprintln!("fips: TUN packet had no FIPS route {description}");
                    }
                    continue;
                };
                routed_packets += 1;
                if packet_debug {
                    eprintln!(
                        "fips: TUN packet routed to {} {}",
                        routed.participant_pubkey,
                        describe_ip_packet(&routed.bytes)
                    );
                }
                let participant_key = routed.participant_pubkey_bytes;
                Self::push_endpoint_send_run(
                    runs,
                    &peer_identities,
                    routed.participant_pubkey,
                    participant_key,
                    &routed.endpoint_node_addr,
                    routed.bytes,
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
            turn_capacity,
        );
        Ok(())
    }

    fn push_endpoint_send_run(
        runs: &mut Vec<FipsEndpointSendRun>,
        peer_identities: &FipsPeerIdentityMap,
        participant_pubkey: &str,
        participant_key: Option<ParticipantPubkeyBytes>,
        endpoint_node_addr: &[u8; 16],
        payload: Vec<u8>,
    ) {
        let bytes_len = payload.len();

        if let Some(FipsEndpointSendRun::Identity(run)) = runs.last_mut()
            && run.matches_endpoint(endpoint_node_addr, participant_key, participant_pubkey)
        {
            run.bytes_len += bytes_len;
            run.payloads.push(payload);
            return;
        }

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

    async fn send_endpoint_send_runs<I>(&self, runs: I) -> Result<usize>
    where
        I: IntoIterator<Item = FipsEndpointSendRun>,
    {
        let mut sent = 0usize;
        for run in runs {
            match run {
                FipsEndpointSendRun::Identity(run) => {
                    let packet_count = run.payloads.len();
                    self.endpoint
                        .send_batch_to_peer(run.identity, run.payloads)
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn blocking_send_endpoint_send_runs<I>(&self, runs: I) -> Result<usize>
    where
        I: IntoIterator<Item = FipsEndpointSendRun>,
    {
        let mut sent = 0usize;
        for run in runs {
            match run {
                FipsEndpointSendRun::Identity(run) => {
                    let packet_count = run.payloads.len();
                    self.endpoint
                        .blocking_send_batch_to_peer(run.identity, run.payloads)
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

struct RoutedTunPipelinePacket<'a> {
    participant_pubkey: &'a str,
    participant_pubkey_bytes: Option<ParticipantPubkeyBytes>,
    endpoint_node_addr: [u8; 16],
    bytes: Vec<u8>,
}

impl<'a> From<nostr_vpn_core::fips_mesh::RoutedFipsPacket<'a>>
    for RoutedTunPipelinePacket<'a>
{
    fn from(value: nostr_vpn_core::fips_mesh::RoutedFipsPacket<'a>) -> Self {
        Self {
            participant_pubkey: value.participant_pubkey,
            participant_pubkey_bytes: value.participant_pubkey_bytes.copied(),
            endpoint_node_addr: *value.endpoint_node_addr,
            bytes: value.bytes,
        }
    }
}

fn cached_tun_destination_route<'a>(
    mesh: &'a FipsMeshRuntime,
    cached: &mut Option<(IpAddr, RoutedFipsPeer<'a>)>,
    destination: IpAddr,
    bytes: Vec<u8>,
) -> Option<RoutedTunPipelinePacket<'a>> {
    let peer = if let Some((cached_destination, peer)) = cached.as_ref()
        && *cached_destination == destination
    {
        *peer
    } else {
        let peer = mesh.route_outbound_destination_peer(destination)?;
        *cached = Some((destination, peer));
        peer
    };

    Some(RoutedTunPipelinePacket {
        participant_pubkey: peer.participant_pubkey,
        participant_pubkey_bytes: peer.participant_pubkey_bytes.copied(),
        endpoint_node_addr: *peer.endpoint_node_addr,
        bytes,
    })
}
