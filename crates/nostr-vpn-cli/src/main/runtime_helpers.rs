impl CliTunnelRuntime {
    fn new(iface: impl Into<String>) -> Self {
        Self {
            iface: iface.into(),
            active_listen_port: None,
        }
    }

    fn stop(&mut self) {
        self.active_listen_port = None;
    }

    #[cfg(target_os = "macos")]
    fn macos_network_cleanup_state(&self) -> Option<MacosNetworkCleanupState> {
        None
    }

    fn listen_port(&self, configured: u16) -> u16 {
        self.active_listen_port.unwrap_or(configured)
    }

    pub(crate) fn owns_interface(&self, iface: &str) -> bool {
        self.iface == iface
    }
}

fn endpoint_with_listen_port(endpoint: &str, listen_port: u16) -> String {
    let trimmed = endpoint.trim();
    if let Ok(mut parsed) = trimmed.parse::<SocketAddr>() {
        parsed.set_port(listen_port);
        return parsed.to_string();
    }
    let Some((host, port)) = trimmed.rsplit_once(':') else {
        return trimmed.to_string();
    };
    if host.is_empty() || host.contains(':') || port.trim().parse::<u16>().is_err() {
        return trimmed.to_string();
    }
    format!("{}:{listen_port}", host.trim())
}

fn detect_runtime_primary_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}

fn endpoint_prefers_runtime_local_ipv4(endpoint: &str) -> bool {
    let value = endpoint.trim();
    if value.is_empty() {
        return true;
    }

    let host = value
        .rsplit_once(':')
        .map_or(value, |(host, _port)| host)
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ipv4_is_local_only(ip),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

fn runtime_local_signal_endpoint(
    endpoint: &str,
    listen_port: u16,
    detected_ipv4: Option<Ipv4Addr>,
) -> String {
    if endpoint_prefers_runtime_local_ipv4(endpoint)
        && let Some(ip) = detected_ipv4
    {
        return SocketAddrV4::new(ip, listen_port).to_string();
    }

    endpoint_with_listen_port(endpoint, listen_port)
}

fn runtime_signal_ipv4(detected_ipv4: Option<Ipv4Addr>, tunnel_ip: &str) -> Option<Ipv4Addr> {
    let tunnel_ipv4 = strip_cidr(tunnel_ip).parse::<Ipv4Addr>().ok();
    detected_ipv4.filter(|ip| Some(*ip) != tunnel_ipv4)
}

fn local_signal_endpoint(app: &AppConfig, listen_port: u16) -> String {
    let detected_ipv4 = if endpoint_prefers_runtime_local_ipv4(&app.node.endpoint) {
        detect_runtime_primary_ipv4()
    } else {
        None
    };
    runtime_local_signal_endpoint(
        &app.node.endpoint,
        listen_port,
        runtime_signal_ipv4(detected_ipv4, &app.node.tunnel_ip),
    )
}

async fn refresh_port_mapping(
    app: &AppConfig,
    network_snapshot: &diagnostics::NetworkSnapshot,
    listen_port: u16,
    port_mapping_runtime: &mut PortMappingRuntime,
) {
    if !app.nat.enabled {
        port_mapping_runtime.stop().await;
        return;
    }

    let timeout = Duration::from_secs(app.nat.discovery_timeout_secs.max(1));
    if let Err(error) = port_mapping_runtime
        .refresh(network_snapshot, listen_port, timeout)
        .await
    {
        eprintln!("nat: port mapping refresh failed: {error}");
    }
}

fn network_probe_timeout(app: &AppConfig) -> Duration {
    Duration::from_secs(app.nat.discovery_timeout_secs.max(2))
}

fn parse_exit_node_arg(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "off" | "none" | "disable" | "disabled" | "clear"
        )
    {
        return Ok(None);
    }

    normalize_nostr_pubkey(value).map(Some)
}

#[cfg(target_os = "linux")]
fn is_exit_node_route(route: &str) -> bool {
    route == "0.0.0.0/0" || route == "::/0"
}

