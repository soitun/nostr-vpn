use super::*;
#[cfg(any(target_os = "linux", test))]
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[path = "webvm_guest/daemon.rs"]
mod daemon;
pub(crate) use daemon::{args_from_daemon, run_daemon};

#[cfg(target_os = "linux")]
use fips_core::FipsEndpointServiceDatagram;
#[cfg(target_os = "linux")]
use fips_endpoint::{FipsEndpoint, PeerIdentity};
use nostr_vpn_core::join_pubsub::NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT;
#[cfg(target_os = "linux")]
use nostr_vpn_core::join_pubsub::{NostrJoinFipsPubsubClient, NostrJoinFipsPubsubDatagram};
#[cfg(any(target_os = "linux", test))]
use std::io::Write as _;
#[cfg(all(unix, any(target_os = "linux", test)))]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(any(target_os = "linux", test))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(any(target_os = "linux", test))]
const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
#[cfg(target_os = "linux")]
const HOST_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const BROWSER_HOST_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(target_os = "linux")]
const SERVICE_RECV_BATCH: usize = 8;
#[cfg(target_os = "linux")]
const WEBVM_APPROVAL_ROUTE_REGISTRATION: &[u8; 9] = b"NVPNPAIR1";
const DEFAULT_WEBVM_PAIRING_URI_PATH: &str = "/run/webvm/join-request";

pub(crate) async fn run(args: WebvmGuestArgs) -> Result<()> {
    run_daemon(args, false).await
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct WebvmStop;

#[cfg(target_os = "linux")]
impl std::fmt::Display for WebvmStop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WebVM daemon stopped")
    }
}

#[cfg(target_os = "linux")]
impl std::error::Error for WebvmStop {}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct WebvmReloadJoinRequest;

#[cfg(target_os = "linux")]
impl std::fmt::Display for WebvmReloadJoinRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WebVM join request changed")
    }
}

#[cfg(target_os = "linux")]
impl std::error::Error for WebvmReloadJoinRequest {}

