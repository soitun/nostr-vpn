#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FipsPeerAddressHint {
    addr: String,
    seen_at_ms: Option<u64>,
    #[serde(default = "default_fips_peer_address_priority")]
    priority: u8,
}

const FIPS_STATIC_PEER_ENDPOINT_PRIORITY: u8 = 10;
const FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY: u8 = 100;
const FIPS_PRIVATE_PEER_ENDPOINT_PRIORITY: u8 = 200;
const FIPS_ROSTER_AUTO_RECONNECT: bool = true;
const FIPS_TRANSIT_AUTO_RECONNECT: bool = false;

fn default_fips_peer_address_priority() -> u8 {
    FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY
}

#[derive(Debug, Clone, Default)]
struct MobilePeerPresence {
    last_seen_at: Option<u64>,
    last_control_seen_at: Option<u64>,
    last_data_seen_at: Option<u64>,
    last_ping_sent_at: Option<u64>,
    last_ping_started_at: Option<Instant>,
    rtt_ms: Option<u64>,
    tx_bytes: u64,
    rx_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
enum MobilePeerRxKind {
    Control,
    Data,
}

fn mobile_timestamp_within_grace(now: u64, timestamp: u64, grace_secs: u64) -> bool {
    if timestamp > now {
        return timestamp - now <= MOBILE_PEER_MAX_FUTURE_SKEW_SECS;
    }
    now - timestamp <= grace_secs
}

fn credible_mobile_timestamp(now: u64, timestamp: Option<u64>) -> Option<u64> {
    let timestamp = timestamp?;
    if timestamp > now && timestamp - now > MOBILE_PEER_MAX_FUTURE_SKEW_SECS {
        return None;
    }
    Some(timestamp)
}

fn mobile_elapsed_at_least(now: u64, timestamp: u64, interval_secs: u64) -> bool {
    if timestamp > now {
        return timestamp - now > MOBILE_PEER_MAX_FUTURE_SKEW_SECS;
    }
    now - timestamp >= interval_secs
}

#[allow(clippy::too_many_arguments)]
async fn handle_mobile_endpoint_message(
    endpoint: &FipsEndpoint,
    mesh: &Arc<RwLock<FipsMeshRuntime>>,
    mesh_peers: &Arc<RwLock<Vec<FipsMeshPeerConfig>>>,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    peer_hints: &Arc<RwLock<HashMap<String, Vec<FipsPeerAddressHint>>>>,
    presence: &Arc<RwLock<HashMap<String, MobilePeerPresence>>>,
    config_state: &Arc<RwLock<MobileTunnelConfig>>,
    app_config: &Arc<RwLock<AppConfig>>,
    app_config_dirty: &AtomicBool,
    config_path: Option<&Path>,
    network_id: &str,
    join_request_active: &AtomicBool,
    control_fragments: &mut FipsControlFragmentBuffer,
    inbound_tx: &mpsc::SyncSender<Vec<u8>>,
    message: FipsEndpointMessage,
) -> Result<bool> {
    if handle_mobile_control_frame(
        endpoint,
        mesh,
        mesh_peers,
        peer_identities,
        peer_hints,
        presence,
        config_state,
        app_config,
        app_config_dirty,
        config_path,
        network_id,
        join_request_active,
        control_fragments,
        &message,
    )
    .await?
    {
        return Ok(true);
    }

    let source_node_addr = *message.source_peer.node_addr();
    let message_len = message.data.len();
    let packet = mesh.read().ok().and_then(|mesh| {
        mesh.receive_endpoint_data_owned_from_node_addr(
            source_node_addr.as_bytes(),
            message.data.into_vec(),
        )
    });
    if let Some(packet) = packet {
        note_mobile_peer_rx(
            presence,
            &packet.source_pubkey,
            message_len,
            MobilePeerRxKind::Data,
        );
        let mut bytes = packet.bytes;
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut bytes);
        if inbound_tx.send(bytes).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn handle_mobile_control_frame(
    endpoint: &FipsEndpoint,
    mesh: &Arc<RwLock<FipsMeshRuntime>>,
    mesh_peers: &Arc<RwLock<Vec<FipsMeshPeerConfig>>>,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    peer_hints: &Arc<RwLock<HashMap<String, Vec<FipsPeerAddressHint>>>>,
    presence: &Arc<RwLock<HashMap<String, MobilePeerPresence>>>,
    config_state: &Arc<RwLock<MobileTunnelConfig>>,
    app_config: &Arc<RwLock<AppConfig>>,
    app_config_dirty: &AtomicBool,
    config_path: Option<&Path>,
    network_id: &str,
    join_request_active: &AtomicBool,
    control_fragments: &mut FipsControlFragmentBuffer,
    message: &FipsEndpointMessage,
) -> Result<bool> {
    let Some(frame) = decode_mobile_control_frame(control_fragments, message)? else {
        return Ok(false);
    };
    if !control_frame_network_matches(network_id, &frame) {
        return Ok(true);
    }
    let Some(source_pubkey) = mobile_control_source_pubkey(mesh, message.source_peer, &frame)?
    else {
        return Ok(true);
    };
    note_mobile_peer_rx(
        presence,
        &source_pubkey,
        message.data.len(),
        MobilePeerRxKind::Control,
    );

    match frame {
        FipsControlFrame::Roster { signed_roster, .. } => {
            apply_mobile_roster_frame(
                endpoint,
                mesh,
                mesh_peers,
                peer_identities,
                peer_hints,
                config_state,
                app_config,
                app_config_dirty,
                config_path,
                join_request_active,
                signed_roster.as_deref(),
            )
            .await?;
        }
        FipsControlFrame::Capabilities { capabilities, .. } => {
            if update_mobile_peer_hints(peer_hints, &source_pubkey, &capabilities)? {
                sync_mobile_config_peer_hints(config_state, peer_hints)?;
                persist_mobile_peer_hints(
                    app_config,
                    app_config_dirty,
                    config_path,
                    &source_pubkey,
                    &capabilities,
                )?;
                refresh_mobile_endpoint_peers(endpoint, mesh_peers, peer_hints, config_state)
                    .await?;
            }
        }
        FipsControlFrame::Ping {
            network_id,
            sent_at,
        } => {
            reply_mobile_ping(endpoint, message.source_peer, network_id, sent_at).await?;
        }
        FipsControlFrame::JoinRequest {
            requested_at,
            request,
        } => {
            record_mobile_join_request(
                app_config,
                app_config_dirty,
                config_path,
                &source_pubkey,
                requested_at,
                &request,
            )?;
        }
        FipsControlFrame::Pong { sent_at, .. } => {
            note_mobile_peer_pong(presence, &source_pubkey, sent_at);
        }
        FipsControlFrame::Fragment { .. } => {}
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn apply_mobile_roster_frame(
    endpoint: &FipsEndpoint,
    mesh: &Arc<RwLock<FipsMeshRuntime>>,
    mesh_peers: &Arc<RwLock<Vec<FipsMeshPeerConfig>>>,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    peer_hints: &Arc<RwLock<HashMap<String, Vec<FipsPeerAddressHint>>>>,
    config_state: &Arc<RwLock<MobileTunnelConfig>>,
    app_config: &Arc<RwLock<AppConfig>>,
    app_config_dirty: &AtomicBool,
    config_path: Option<&Path>,
    join_request_active: &AtomicBool,
    signed_roster: Option<&SignedRoster>,
) -> Result<()> {
    let Some(updated) =
        apply_mobile_roster(app_config, app_config_dirty, config_path, signed_roster)?
    else {
        return Ok(());
    };
    let local_routes = vec![updated.local_address.clone()];
    let updated_peers = updated.peers.clone();
    let updated_peer_identities = mobile_peer_identity_map(&updated_peers);
    let updated_hints = updated.peer_hints.clone();
    {
        let mut mesh = mesh
            .write()
            .map_err(|_| anyhow!("mobile FIPS mesh route table lock poisoned"))?;
        *mesh = FipsMeshRuntime::with_local_routes(updated_peers.clone(), local_routes);
    }
    {
        let mut peers = mesh_peers
            .write()
            .map_err(|_| anyhow!("mobile FIPS peer lock poisoned"))?;
        *peers = updated_peers;
    }
    {
        let mut identities = peer_identities
            .write()
            .map_err(|_| anyhow!("mobile FIPS peer identity lock poisoned"))?;
        *identities = updated_peer_identities;
    }
    {
        let mut hints = peer_hints
            .write()
            .map_err(|_| anyhow!("mobile FIPS peer hint lock poisoned"))?;
        *hints = updated_hints;
    }
    {
        let mut config = config_state
            .write()
            .map_err(|_| anyhow!("mobile FIPS config lock poisoned"))?;
        *config = updated.clone();
    }
    if updated.pending_join_request_recipient.trim().is_empty() {
        join_request_active.store(false, Ordering::Relaxed);
    }
    refresh_mobile_endpoint_peers(endpoint, mesh_peers, peer_hints, config_state).await
}

async fn reply_mobile_ping(
    endpoint: &FipsEndpoint,
    source_peer: PeerIdentity,
    network_id: String,
    sent_at: u64,
) -> Result<()> {
    let reply = FipsControlFrame::Pong {
        network_id,
        sent_at,
        replied_at: unix_timestamp(),
    };
    let encoded = encode_fips_control_frame(&reply)?;
    let _ = endpoint.send_to_peer(source_peer, encoded).await;
    Ok(())
}

fn mobile_control_source_pubkey(
    mesh: &Arc<RwLock<FipsMeshRuntime>>,
    source_peer: PeerIdentity,
    frame: &FipsControlFrame,
) -> Result<Option<String>> {
    let mesh = mesh
        .read()
        .map_err(|_| anyhow!("mobile FIPS mesh route table lock poisoned"))?;
    Ok(control_frame_source_pubkey(&mesh, source_peer, frame))
}

fn decode_mobile_control_frame(
    control_fragments: &mut FipsControlFragmentBuffer,
    message: &FipsEndpointMessage,
) -> Result<Option<FipsControlFrame>> {
    let Some(frame) = decode_fips_control_frame(&message.data)? else {
        return Ok(None);
    };
    let FipsControlFrame::Fragment { .. } = frame else {
        return Ok(Some(frame));
    };
    let source_key = endpoint_source_key(message.source_peer);
    control_fragments.decode(&source_key, &message.data, unix_timestamp())
}

fn control_frame_network_matches(expected_network_id: &str, frame: &FipsControlFrame) -> bool {
    let frame_network_id = match frame {
        FipsControlFrame::Ping { network_id, .. }
        | FipsControlFrame::Pong { network_id, .. }
        | FipsControlFrame::Roster { network_id, .. }
        | FipsControlFrame::Capabilities { network_id, .. } => network_id,
        FipsControlFrame::JoinRequest { request, .. } => &request.network_id,
        FipsControlFrame::Fragment { .. } => return false,
    };
    normalize_runtime_network_id(expected_network_id)
        == normalize_runtime_network_id(frame_network_id)
}

fn control_frame_source_pubkey(
    mesh: &FipsMeshRuntime,
    source_peer: PeerIdentity,
    frame: &FipsControlFrame,
) -> Option<String> {
    mesh.participant_for_endpoint_node_addr(source_peer.node_addr().as_bytes())
        .or_else(|| {
            matches!(frame, FipsControlFrame::JoinRequest { .. })
                .then(|| source_peer.pubkey().to_string())
        })
}

fn endpoint_source_key(source_peer: PeerIdentity) -> String {
    source_peer.node_addr().to_string()
}

#[derive(Debug, Clone, Default)]
struct MobilePeerIdentityMap {
    by_participant: HashMap<MobileParticipantPubkeyBytes, PeerIdentity>,
    by_endpoint_node_addr: HashMap<[u8; 16], PeerIdentity>,
}

type MobileParticipantPubkeyBytes = [u8; 32];

impl MobilePeerIdentityMap {
    fn identity_for_send(
        &self,
        participant: Option<&MobileParticipantPubkeyBytes>,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<PeerIdentity> {
        self.by_endpoint_node_addr
            .get(endpoint_node_addr)
            .or_else(|| participant.and_then(|pubkey| self.by_participant.get(pubkey)))
            .copied()
    }

    fn identity_for_participant(&self, participant: &str) -> Option<PeerIdentity> {
        let participant = mobile_participant_pubkey_bytes(participant)?;
        self.identity_for_participant_bytes(&participant)
    }

    fn identity_for_participant_bytes(
        &self,
        participant: &MobileParticipantPubkeyBytes,
    ) -> Option<PeerIdentity> {
        self.by_participant.get(participant).copied()
    }
}

fn mobile_peer_identity_map(peers: &[FipsMeshPeerConfig]) -> MobilePeerIdentityMap {
    let mut identities = MobilePeerIdentityMap::default();
    for peer in peers {
        let endpoint_npub = normalize_mobile_endpoint_npub(&peer.endpoint_npub);
        let Ok(identity) = PeerIdentity::from_npub(&endpoint_npub) else {
            continue;
        };

        if let Some(participant) = mobile_participant_pubkey_bytes(&peer.participant_pubkey) {
            identities.by_participant.insert(participant, identity);
        }
        identities
            .by_endpoint_node_addr
            .insert(*identity.node_addr().as_bytes(), identity);
    }
    identities
}

fn mobile_identity_for_send(
    identities: &MobilePeerIdentityMap,
    participant: Option<&MobileParticipantPubkeyBytes>,
    endpoint_node_addr: &[u8; 16],
) -> Option<PeerIdentity> {
    identities.identity_for_send(participant, endpoint_node_addr)
}

fn mobile_participant_pubkey_bytes(value: &str) -> Option<MobileParticipantPubkeyBytes> {
    PublicKey::parse(value.trim())
        .ok()
        .map(|pubkey| *pubkey.as_bytes())
}

fn normalize_mobile_endpoint_npub(value: &str) -> String {
    let trimmed = value.trim();
    normalize_nostr_pubkey(trimmed).ok().map_or_else(
        || trimmed.to_string(),
        |pubkey| nostr_vpn_core::invite::to_npub(&pubkey),
    )
}

enum MobileEndpointSendRun {
    Identity {
        participant_fallback: Option<String>,
        participant_key: Option<MobileParticipantPubkeyBytes>,
        identity: PeerIdentity,
        payloads: Vec<Vec<u8>>,
    },
}

fn mobile_endpoint_send_run_matches(
    current_identity: PeerIdentity,
    current_participant_key: Option<MobileParticipantPubkeyBytes>,
    current_participant_fallback: Option<&str>,
    identity: PeerIdentity,
    participant_key: Option<MobileParticipantPubkeyBytes>,
    participant_fallback: Option<&str>,
) -> bool {
    if current_identity != identity {
        return false;
    }
    match (current_participant_key, participant_key) {
        (Some(left), Some(right)) => left == right,
        (None, None) => current_participant_fallback == participant_fallback,
        _ => false,
    }
}

fn drain_mobile_outbound_ready(
    outbound_rx: &mut tokio_mpsc::Receiver<Vec<u8>>,
    packets: &mut Vec<Vec<u8>>,
    first: Vec<u8>,
) {
    packets.clear();
    packets.push(first);
    while packets.len() < MOBILE_FIPS_SEND_BATCH {
        match outbound_rx.try_recv() {
            Ok(packet) => packets.push(packet),
            Err(
                tokio_mpsc::error::TryRecvError::Empty
                | tokio_mpsc::error::TryRecvError::Disconnected,
            ) => break,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_mobile_outbound_packets(
    endpoint: &FipsEndpoint,
    mesh: &Arc<RwLock<FipsMeshRuntime>>,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    wg_send_tx: Option<&tokio_mpsc::Sender<Vec<u8>>>,
    wg_addr: Option<Ipv4Addr>,
    mesh_addr: Option<Ipv4Addr>,
    inbound_tx_for_dns: &mpsc::SyncSender<Vec<u8>>,
    app_config_for_dns: &Arc<RwLock<AppConfig>>,
    dns_forwarders: &[SocketAddr],
    outbound_count: &mut u32,
    packets: &mut Vec<Vec<u8>>,
) -> bool {
    let mut pending_run = None;
    let packet_count = packets.len();
    for index in 0..packet_count {
        let packet = std::mem::take(&mut packets[index]);
        // Local MagicDNS responder. The well-known DNS address is owned by this
        // tunnel instance, so answer before mesh/WG routing and never treat it
        // as a remote nvpn node.
        if let Some(response) =
            mobile_magic_dns_response_packet(&packet, app_config_for_dns, dns_forwarders).await
        {
            flush_mobile_endpoint_send_run(endpoint, &mut pending_run).await;
            if inbound_tx_for_dns.send(response).is_err() {
                packets.clear();
                return false;
            }
            continue;
        }

        let outgoing_peer = mesh.read().ok().and_then(|mesh| {
            mesh.route_outbound_packet_peer(&packet).map(|peer| {
                (
                    peer.participant_pubkey_bytes
                        .is_none()
                        .then(|| peer.participant_pubkey.to_string()),
                    peer.participant_pubkey_bytes.copied(),
                    *peer.endpoint_node_addr,
                )
            })
        });
        if let Some((participant_fallback, participant_key, endpoint_node_addr)) = outgoing_peer {
            if let Some(run) = push_mobile_endpoint_send_run(
                &mut pending_run,
                peer_identities,
                participant_fallback,
                participant_key,
                endpoint_node_addr,
                packet,
            ) {
                send_mobile_endpoint_run(endpoint, run).await;
            }
            continue;
        }

        flush_mobile_endpoint_send_run(endpoint, &mut pending_run).await;
        if let Some(wg_tx) = wg_send_tx {
            dispatch_mobile_wg_packet(wg_tx, packet, wg_addr, mesh_addr, outbound_count);
        }
    }
    packets.clear();
    flush_mobile_endpoint_send_run(endpoint, &mut pending_run).await;
    true
}

fn push_mobile_endpoint_send_run(
    run: &mut Option<MobileEndpointSendRun>,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    participant_fallback: Option<String>,
    participant_key: Option<MobileParticipantPubkeyBytes>,
    endpoint_node_addr: [u8; 16],
    packet: Vec<u8>,
) -> Option<MobileEndpointSendRun> {
    let identity = peer_identities.read().ok().and_then(|identities| {
        mobile_identity_for_send(&identities, participant_key.as_ref(), &endpoint_node_addr)
    });
    let Some(identity) = identity else {
        return run.take();
    };

    if let Some(MobileEndpointSendRun::Identity {
        participant_fallback: current_participant_fallback,
        participant_key: current_participant_key,
        identity: current_identity,
        payloads,
    }) = run.as_mut()
        && mobile_endpoint_send_run_matches(
            *current_identity,
            *current_participant_key,
            current_participant_fallback.as_deref(),
            identity,
            participant_key,
            participant_fallback.as_deref(),
        )
    {
        payloads.push(packet);
        return None;
    }

    let previous = run.take();
    *run = Some(MobileEndpointSendRun::Identity {
        participant_fallback,
        participant_key,
        identity,
        payloads: vec![packet],
    });
    previous
}

async fn flush_mobile_endpoint_send_run(
    endpoint: &FipsEndpoint,
    run: &mut Option<MobileEndpointSendRun>,
) {
    if let Some(run) = run.take() {
        send_mobile_endpoint_run(endpoint, run).await;
    }
}

async fn send_mobile_endpoint_run(endpoint: &FipsEndpoint, run: MobileEndpointSendRun) {
    match run {
        MobileEndpointSendRun::Identity {
            identity, payloads, ..
        } => {
            let _ = endpoint.send_batch_to_peer(identity, payloads).await;
        }
    }
}

fn dispatch_mobile_wg_packet(
    wg_tx: &tokio_mpsc::Sender<Vec<u8>>,
    mut packet: Vec<u8>,
    wg_addr: Option<Ipv4Addr>,
    mesh_addr: Option<Ipv4Addr>,
    outbound_count: &mut u32,
) {
    // No matching mesh peer route: hand the plaintext off to the WG runtime,
    // which will boringtun-encapsulate and send out via the upstream UDP
    // socket. SNAT first so the inner source IP matches the WG peer's
    // configured address; Mullvad/Proton silently drop other source IPs.
    let len_before = packet.len();
    let pre_log = if *outbound_count <= 10 && packet.len() >= 20 && packet[0] >> 4 == 4 {
        *outbound_count = (*outbound_count).saturating_add(1);
        let proto = packet[9];
        let src_before = format!(
            "{}.{}.{}.{}",
            packet[12], packet[13], packet[14], packet[15]
        );
        let dst = format!(
            "{}.{}.{}.{}",
            packet[16], packet[17], packet[18], packet[19]
        );
        Some((proto, src_before, dst))
    } else {
        None
    };
    if let (Some(wg), Some(mesh)) = (wg_addr, mesh_addr) {
        rewrite_ipv4_source(&mut packet, mesh, wg);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut packet);
    }
    if let Some((proto, src_before, dst)) = pre_log {
        let src_after = format!(
            "{}.{}.{}.{}",
            packet[12], packet[13], packet[14], packet[15]
        );
        log_pump_packet(&format!(
            "outbound #{} {len_before}B proto={proto} src={src_before}->{src_after} dst={dst}",
            *outbound_count
        ));
    }
    let _ = wg_tx.try_send(packet);
}

async fn send_mobile_endpoint_data(
    endpoint: &FipsEndpoint,
    peer_identities: &Arc<RwLock<MobilePeerIdentityMap>>,
    participant: &str,
    data: Vec<u8>,
) -> Result<()> {
    let participant_key = mobile_participant_pubkey_bytes(participant);
    let identity =
        peer_identities
            .read()
            .ok()
            .and_then(|identities| match participant_key.as_ref() {
                Some(participant) => identities.identity_for_participant_bytes(participant),
                None => identities.identity_for_participant(participant),
            });
    let identity = identity.ok_or_else(|| {
        anyhow!("missing mobile FIPS endpoint identity for participant {participant}")
    })?;
    endpoint
        .send_to_peer(identity, data)
        .await
        .context("failed to send mobile FIPS endpoint data")
}