#[cfg(target_os = "linux")]
fn route_is_host_route(route: &str) -> bool {
    let Some((host, bits)) = route.split_once('/') else {
        return true;
    };
    let Ok(bits) = bits.parse::<u8>() else {
        return false;
    };

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => bits == 32,
        Ok(IpAddr::V6(_)) => bits == 128,
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn route_targets_require_endpoint_bypass(route_targets: &[String]) -> bool {
    route_targets
        .iter()
        .any(|route| !route_is_host_route(route))
}

fn daemon_vpn_active(vpn_enabled: bool, expected_peers: usize) -> bool {
    vpn_enabled && expected_peers > 0
}

fn daemon_start_vpn_enabled(app: &AppConfig, paused: bool) -> bool {
    app.autoconnect && !paused
}

fn fips_host_runtime_active(app: &AppConfig, vpn_enabled: bool) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        vpn_enabled && app.fips_host_tunnel_enabled
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (app, vpn_enabled);
        false
    }
}

fn fips_private_runtime_active(app: &AppConfig, vpn_enabled: bool, expected_peers: usize) -> bool {
    daemon_vpn_active(vpn_enabled, expected_peers)
        || fips_host_runtime_active(app, vpn_enabled)
        || app.join_requests_enabled()
        || app
            .active_network_opt()
            .and_then(|network| network.outbound_join_request.as_ref())
            .is_some()
        || app.has_fips_static_peer_endpoints()
}

fn daemon_vpn_idle_status(
    vpn_enabled: bool,
    expected_peers: usize,
    join_requests_active: bool,
) -> &'static str {
    if vpn_enabled && expected_peers == 0 {
        WAITING_FOR_PARTICIPANTS_STATUS
    } else if join_requests_active {
        LISTENING_FOR_JOIN_REQUESTS_STATUS
    } else {
        "Paused"
    }
}

#[derive(Clone, Copy, Debug)]
struct WallTimeJumpObserver {
    wall_observed_at: u64,
    monotonic_observed_at: Instant,
}

impl WallTimeJumpObserver {
    fn new(wall_observed_at: u64) -> Self {
        Self {
            wall_observed_at,
            monotonic_observed_at: Instant::now(),
        }
    }
}

fn wall_time_jump_detected(
    previous_wall_observed_at: u64,
    now_wall: u64,
    previous_monotonic_observed_at: Instant,
    now_monotonic: Instant,
    threshold_secs: u64,
) -> bool {
    if previous_wall_observed_at == 0 || threshold_secs == 0 {
        return false;
    }

    let wall_elapsed = now_wall.saturating_sub(previous_wall_observed_at);
    if wall_elapsed < threshold_secs {
        return false;
    }

    let monotonic_elapsed = now_monotonic
        .saturating_duration_since(previous_monotonic_observed_at)
        .as_secs();
    wall_elapsed.saturating_sub(monotonic_elapsed) >= threshold_secs
}

fn observe_wall_time_jump(
    last_observed_at: &mut WallTimeJumpObserver,
    now_wall: u64,
    now_monotonic: Instant,
    threshold_secs: u64,
) -> bool {
    let jumped = wall_time_jump_detected(
        last_observed_at.wall_observed_at,
        now_wall,
        last_observed_at.monotonic_observed_at,
        now_monotonic,
        threshold_secs,
    );
    last_observed_at.wall_observed_at = now_wall;
    last_observed_at.monotonic_observed_at = now_monotonic;
    jumped
}