fn validate_args(args: &WebvmGuestArgs) -> Result<()> {
    if args.config.as_os_str().is_empty() {
        return Err(anyhow!("--config must not be empty"));
    }
    if args.ethernet_interface.trim().is_empty() {
        return Err(anyhow!("--ethernet-interface must not be empty"));
    }
    if args.discovery_scope.trim().is_empty() {
        return Err(anyhow!("--discovery-scope must not be empty"));
    }
    if args.join_pubsub_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT {
        return Err(anyhow!(
            "--join-pubsub-port must be {NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT}"
        ));
    }
    if args.pairing_uri_file.as_os_str().is_empty() {
        return Err(anyhow!("--pairing-uri-file must not be empty"));
    }
    if args.tun_interface.trim().is_empty() {
        return Err(anyhow!("--tun-interface must not be empty"));
    }
    if args.tun_interface.trim() == args.ethernet_interface.trim() {
        return Err(anyhow!(
            "--tun-interface must differ from --ethernet-interface"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn run_linux(args: WebvmGuestArgs) -> Result<()> {
    validate_ethernet_underlay_is_layer2_only(args.ethernet_interface.trim())?;
    let mut app = load_or_initialize_config(&args.config, unix_timestamp())?;
    let shared = crate::fips_private_mesh::bind_local_ethernet_shared_endpoint(
        app.nostr.secret_key.clone(),
        args.ethernet_interface.trim(),
        args.discovery_scope.trim(),
    )
    .await?;
    let endpoint = Arc::clone(shared.endpoint());
    let host_network =
        match crate::fips_private_mesh::WebvmFipsHostNetworkRuntime::start(Arc::clone(&endpoint))
            .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = endpoint.shutdown().await;
                return Err(error);
            }
        };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let supervisor = tokio::spawn(supervise_webvm_daemon(
        args.config.clone(),
        args.tun_interface.clone(),
        Arc::clone(&endpoint),
        shutdown_tx,
    ));
    if app.active_network_opt().is_none()
        && let Err(error) = endpoint.register_service(args.join_pubsub_port).await
    {
        supervisor.abort();
        let _ = supervisor.await;
        let _ = host_network.stop().await;
        let _ = endpoint.shutdown().await;
        return Err(error).context("failed to register WebVM join pubsub service");
    }
    let setup_result = async {
        while app.active_network_opt().is_none() {
            match pair_over_fips(&args, &endpoint, &mut app, shutdown_rx.clone()).await {
                Err(error) if error.downcast_ref::<WebvmReloadJoinRequest>().is_some() => {
                    app = load_or_initialize_config(&args.config, unix_timestamp())?;
                }
                other => other?,
            }
        }
        validate_approved_config(&app)?;
        remove_pairing_uri(&args.pairing_uri_file)
    }
    .await;
    if let Err(error) = setup_result {
        supervisor.abort();
        let _ = supervisor.await;
        let _ = host_network.stop().await;
        let _ = endpoint.shutdown().await;
        return Err(error);
    }
    let result = run_tunnel(&args, app, shared, host_network, shutdown_rx).await;
    supervisor.abort();
    let _ = supervisor.await;
    result
}

#[cfg(target_os = "linux")]
async fn supervise_webvm_daemon(
    config_path: PathBuf,
    tun_interface: String,
    endpoint: Arc<FipsEndpoint>,
    shutdown: tokio::sync::watch::Sender<bool>,
) {
    let state_file = daemon_state_file_path(&config_path);
    let mut state_tick = tokio::time::interval(Duration::from_millis(500));
    state_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                let _ = signal.recv().await;
            }
            Err(error) => {
                eprintln!("daemon: failed to install WebVM SIGTERM handler: {error}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::pin!(terminate);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                let _ = shutdown.send(true);
                return;
            }
            _ = &mut terminate => {
                let _ = shutdown.send(true);
                return;
            }
            _ = state_tick.tick() => {
                if let Some(request) = take_daemon_control_request(&config_path) {
                    match request {
                        DaemonControlRequest::Stop => {
                            let _ = write_daemon_control_result(&config_path, request, Ok(()));
                            let _ = shutdown.send(true);
                            return;
                        }
                        DaemonControlRequest::Reload => {
                            let _ = write_daemon_control_result(&config_path, request, Ok(()));
                        }
                        DaemonControlRequest::Pause | DaemonControlRequest::Resume => {
                            let _ = write_daemon_control_result(
                                &config_path,
                                request,
                                Err(anyhow!("pause/resume is not supported by the WebVM FIPS daemon")),
                            );
                        }
                    }
                }
                if let Err(error) = persist_webvm_daemon_state(
                    &config_path,
                    &state_file,
                    &tun_interface,
                    &endpoint,
                ).await {
                    eprintln!("daemon: failed to persist WebVM state: {error}");
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn persist_webvm_daemon_state(
    config_path: &Path,
    state_file: &Path,
    tun_interface: &str,
    endpoint: &FipsEndpoint,
) -> Result<()> {
    let app = AppConfig::load(config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    let endpoint_peers = endpoint
        .peers()
        .await
        .context("failed to inspect WebVM FIPS endpoint peers")?;
    let peer_statuses =
        crate::fips_private_mesh::endpoint_peer_statuses(&endpoint_peers, unix_timestamp());
    let connected = peer_statuses.iter().filter(|peer| peer.connected).count();
    let approved = app.active_network_has_confirmed_local_identity();
    let vpn_status = if approved {
        "Nostr VPN active"
    } else if connected == 0 {
        "FIPS active; waiting for join approval (no FIPS peers)"
    } else {
        "FIPS active; waiting for join approval"
    };
    let tunnel_runtime = CliTunnelRuntime::new(tun_interface.to_string());
    let advertised_routes = HashMap::new();
    let mut state = build_daemon_runtime_state(DaemonRuntimeStateInput {
        app: &app,
        vpn_enabled: true,
        vpn_active: true,
        expected_peers: expected_peer_count(&app),
        tunnel_runtime: &tunnel_runtime,
        fips_peer_statuses: &peer_statuses,
        fips_relay_statuses: &[],
        fips_endpoint_peers: &[],
        advertised_routes_by_participant: &advertised_routes,
        vpn_status,
        network: &NetworkSummary::default(),
        port_mapping: &PortMappingStatus::default(),
    });
    let own_pubkey = app.own_nostr_pubkey_hex().ok();
    for peer in &peer_statuses {
        if own_pubkey.as_deref() == Some(peer.pubkey.as_str())
            || state
                .peers
                .iter()
                .any(|known| known.participant_pubkey == peer.pubkey)
        {
            continue;
        }
        state.peers.push(daemon_peer_state_from_fips_status(
            &app.effective_network_id(),
            &peer.pubkey,
            Some(peer),
            Vec::new(),
            unix_timestamp(),
            true,
        ));
    }
    state.connected_peer_count = state.peers.iter().filter(|peer| peer.reachable).count();
    write_daemon_state(state_file, &state)
}

#[cfg(target_os = "linux")]
fn validate_ethernet_underlay_is_layer2_only(interface: &str) -> Result<()> {
    let addresses = crate::command_stdout_checked(
        ProcessCommand::new("ip")
            .arg("-o")
            .arg("address")
            .arg("show")
            .arg("dev")
            .arg(interface),
    )
    .with_context(|| format!("failed to inspect WebVM Ethernet underlay {interface}"))?;
    let ipv4_default_routes = crate::command_stdout_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("show")
            .arg("table")
            .arg("all")
            .arg("default")
            .arg("dev")
            .arg(interface),
    )
    .with_context(|| {
        format!("failed to inspect WebVM Ethernet underlay {interface} IPv4 routes")
    })?;
    let ipv6_default_routes = crate::command_stdout_checked(
        ProcessCommand::new("ip")
            .arg("-6")
            .arg("route")
            .arg("show")
            .arg("table")
            .arg("all")
            .arg("default")
            .arg("dev")
            .arg(interface),
    )
    .with_context(|| {
        format!("failed to inspect WebVM Ethernet underlay {interface} IPv6 routes")
    })?;

    validate_ethernet_underlay_snapshot(
        interface,
        &addresses,
        &ipv4_default_routes,
        &ipv6_default_routes,
    )
}

#[cfg(any(target_os = "linux", test))]
fn validate_ethernet_underlay_snapshot(
    interface: &str,
    addresses: &str,
    ipv4_default_routes: &str,
    ipv6_default_routes: &str,
) -> Result<()> {
    if let Some(address) = addresses.lines().find(|line| !line.trim().is_empty()) {
        return Err(anyhow!(
            "WebVM Ethernet underlay {interface} has an L3 address configured: {}",
            address.trim()
        ));
    }
    if let Some(route) = ipv4_default_routes
        .lines()
        .chain(ipv6_default_routes.lines())
        .find(|line| !line.trim().is_empty())
    {
        return Err(anyhow!(
            "WebVM Ethernet underlay {interface} has a default route configured: {}",
            route.trim()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn load_or_initialize_config(path: &Path, now: u64) -> Result<AppConfig> {
    let exists = path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    let mut app = if exists {
        AppConfig::load(path).with_context(|| format!("failed to load {}", path.display()))?
    } else {
        AppConfig::generated_without_networks()
    };
    app.ensure_defaults();

    let changed = if app.active_network_opt().is_some() {
        if app.pending_nostr_join_request.is_some() {
            return Err(anyhow!(
                "approved WebVM config still contains a pending Nostr join request"
            ));
        }
        false
    } else {
        app.ensure_pending_nostr_join_request(now)?
    };
    if !exists || changed {
        app.save(path)
            .with_context(|| format!("failed to persist {}", path.display()))?;
    }
    Ok(app)
}

#[cfg(any(target_os = "linux", test))]
fn validate_approved_config(app: &AppConfig) -> Result<()> {
    let network = app
        .active_network_opt()
        .ok_or_else(|| anyhow!("WebVM guest has not been approved"))?;
    let devices = app.participant_pubkeys_hex();
    if network.shared_roster_updated_at == 0 || network.shared_roster_signed_by.is_empty() {
        return Err(anyhow!(
            "approved WebVM config does not contain a verified signed roster"
        ));
    }
    if app.exit_node.is_empty() {
        return Err(anyhow!("approved WebVM config has no VPN exit node"));
    }
    if !devices.iter().any(|device| device == &app.exit_node) {
        return Err(anyhow!(
            "approved WebVM exit node is not present in the signed roster"
        ));
    }
    if normalize_runtime_network_id(&network.network_id).is_empty() {
        return Err(anyhow!("approved WebVM network id is empty"));
    }
    if app.wireguard_exit.enabled {
        return Err(anyhow!(
            "WebVM guest requires a Nostr VPN exit, not a WireGuard fallback"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn webvm_pairing_uri(app: &AppConfig, fips_route_npub: &str) -> Result<String> {
    let route = normalize_nostr_pubkey(fips_route_npub)?;
    let route = hex::decode(route).context("invalid WebVM FIPS return route")?;
    let route = URL_SAFE_NO_PAD.encode(route);
    app.pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
        .map(|request| format!("{request}?r={route}"))
}

#[cfg(target_os = "linux")]
async fn pair_over_fips(
    args: &WebvmGuestArgs,
    endpoint: &FipsEndpoint,
    app: &mut AppConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let browser_host = wait_for_browser_host(endpoint, shutdown.clone()).await?;
    let browser_identity = PeerIdentity::from_npub(&browser_host)
        .context("invalid WebVM browser FIPS return route")?;
    endpoint
        .send_datagram(
            browser_identity,
            args.join_pubsub_port,
            args.join_pubsub_port,
            WEBVM_APPROVAL_ROUTE_REGISTRATION.to_vec(),
        )
        .await
        .context("failed to register WebVM approval return route")?;
    let pairing_uri = webvm_pairing_uri(app, &browser_host)?;
    write_pairing_uri(&args.pairing_uri_file, &pairing_uri)?;

    println!(
        "webvm: awaiting direct approval over FIPS service {}",
        args.join_pubsub_port
    );
    let mut client = NostrJoinFipsPubsubClient::new(app)?;
    wait_for_approval(endpoint, &args.config, app, &mut client, shutdown).await?;
    remove_pairing_uri(&args.pairing_uri_file)?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn wait_for_browser_host(
    endpoint: &FipsEndpoint,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<String> {
    loop {
        let mut browser_hosts = endpoint
            .peers()
            .await
            .context("failed to inspect WebVM FIPS peers")?
            .into_iter()
            .filter(|peer| {
                peer.connected
                    && peer
                        .transport_type
                        .as_deref()
                        .is_some_and(|transport| transport.eq_ignore_ascii_case("ethernet"))
            })
            .map(|peer| peer.npub)
            .collect::<Vec<_>>();
        browser_hosts.sort();
        browser_hosts.dedup();
        if let Some(browser_host) = browser_hosts.into_iter().next() {
            return Ok(browser_host);
        }

        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err(anyhow::Error::new(WebvmStop));
                }
            }
            _ = tokio::time::sleep(BROWSER_HOST_POLL_INTERVAL) => {}
        }
    }
}

#[cfg(target_os = "linux")]
async fn wait_for_approval(
    endpoint: &FipsEndpoint,
    config_path: &Path,
    app: &mut AppConfig,
    client: &mut NostrJoinFipsPubsubClient,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let mut datagrams = Vec::<FipsEndpointServiceDatagram>::with_capacity(SERVICE_RECV_BATCH);
    let mut host_poll = tokio::time::interval(HOST_POLL_INTERVAL);
    host_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err(anyhow::Error::new(WebvmStop));
                }
            }
            _ = host_poll.tick() => {
                if pending_join_request_changed(config_path, app)? {
                    return Err(anyhow::Error::new(WebvmReloadJoinRequest));
                }
            }
            received = endpoint.recv_service_datagram_batch_into(
                &mut datagrams,
                SERVICE_RECV_BATCH,
            ) => {
                let Some(_) = received else {
                    return Err(anyhow!("WebVM pairing FIPS endpoint closed"));
                };
                for datagram in &datagrams {
                    // FIPS is routed, so the authenticated source can be an admin
                    // reached through a transit peer. The signed payload and request
                    // secret still authorize the approval itself.
                    let inbound = NostrJoinFipsPubsubDatagram {
                        source_port: datagram.source_port,
                        destination_port: datagram.destination_port,
                        payload: datagram.data.as_ref().to_vec(),
                    };
                    println!("webvm: received approval candidate over FIPS");
                    let mut candidate = app.clone();
                    if let Some(applied) = client.ingest_datagram(
                        &mut candidate,
                        &inbound,
                        unix_timestamp(),
                    )? {
                        validate_approved_config(&candidate)?;
                        candidate.save(config_path).with_context(|| {
                            format!("failed to persist approved config {}", config_path.display())
                        })?;
                        *app = candidate;
                        println!(
                            "webvm: approval applied for network {} by {}",
                            applied.network_id,
                            applied.approved_by_pubkey,
                        );
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn pending_join_request_changed(config_path: &Path, app: &AppConfig) -> Result<bool> {
    let current = app
        .pending_nostr_join_request
        .as_ref()
        .map(|pending| pending.request.request_pubkey.as_str());
    let reloaded = AppConfig::load(config_path)
        .with_context(|| format!("failed to reload {}", config_path.display()))?;
    let next = reloaded
        .pending_nostr_join_request
        .as_ref()
        .map(|pending| pending.request.request_pubkey.as_str());
    Ok(current != next)
}

#[cfg(target_os = "linux")]
async fn run_tunnel(
    args: &WebvmGuestArgs,
    mut app: AppConfig,
    shared: crate::fips_private_mesh::FipsSharedEndpoint,
    host_network: crate::fips_private_mesh::WebvmFipsHostNetworkRuntime,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let endpoint = Arc::clone(shared.endpoint());
    let mut tunnel = match build_tunnel_config(args, &app) {
        Ok(tunnel) => tunnel,
        Err(error) => {
            let _ = host_network.stop().await;
            let _ = endpoint.shutdown().await;
            return Err(error);
        }
    };
    let runtime = crate::fips_private_mesh::FipsPrivateTunnelRuntime::start_with_shared_endpoint(
        tunnel.clone(),
        shared,
    )
    .await;
    let mut runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = host_network.stop().await;
            let _ = endpoint.shutdown().await;
            return Err(error).context("failed to start WebVM guest VPN tunnel");
        }
    };
    if let Err(error) = host_network.enable_vpn_dns(&app.exit_node) {
        let _ = host_network.stop().await;
        let _ = runtime.stop().await;
        return Err(error);
    }
    println!(
        "webvm: Nostr VPN tunnel {} over Ethernet {}",
        runtime.iface(),
        args.ethernet_interface
    );

    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status = String::new();
    let run_result = loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break Err(anyhow::Error::new(WebvmStop));
                }
            }
            _ = heartbeat.tick() => {
                let network_id = app.effective_network_id();
                if let Err(error) = runtime.ping_peers(&network_id, unix_timestamp()).await {
                    eprintln!("webvm: FIPS peer ping failed: {error}");
                }
                if let Err(error) = runtime.refresh_link_statuses().await {
                    eprintln!("webvm: FIPS link snapshot failed: {error}");
                }
                match drain_fips_mesh_events(
                    &mut runtime,
                    &mut app,
                    &args.config,
                    &mut status,
                ) {
                    Ok(drained) if drained.roster_changed => {
                        tunnel = match build_tunnel_config(args, &app) {
                            Ok(tunnel) => tunnel,
                            Err(error) => break Err(error),
                        };
                        if let Err(error) = runtime.apply_config(tunnel.clone()).await {
                            break Err(error).context("failed to apply updated WebVM roster");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("webvm: FIPS event handling failed: {error}"),
                }
                if let Err(error) = runtime.refresh_peer_dependent_routes().await {
                    eprintln!("webvm: route refresh failed: {error}");
                }
            }
        }
    };

    let host_result = host_network.stop().await;
    let tunnel_result = runtime.stop().await;
    run_result?;
    host_result.context("failed to stop WebVM .fips host network")?;
    tunnel_result.context("failed to stop WebVM guest tunnel")
}

#[cfg(target_os = "linux")]
fn build_tunnel_config(
    args: &WebvmGuestArgs,
    app: &AppConfig,
) -> Result<crate::fips_private_mesh::FipsPrivateTunnelConfig> {
    validate_approved_config(app)?;
    let network_id = app.effective_network_id();
    let own_pubkey = app.own_nostr_pubkey_hex()?;
    let underlay_interface_mtu = netdev::get_interfaces()
        .into_iter()
        .find(|interface| interface.name == args.ethernet_interface)
        .and_then(|interface| interface.mtu);
    let mut config = fips_tunnel_config_from_app(FipsTunnelConfigInput {
        app,
        config_path: &args.config,
        network_id: &network_id,
        iface: args.tun_interface.clone(),
        underlay_interface_mtu,
        own_pubkey: Some(&own_pubkey),
        recent_peers: None,
        live_peer_endpoints: &[],
    })?;
    config.use_local_ethernet_only(args.ethernet_interface.trim(), args.discovery_scope.trim());
    Ok(config)
}

#[cfg(any(target_os = "linux", test))]
fn write_pairing_uri(path: &Path, uri: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("pairing-uri");
    let temp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temp)
        .with_context(|| format!("failed to create {}", temp.display()))?;
    let write_result = (|| -> Result<()> {
        file.write_all(uri.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
            .with_context(|| format!("failed to replace pairing URI file {}", path.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

#[cfg(any(target_os = "linux", test))]
fn remove_pairing_uri(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove pairing URI file {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nvpn-webvm-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ))
    }

    #[test]
    fn first_boot_persists_one_stable_compact_join_bootstrap() {
        let path = temp_path("config").with_extension("toml");
        let first = load_or_initialize_config(&path, 1_778_998_000).expect("first boot");
        let route = first
            .nostr_keys()
            .expect("route keys")
            .public_key()
            .to_bech32()
            .expect("route npub");
        let first_uri = webvm_pairing_uri(&first, &route).expect("first URI");
        let second = load_or_initialize_config(&path, 1_778_998_100).expect("second boot");
        let second_uri = webvm_pairing_uri(&second, &route).expect("second URI");
        assert_eq!(first_uri, second_uri);
        assert!(first_uri.starts_with(JOIN_REQUEST_LINK_PREFIX));
        assert!(
            first_uri.len() <= 420,
            "pairing URI was {} bytes",
            first_uri.len()
        );
        let bootstrap =
            nostr_vpn_core::identity_bridge::parse_nostr_identity_device_approval_bootstrap(
                first_uri.split_once('?').expect("return route").0,
                &[JOIN_REQUEST_LINK_PREFIX],
            )
            .expect("parse compact bootstrap")
            .expect("bootstrap payload");
        assert_eq!(
            serde_json::to_value(&bootstrap)
                .expect("serialize bootstrap")
                .as_object()
                .expect("bootstrap object")
                .len(),
            4
        );
        assert!(bootstrap.label.is_some());
        let pending = second.pending_nostr_join_request.expect("pending request");
        assert_ne!(
            pending.request.request_pubkey,
            pending.request.device_app_key_pubkey
        );
        assert_eq!(bootstrap.request_secret, pending.request.request_secret);

        AppConfig::delete_persisted_secrets_for_path(&path).expect("delete secrets");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn pairing_uri_replace_is_atomic_and_private() {
        let path = temp_path("pairing-uri");
        write_pairing_uri(&path, "nvpn://join-request/first").expect("first write");
        write_pairing_uri(&path, "nvpn://join-request/second").expect("second write");
        assert_eq!(
            fs::read_to_string(&path).expect("read pairing URI"),
            "nvpn://join-request/second\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path)
                    .expect("pairing metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        remove_pairing_uri(&path).expect("remove pairing URI");
        assert!(!path.exists());
    }

    #[test]
    fn invalid_webvm_arguments_are_rejected_before_networking() {
        let args = WebvmGuestArgs {
            config: PathBuf::from("/tmp/config.toml"),
            ethernet_interface: "eth0".to_string(),
            discovery_scope: "fips-overlay-v1".to_string(),
            join_pubsub_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT + 1,
            pairing_uri_file: PathBuf::from("/run/webvm/pairing-uri"),
            tun_interface: "nvpn0".to_string(),
        };
        assert!(
            validate_args(&args)
                .expect_err("wrong service port")
                .to_string()
                .contains("7368")
        );
    }

    #[test]
    fn webvm_ethernet_underlay_rejects_any_l3_address() {
        validate_ethernet_underlay_snapshot("eth0", "", "", "")
            .expect("unconfigured Ethernet underlay");

        for addresses in [
            "2: eth0    inet 192.0.2.2/24 scope global eth0\n",
            "2: eth0    inet6 fe80::1/64 scope link\n",
        ] {
            let error = validate_ethernet_underlay_snapshot("eth0", addresses, "", "")
                .expect_err("L3 address must fail closed");
            assert!(error.to_string().contains("L3 address"));
        }
    }

    #[test]
    fn webvm_ethernet_underlay_rejects_ipv4_or_ipv6_default_route() {
        for (ipv4_defaults, ipv6_defaults) in [
            ("default via 192.0.2.1 dev eth0\n", ""),
            ("", "default via fe80::1 dev eth0 metric 1024\n"),
        ] {
            let error =
                validate_ethernet_underlay_snapshot("eth0", "", ipv4_defaults, ipv6_defaults)
                    .expect_err("default route must fail closed");
            assert!(error.to_string().contains("default route"));
        }
    }

    #[test]
    fn approved_webvm_config_requires_selected_exit_in_signed_roster() {
        use nostr_sdk::prelude::Keys;

        let mut app = AppConfig::generated();
        let own_pubkey = app.own_nostr_pubkey_hex().expect("own AppKey");
        let exit_pubkey = Keys::generate().public_key().to_hex();
        app.networks[0].enabled = true;
        app.networks[0].devices = vec![exit_pubkey.clone()];
        app.networks[0].admins = vec![own_pubkey.clone()];
        app.networks[0].shared_roster_updated_at = 1;
        app.networks[0].shared_roster_signed_by = own_pubkey.clone();
        app.exit_node = exit_pubkey;
        app.ensure_defaults();

        assert!(!app.participant_pubkeys_hex().contains(&own_pubkey));
        validate_approved_config(&app).expect("rostered Nostr VPN exit");
        app.exit_node = Keys::generate().public_key().to_hex();
        assert!(
            validate_approved_config(&app)
                .expect_err("unrostered exit must fail")
                .to_string()
                .contains("signed roster")
        );
    }
}
