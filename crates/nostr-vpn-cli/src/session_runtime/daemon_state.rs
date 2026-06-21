struct DaemonStartupFailureContext<'a> {
    network: &'a NetworkSummary,
    port_mapping: &'a PortMappingStatus,
}

fn persist_daemon_startup_failure_state(
    state_file: &Path,
    app: &AppConfig,
    vpn_enabled: bool,
    expected_peers: usize,
    tunnel_runtime: &CliTunnelRuntime,
    context: DaemonStartupFailureContext<'_>,
    vpn_status: &str,
) {
    let advertised_routes = HashMap::<String, Vec<String>>::new();
    let state = build_daemon_runtime_state(
        app,
        vpn_enabled,
        false,
        expected_peers,
        tunnel_runtime,
        &[],
        &[],
        &advertised_routes,
        vpn_status,
        context.network,
        context.port_mapping,
    );
    if let Err(error) = write_daemon_state(state_file, &state) {
        eprintln!("daemon: failed to persist startup failure state: {error}");
    }
}

fn remove_current_daemon_pid_record(pid_file: &Path) {
    let current_pid = std::process::id();
    match read_daemon_pid_record(pid_file) {
        Ok(Some(record)) if record.pid == current_pid => {
            let _ = fs::remove_file(pid_file);
        }
        Ok(_) => {}
        Err(error) => eprintln!(
            "daemon: failed to inspect pid file {} before cleanup: {error}",
            pid_file.display()
        ),
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn run_windows_service_dispatcher(args: DaemonArgs) -> Result<()> {
    WINDOWS_SERVICE_DAEMON_ARGS
        .set(args)
        .map_err(|_| anyhow!("windows service daemon arguments already initialized"))?;
    service_dispatcher::start(WINDOWS_SERVICE_NAME, ffi_windows_service_main)
        .context("failed to start Windows service dispatcher")
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_windows_service() {
        eprintln!("windows service failed: {error:?}");
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn run_windows_service() -> Result<()> {
    let args = WINDOWS_SERVICE_DAEMON_ARGS
        .get()
        .cloned()
        .ok_or_else(|| anyhow!("windows service launched without daemon arguments"))?;
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let status_handle = service_control_handler::register(WINDOWS_SERVICE_NAME, {
        let config_path = config_path.clone();
        move |control_event| match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = request_daemon_stop(&config_path);
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })
    .context("failed to register Windows service control handler")?;

    set_windows_service_status(
        &status_handle,
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        Duration::from_secs(10),
    )?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for Windows service")?;

    set_windows_service_status(
        &status_handle,
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
        Duration::default(),
    )?;

    let result = runtime.block_on(daemon_vpn(args));
    let exit_code = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::Win32(1)
    };
    set_windows_service_status(
        &status_handle,
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
        Duration::default(),
    )?;
    result
}

#[cfg(target_os = "windows")]
pub(crate) fn set_windows_service_status(
    status_handle: &service_control_handler::ServiceStatusHandle,
    state: ServiceState,
    controls_accepted: ServiceControlAccept,
    exit_code: ServiceExitCode,
    wait_hint: Duration,
) -> Result<()> {
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint: 0,
            wait_hint,
            process_id: None,
        })
        .with_context(|| format!("failed to update Windows service status to {state:?}"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_daemon_runtime_state(
    app: &AppConfig,
    vpn_enabled: bool,
    vpn_active: bool,
    expected_peers: usize,
    tunnel_runtime: &CliTunnelRuntime,
    fips_peer_statuses: &[MeshPeerStatus],
    fips_relay_statuses: &[DaemonRelayState],
    advertised_routes_by_participant: &HashMap<String, Vec<String>>,
    vpn_status: &str,
    network: &NetworkSummary,
    port_mapping: &PortMappingStatus,
) -> DaemonRuntimeState {
    let own_pubkey = app.own_nostr_pubkey_hex().ok();
    let now = unix_timestamp();
    let listen_port = tunnel_runtime.listen_port(app.node.listen_port);
    let local_endpoint = local_signal_endpoint(app, listen_port);
    // Daemon no longer pre-discovers a public endpoint; fips-core advertises
    // its own (and falls back to udp:nat traversal when behind NAT). The
    // advertised_endpoint we surface in state.json is the local-network
    // endpoint, used by other peers on the same LAN.
    let advertised_endpoint = local_endpoint.clone();
    let mut peers = Vec::new();

    let participant_pubkeys_list = app.participant_pubkeys_hex();
    let participant_pubkeys = participant_pubkeys_list
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let fips_status_by_pubkey = fips_peer_statuses
        .iter()
        .map(|status| (status.pubkey.as_str(), status))
        .collect::<HashMap<_, _>>();
    let network_id = app.effective_network_id();
    for participant in &participant_pubkeys_list {
        if Some(participant.as_str()) == own_pubkey.as_deref() {
            continue;
        }
        let status = if vpn_active {
            fips_status_by_pubkey.get(participant.as_str()).copied()
        } else {
            None
        };
        let last_seen_at =
            status.and_then(|status| credible_daemon_peer_timestamp(now, status.last_seen_at));
        let last_control_seen_at = status
            .and_then(|status| credible_daemon_peer_timestamp(now, status.last_control_seen_at));
        let last_data_seen_at =
            status.and_then(|status| credible_daemon_peer_timestamp(now, status.last_data_seen_at));
        let reachable = vpn_active && status.is_some_and(|status| status.connected);
        let fips_transport_addr = status.and_then(|status| status.transport_addr.clone());
        let tunnel_ip = derive_mesh_tunnel_ip(&network_id, participant).unwrap_or_default();
        peers.push(DaemonPeerState {
            participant_pubkey: participant.clone(),
            node_id: String::new(),
            tunnel_ip,
            endpoint: "fips".to_string(),
            runtime_endpoint: fips_transport_addr
                .clone()
                .or_else(|| reachable.then(|| "fips".to_string())),
            fips_endpoint_npub: status
                .map(|status| status.endpoint_npub.clone())
                .unwrap_or_default(),
            fips_transport_addr: fips_transport_addr.unwrap_or_default(),
            fips_transport_type: status
                .and_then(|status| status.transport_type.clone())
                .unwrap_or_default(),
            fips_srtt_ms: status.and_then(|status| status.srtt_ms),
            fips_srtt_age_ms: status.and_then(|status| status.srtt_age_ms),
            fips_packets_sent: status.map(|status| status.link_packets_sent).unwrap_or(0),
            fips_packets_recv: status.map(|status| status.link_packets_recv).unwrap_or(0),
            fips_bytes_sent: status.map(|status| status.link_bytes_sent).unwrap_or(0),
            fips_bytes_recv: status.map(|status| status.link_bytes_recv).unwrap_or(0),
            fips_rekey_in_progress: status.is_some_and(|status| status.rekey_in_progress),
            fips_rekey_draining: status.is_some_and(|status| status.rekey_draining),
            fips_current_k_bit: status.and_then(|status| status.current_k_bit),
            fips_last_outbound_route: status
                .and_then(|status| status.last_outbound_route.clone())
                .unwrap_or_default(),
            direct_probe_pending: status.is_some_and(|status| status.direct_probe_pending),
            direct_probe_after_ms: status.and_then(|status| status.direct_probe_after_ms),
            direct_probe_retry_count: status
                .map(|status| status.direct_probe_retry_count)
                .unwrap_or(0),
            direct_probe_auto_reconnect: status
                .is_some_and(|status| status.direct_probe_auto_reconnect),
            direct_probe_expires_at_ms: status.and_then(|status| status.direct_probe_expires_at_ms),
            fips_nostr_traversal_failures: status
                .map(|status| status.nostr_traversal_consecutive_failures)
                .unwrap_or(0),
            fips_nostr_traversal_in_cooldown: status
                .is_some_and(|status| status.nostr_traversal_in_cooldown),
            fips_nostr_traversal_cooldown_until_ms: status
                .and_then(|status| status.nostr_traversal_cooldown_until_ms),
            fips_nostr_traversal_last_observed_skew_ms: status
                .and_then(|status| status.nostr_traversal_last_observed_skew_ms),
            tx_bytes: status.map(|status| status.tx_bytes).unwrap_or(0),
            rx_bytes: status.map(|status| status.rx_bytes).unwrap_or(0),
            public_key: String::new(),
            advertised_routes: advertised_routes_by_participant
                .get(participant)
                .cloned()
                .unwrap_or_default(),
            last_mesh_seen_at: last_seen_at.unwrap_or(0),
            last_fips_seen_at: last_seen_at,
            last_fips_control_seen_at: last_control_seen_at,
            last_fips_data_seen_at: last_data_seen_at,
            reachable,
            last_handshake_at: last_seen_at,
            error: if reachable {
                None
            } else {
                status
                    .and_then(|status| status.error.clone())
                    .or_else(|| Some("fips link pending".to_string()))
            },
        });
    }

    let connected_peer_count = if !vpn_active {
        0
    } else {
        fips_peer_statuses
            .iter()
            .filter(|status| Some(status.pubkey.as_str()) != own_pubkey.as_deref())
            .filter(|status| participant_pubkeys.contains(&status.pubkey))
            .filter(|status| status.connected)
            .count()
    };
    let fips_direct_roster_peer_count = if !vpn_active {
        0
    } else {
        fips_peer_statuses
            .iter()
            .filter(|status| Some(status.pubkey.as_str()) != own_pubkey.as_deref())
            .filter(|status| participant_pubkeys.contains(&status.pubkey))
            .filter(|status| status.connected)
            .filter(|status| {
                status
                    .transport_addr
                    .as_deref()
                    .is_some_and(|addr| !addr.trim().is_empty())
            })
            .count()
    };
    let fips_other_peer_count = if !vpn_active {
        0
    } else {
        fips_peer_statuses
            .iter()
            .filter(|status| Some(status.pubkey.as_str()) != own_pubkey.as_deref())
            .filter(|status| !participant_pubkeys.contains(&status.pubkey))
            .filter(|status| status.connected)
            .count()
    };
    let mesh_ready = vpn_active;
    let health = build_health_issues(app, vpn_active, mesh_ready, network, port_mapping, &peers);
    DaemonRuntimeState {
        updated_at: now,
        binary_version: PRODUCT_VERSION.to_string(),
        fips_core_version: fips_core_build_version(),
        local_endpoint,
        advertised_endpoint,
        listen_port,
        vpn_enabled,
        vpn_active,
        vpn_status: vpn_status.to_string(),
        expected_peer_count: expected_peers,
        connected_peer_count,
        fips_direct_roster_peer_count,
        fips_other_peer_count,
        mesh_ready,
        health,
        network: network.clone(),
        port_mapping: port_mapping.clone(),
        relays: fips_relay_statuses.to_vec(),
        peers,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_daemon_runtime_state(
    path: &Path,
    app: &AppConfig,
    vpn_enabled: bool,
    expected_peers: usize,
    tunnel_runtime: &CliTunnelRuntime,
    fips_peer_statuses: &[MeshPeerStatus],
    fips_relay_statuses: &[DaemonRelayState],
    advertised_routes_by_participant: &HashMap<String, Vec<String>>,
    vpn_status: &str,
    network: &NetworkSummary,
    port_mapping: &PortMappingStatus,
) -> Result<()> {
    write_daemon_state(
        path,
        &build_daemon_runtime_state(
            app,
            vpn_enabled,
            daemon_vpn_active(vpn_enabled, expected_peers),
            expected_peers,
            tunnel_runtime,
            fips_peer_statuses,
            fips_relay_statuses,
            advertised_routes_by_participant,
            vpn_status,
            network,
            port_mapping,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_daemon_runtime_and_cleanup_state(
    state_file: &Path,
    config_path: &Path,
    app: &AppConfig,
    vpn_enabled: bool,
    expected_peers: usize,
    tunnel_runtime: &CliTunnelRuntime,
    fips_peer_statuses: &[MeshPeerStatus],
    fips_relay_statuses: &[DaemonRelayState],
    advertised_routes_by_participant: &HashMap<String, Vec<String>>,
    vpn_status: &str,
    network: &NetworkSummary,
    port_mapping: &PortMappingStatus,
) -> bool {
    let persisted = match persist_daemon_runtime_state(
        state_file,
        app,
        vpn_enabled,
        expected_peers,
        tunnel_runtime,
        fips_peer_statuses,
        fips_relay_statuses,
        advertised_routes_by_participant,
        vpn_status,
        network,
        port_mapping,
    ) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("daemon: failed to persist runtime state: {error}");
            false
        }
    };
    if let Err(error) = persist_daemon_network_cleanup_state(config_path, tunnel_runtime) {
        eprintln!("daemon: failed to persist network cleanup state: {error}");
    }
    persisted
}

pub(crate) fn disconnected_daemon_runtime_state(
    expected_peers: usize,
    network: &NetworkSummary,
) -> DaemonRuntimeState {
    DaemonRuntimeState {
        updated_at: unix_timestamp(),
        binary_version: PRODUCT_VERSION.to_string(),
        fips_core_version: fips_core_build_version(),
        local_endpoint: String::new(),
        advertised_endpoint: String::new(),
        listen_port: 0,
        vpn_enabled: false,
        vpn_active: false,
        vpn_status: "Disconnected".to_string(),
        expected_peer_count: expected_peers,
        connected_peer_count: 0,
        fips_direct_roster_peer_count: 0,
        fips_other_peer_count: 0,
        mesh_ready: false,
        health: Vec::new(),
        network: network.clone(),
        port_mapping: PortMappingStatus::default(),
        relays: Vec::new(),
        peers: Vec::new(),
    }
}