#[allow(clippy::too_many_arguments)]
fn persist_inbound_join_request(
    app: &mut AppConfig,
    config_path: &Path,
    sender_pubkey: &str,
    requested_at: u64,
    network_id: &str,
    invite_secret: &str,
    requester_node_name: &str,
    vpn_status: &mut String,
) {
    match app.record_inbound_join_request(
        network_id,
        invite_secret,
        sender_pubkey,
        requester_node_name,
        requested_at,
    ) {
        Ok(Some(network_name)) => {
            if let Err(error) = app.save(config_path) {
                *vpn_status = format!("Failed to persist join request: {error}");
            } else {
                *vpn_status = format!("Join request received for {network_name}.");
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("daemon: ignoring invalid join request from {sender_pubkey}: {error}");
        }
    }
}

fn persist_shared_network_roster(
    app: &mut AppConfig,
    config_path: &Path,
    signed_roster: Option<&SignedRoster>,
    vpn_status: &mut String,
) -> Result<Option<String>> {
    let signed_roster =
        signed_roster.ok_or_else(|| anyhow!("FIPS roster frame is missing signed roster event"))?;
    let changed = app.apply_verified_admin_signed_shared_roster(signed_roster)?;
    let apply_network_id = signed_roster.network_id()?;
    if signed_roster_is_current_for_app(app, &apply_network_id, signed_roster) {
        upsert_signed_roster(
            &signed_rosters_file_path(config_path),
            signed_roster.clone(),
        )?;
    }
    if !changed {
        return Ok(None);
    }

    maybe_autoconfigure_node(app);
    app.save(config_path)?;
    let network_name = app
        .networks
        .iter()
        .find(|network| {
            normalize_runtime_network_id(&network.network_id)
                == normalize_runtime_network_id(&apply_network_id)
        })
        .map(|network| network.name.clone())
        .unwrap_or_else(|| apply_network_id.to_string());
    *vpn_status = format!("Roster updated for {network_name}.");
    Ok(Some(network_name))
}

#[cfg(feature = "embedded-fips")]
fn drain_fips_mesh_events(
    runtime: &mut crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &mut AppConfig,
    config_path: &Path,
    vpn_status: &mut String,
) -> Result<bool> {
    let mut roster_changed = false;
    for event in runtime.drain_events() {
        match event {
            crate::fips_private_mesh::FipsPrivateMeshEvent::Packet(packet) => {
                let _ = packet;
            }
            crate::fips_private_mesh::FipsPrivateMeshEvent::Presence {
                participant_pubkey,
                last_seen_at,
            } => {
                let _ = (participant_pubkey, last_seen_at);
            }
            crate::fips_private_mesh::FipsPrivateMeshEvent::JoinRequest {
                sender_pubkey,
                requested_at,
                request,
            } => {
                persist_inbound_join_request(
                    app,
                    config_path,
                    &sender_pubkey,
                    requested_at,
                    &request.network_id,
                    &request.invite_secret,
                    &request.requester_node_name,
                    vpn_status,
                );
            }
            crate::fips_private_mesh::FipsPrivateMeshEvent::Roster {
                sender_pubkey,
                signed_roster,
                ..
            } => match persist_shared_network_roster(
                app,
                config_path,
                signed_roster.as_deref(),
                vpn_status,
            ) {
                Ok(Some(_)) => roster_changed = true,
                Ok(None) => {}
                Err(error) => {
                    eprintln!("daemon: ignoring invalid FIPS roster from {sender_pubkey}: {error}");
                }
            },
            crate::fips_private_mesh::FipsPrivateMeshEvent::Capabilities {
                sender_pubkey,
                network_id,
                capabilities,
            } => {
                let _ = (sender_pubkey, network_id, capabilities);
                // The FIPS receive path records capabilities before queuing
                // this event. The daemon heartbeat applies fresh endpoint
                // hints with a signature guard, and the status tick reads
                // advertised routes from the same cache, so a capability frame
                // should not force a full FIPS config refresh by itself.
            }
        }
    }
    Ok(roster_changed)
}

#[cfg(feature = "embedded-fips")]
async fn refresh_fips_tunnel_config(
    runtime: &mut crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    network_id: &str,
    own_pubkey: Option<&str>,
) -> Result<()> {
    let config = fips_tunnel_config_from_app(
        app,
        config_path,
        network_id,
        runtime.iface().to_string(),
        own_pubkey,
        None,
        &runtime.peer_endpoint_hints(),
    )?;
    runtime.apply_config(config).await
}

#[cfg(feature = "embedded-fips")]
fn fips_tunnel_config_from_app(
    app: &AppConfig,
    config_path: &Path,
    network_id: &str,
    iface: impl Into<String>,
    own_pubkey: Option<&str>,
    recent_peers: Option<&nostr_vpn_core::recent_peers::RecentPeerEndpoints>,
    live_peer_endpoints: &[(String, Vec<(String, u64)>)],
) -> Result<crate::fips_private_mesh::FipsPrivateTunnelConfig> {
    let mut config = crate::fips_private_mesh::FipsPrivateTunnelConfig::from_app(
        app,
        network_id,
        iface,
        own_pubkey,
        recent_peers,
        live_peer_endpoints,
    )?;
    #[cfg(feature = "paid-exit")]
    {
        config.paid_exit = app.paid_exit.clone();
        config.paid_route_store_path = paid_route_store_file_path(config_path);
        config.paid_route_wallet_data_dir = paid_exit_wallet_data_dir(config_path);
        config.paid_route_payment_relays = paid_exit_relay_urls(app, &[]);
        config.paid_route_admissions = fips_paid_route_admissions_from_store(app, config_path)?;
    }
    #[cfg(not(feature = "paid-exit"))]
    let _ = config_path;
    // Daemon no longer pre-discovers a public endpoint. fips-core's
    // build_overlay_advert performs its own STUN observation and advertises
    // <reflexive_ip>:<listen_port> directly; if that's wrong (e.g. symmetric
    // NAT), peers fall back to udp:nat traversal via Nostr signaling. Use
    // any operator-configured endpoint as a hint when set.
    let configured = endpoint_with_listen_port(&app.node.endpoint, config.listen_port);
    config.advertised_endpoint = if endpoint_is_local_only(&configured) {
        String::new()
    } else {
        configured
    };
    Ok(config)
}

#[cfg(all(feature = "embedded-fips", feature = "paid-exit"))]
fn fips_paid_route_admissions_from_store(
    app: &AppConfig,
    config_path: &Path,
) -> Result<Vec<FipsPaidRouteAdmission>> {
    if !app.paid_exit.enabled {
        return Ok(Vec::new());
    }
    let network_id = app.effective_network_id();
    let store_path = paid_route_store_file_path(config_path);
    let store = load_paid_route_store(&store_path)?;
    Ok(store
        .seller_admissions(&app.paid_exit, unix_timestamp())
        .into_iter()
        .map(|admission| {
            crate::fips_private_mesh::fips_paid_route_admission_from_seller_admission(
                &network_id,
                admission,
            )
        })
        .collect())
}

#[cfg(feature = "embedded-fips")]
struct SyncFipsPrivateRuntimeContext<'a> {
    app: &'a AppConfig,
    config_path: &'a Path,
    network_id: &'a str,
    iface: &'a str,
    own_pubkey: Option<&'a str>,
    vpn_enabled: bool,
    expected_peers: usize,
}

#[cfg(feature = "embedded-fips")]
async fn sync_fips_private_runtime(
    runtime: &mut Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
    context: SyncFipsPrivateRuntimeContext<'_>,
) -> Result<()> {
    if !fips_private_runtime_active(context.app, context.vpn_enabled, context.expected_peers) {
        if let Some(runtime) = runtime.take() {
            runtime.stop().await?;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if !context.app.fips_host_tunnel_enabled {
            crate::fips_host_tunnel::FipsHostTunnelRuntime::cleanup_disabled_artifacts();
        }
        return Ok(());
    }

    let config_iface = runtime
        .as_ref()
        .map(|runtime| runtime.iface().to_string())
        .unwrap_or_else(|| context.iface.to_string());
    let live_peer_endpoints = runtime
        .as_ref()
        .map(|runtime| runtime.peer_endpoint_hints())
        .unwrap_or_default();
    let config = fips_tunnel_config_from_app(
        context.app,
        context.config_path,
        context.network_id,
        config_iface,
        context.own_pubkey,
        None,
        &live_peer_endpoints,
    )?;

    let restart = runtime
        .as_ref()
        .is_some_and(|existing| existing.requires_endpoint_restart(&config));
    if restart {
        if let Some(existing) = runtime.take() {
            existing.stop().await?;
        }
        let started = crate::fips_private_mesh::FipsPrivateTunnelRuntime::start(config).await?;
        eprintln!("daemon: restarted FIPS private mesh on {}", started.iface());
        *runtime = Some(started);
    } else if let Some(existing) = runtime.as_mut() {
        existing.apply_config(config).await?;
    } else {
        let started = crate::fips_private_mesh::FipsPrivateTunnelRuntime::start(config).await?;
        eprintln!("daemon: FIPS private mesh on {}", started.iface());
        *runtime = Some(started);
    }
    Ok(())
}

#[cfg(feature = "embedded-fips")]
async fn send_pending_fips_join_requests(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    sent_cache: &mut HashMap<String, u64>,
    now: u64,
) -> Result<usize> {
    let Some(network) = app.active_network_opt() else {
        return Ok(0);
    };
    let Some(pending) = network.outbound_join_request.as_ref() else {
        return Ok(0);
    };
    let recipients = pending_fips_join_request_recipients(app);
    if recipients.is_empty() {
        return Ok(0);
    }
    let request = MeshJoinRequest {
        network_id: normalize_runtime_network_id(&network.network_id),
        invite_secret: network.invite_secret.clone(),
        requester_node_name: app.node_name.trim().to_string(),
    };

    let mut sent = 0usize;
    for recipient in recipients {
        let fingerprint = format!(
            "{}:{recipient}:{}",
            request.network_id, pending.requested_at
        );
        if sent_cache
            .get(&fingerprint)
            .is_some_and(|last_sent| now.saturating_sub(*last_sent) < FIPS_JOIN_REQUEST_RETRY_SECS)
        {
            continue;
        }
        runtime
            .send_join_request(&recipient, pending.requested_at, request.clone())
            .await?;
        sent_cache.insert(fingerprint, now);
        sent += 1;
    }
    Ok(sent)
}

fn pending_fips_join_request_recipients(app: &AppConfig) -> Vec<String> {
    let Some(network) = app.active_network_opt() else {
        return Vec::new();
    };
    let Some(pending) = network.outbound_join_request.as_ref() else {
        return Vec::new();
    };
    let own_pubkey = app.own_nostr_pubkey_hex().ok();
    let mut recipients = if network
        .admins
        .iter()
        .any(|admin| admin == &pending.recipient)
    {
        vec![pending.recipient.clone()]
    } else {
        network.admins.clone()
    };
    recipients.retain(|recipient| own_pubkey.as_deref() != Some(recipient.as_str()));
    recipients.sort();
    recipients.dedup();
    recipients
}

#[cfg(feature = "embedded-fips")]
async fn publish_fips_active_network_roster(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    pending_recipients: &mut HashSet<String>,
) -> Result<usize> {
    publish_fips_active_network_roster_to(runtime, app, config_path, &[], pending_recipients).await
}

#[cfg(feature = "embedded-fips")]
async fn broadcast_local_fips_capabilities(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
) -> Result<usize> {
    let Some(network) = app.active_network_opt() else {
        return Ok(0);
    };
    let advertised_routes = runtime_effective_advertised_routes(app);
    let local_ipv4_candidates =
        runtime_signal_ipv4_candidates(detect_runtime_primary_ipv4(), &app.node.tunnel_ip);
    let endpoint_hints = local_fips_endpoint_hints(app, local_ipv4_candidates);
    let desired_hint_recipients = desired_fips_endpoint_hint_recipients(app);
    let signed_at = unix_timestamp();
    let mut sent = 0usize;

    for participant in runtime.peer_pubkeys() {
        let capabilities = PeerCapabilities {
            advertised_routes: advertised_routes.clone(),
            endpoint_hints: if desired_hint_recipients.contains(&participant) {
                endpoint_hints.clone()
            } else {
                Vec::new()
            },
            dataplane_features: local_fips_dataplane_features(),
            signed_at,
        };
        if runtime
            .send_capabilities(&participant, &network.id, capabilities)
            .await
            .is_ok()
        {
            sent += 1;
        }
    }

    Ok(sent)
}

#[cfg(feature = "embedded-fips")]
fn local_fips_endpoint_hints(
    app: &AppConfig,
    local_ipv4_candidates: Vec<Ipv4Addr>,
) -> Vec<PeerEndpointHint> {
    let mut endpoints = Vec::new();

    let configured = endpoint_with_listen_port(&app.node.endpoint, app.node.listen_port);
    if endpoint_is_gossipable_direct_hint(&configured, true)
        && !endpoint_uses_tunnel_ip(&configured, &app.node.tunnel_ip)
    {
        endpoints.push(configured);
    }

    if app.lan_discovery_enabled {
        for ip in local_ipv4_candidates {
            if !ipv4_is_lan_endpoint_hint(ip) {
                continue;
            }
            endpoints.push(SocketAddrV4::new(ip, app.node.listen_port).to_string());
        }
    }

    endpoints.sort();
    endpoints.dedup();
    endpoints.into_iter().map(PeerEndpointHint::udp).collect()
}

#[cfg(feature = "embedded-fips")]
fn runtime_signal_ipv4_candidates(
    detected_ipv4: Option<Ipv4Addr>,
    tunnel_ip: &str,
) -> Vec<Ipv4Addr> {
    let tunnel_ipv4 = strip_cidr(tunnel_ip).parse::<Ipv4Addr>().ok();
    let mut ips = Vec::new();
    if let Some(ip) = runtime_signal_ipv4(detected_ipv4, tunnel_ip)
        && ipv4_is_lan_endpoint_hint(ip)
    {
        ips.push(ip);
    }
    for iface in netdev::get_interfaces() {
        if iface.is_loopback() {
            continue;
        }
        for net in &iface.ipv4 {
            let ip = net.addr();
            if Some(ip) == tunnel_ipv4
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_link_local()
                || !ipv4_is_lan_endpoint_hint(ip)
            {
                continue;
            }
            ips.push(ip);
        }
    }
    ips.sort();
    ips.dedup();
    ips
}

#[cfg(feature = "embedded-fips")]
fn endpoint_is_gossipable_direct_hint(endpoint: &str, allow_local: bool) -> bool {
    let trimmed = endpoint.trim();
    if let Ok(parsed) = trimmed.parse::<SocketAddr>() {
        if parsed.port() == 0 || endpoint_hint_ip_is_unusable(parsed.ip()) {
            return false;
        }
        return allow_local || !endpoint_is_local_only(&parsed.to_string());
    }

    let Some((host, port)) = trimmed.rsplit_once(':') else {
        return false;
    };
    let host = host.trim();
    let Ok(port) = port.trim().parse::<u16>() else {
        return false;
    };
    if host.is_empty() || port == 0 || host.eq_ignore_ascii_case("localhost") {
        return false;
    }
    if host.contains(':') {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && endpoint_hint_ip_is_unusable(ip)
    {
        return false;
    }
    allow_local || !endpoint_is_local_only(trimmed)
}

#[cfg(feature = "embedded-fips")]
fn endpoint_uses_tunnel_ip(endpoint: &str, tunnel_ip: &str) -> bool {
    let Ok(tunnel_ip) = strip_cidr(tunnel_ip).parse::<IpAddr>() else {
        return false;
    };
    endpoint_addr_ip(endpoint).is_some_and(|ip| ip == tunnel_ip)
}

#[cfg(feature = "embedded-fips")]
fn endpoint_addr_ip(endpoint: &str) -> Option<IpAddr> {
    let trimmed = endpoint.trim();
    if let Ok(parsed) = trimmed.parse::<SocketAddr>() {
        return Some(parsed.ip());
    }

    let (host, _) = trimmed.rsplit_once(':')?;
    host.trim().parse::<IpAddr>().ok()
}

#[cfg(feature = "embedded-fips")]
fn endpoint_hint_ip_is_unusable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ipv4_is_cgnat(ip)
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

#[cfg(feature = "embedded-fips")]
fn ipv4_is_lan_endpoint_hint(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

#[cfg(feature = "embedded-fips")]
fn ipv4_is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

#[cfg(feature = "embedded-fips")]
fn desired_fips_endpoint_hint_recipients(app: &AppConfig) -> HashSet<String> {
    let own_pubkey = app.own_nostr_pubkey_hex().ok();
    app.participant_pubkeys_hex()
        .into_iter()
        .filter(|participant| own_pubkey.as_deref() != Some(participant.as_str()))
        .collect()
}

#[cfg(feature = "embedded-fips")]
async fn publish_fips_active_network_roster_to(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    extra_recipients: &[String],
    pending_recipients: &mut HashSet<String>,
) -> Result<usize> {
    if app.active_network_opt().is_none() {
        return Ok(0);
    }
    let own_pubkey = match app.own_nostr_pubkey_hex() {
        Ok(pubkey) => pubkey,
        Err(_) => return Ok(0),
    };

    let Some(signed_roster) = active_signed_roster_for_sync(app, config_path, false)? else {
        return Ok(0);
    };
    let mut recipients = app.active_network_signal_pubkeys_hex();
    recipients.extend(extra_recipients.iter().cloned());
    recipients.extend(pending_recipients.drain());
    recipients.retain(|recipient| recipient != &own_pubkey);
    recipients.sort();
    recipients.dedup();

    let (ready_recipients, mut retry) = split_ready_fips_roster_recipients(recipients);
    let mut sent = 0usize;
    for recipient in ready_recipients {
        match runtime.send_roster(&recipient, signed_roster.clone()).await {
            Ok(()) => sent += 1,
            Err(error) => {
                eprintln!("fips: roster send to {recipient} failed: {error}");
                retry.insert(recipient);
            }
        }
    }
    *pending_recipients = retry;
    Ok(sent)
}

#[cfg(feature = "embedded-fips")]
fn split_ready_fips_roster_recipients(recipients: Vec<String>) -> (Vec<String>, HashSet<String>) {
    // Do not gate roster sends on nvpn presence. A stale-roster peer may drop
    // Ping/Pong from newly added peers as unknown until this signed roster
    // reaches it, while FIPS can still route/discover the control message.
    (recipients, HashSet::new())
}

fn ipv4_is_local_only(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_link_local()
        || ip.is_loopback()
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
}

fn endpoint_host_ip(endpoint: &str) -> Option<IpAddr> {
    let host = endpoint
        .rsplit_once(':')
        .map_or(endpoint, |(host, _)| host)
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.parse::<IpAddr>().ok()
}

fn endpoint_is_local_only(endpoint: &str) -> bool {
    match endpoint_host_ip(endpoint) {
        Some(IpAddr::V4(ip)) => ipv4_is_local_only(ip),
        Some(IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unicast_link_local() || ip.is_unique_local()
        }
        None => endpoint.eq_ignore_ascii_case("localhost"),
    }
}

#[cfg(test)]
const TEST_MACOS_EUID_SENTINEL: u32 = u32::MAX;
#[cfg(test)]
static TEST_MACOS_EUID_OVERRIDE: AtomicU32 = AtomicU32::new(TEST_MACOS_EUID_SENTINEL);
#[cfg(test)]
static TEST_MACOS_EUID_OVERRIDE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
#[cfg(test)]
static TEST_REPAIR_SAVED_NETWORK_STATE_CALLS: AtomicU32 = AtomicU32::new(0);
#[cfg(test)]
static TEST_REPAIR_SAVED_NETWORK_STATE_CALLS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn macos_euid_override_lock_for_test() -> &'static Mutex<()> {
    TEST_MACOS_EUID_OVERRIDE_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(crate) fn repair_saved_network_state_call_lock_for_test() -> &'static Mutex<()> {
    TEST_REPAIR_SAVED_NETWORK_STATE_CALLS_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(crate) fn set_macos_euid_override_for_test(value: Option<u32>) {
    TEST_MACOS_EUID_OVERRIDE.store(value.unwrap_or(TEST_MACOS_EUID_SENTINEL), Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn reset_repair_saved_network_state_call_count_for_test() {
    TEST_REPAIR_SAVED_NETWORK_STATE_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn repair_saved_network_state_call_count_for_test() -> u32 {
    TEST_REPAIR_SAVED_NETWORK_STATE_CALLS.load(Ordering::Relaxed)
}
