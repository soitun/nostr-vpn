use super::*;
#[cfg(any(target_os = "linux", test))]
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[path = "webvm_guest/daemon.rs"]
mod daemon;
pub(crate) use daemon::{args_from_daemon, run_daemon};
#[cfg(any(target_os = "linux", test))]
#[path = "webvm_guest/pairing_storage.rs"]
mod pairing_storage;
#[cfg(target_os = "linux")]
use pairing_storage::{load_approval_ack, persist_approval_ack};
#[cfg(any(target_os = "linux", test))]
use pairing_storage::{remove_pairing_uri, write_pairing_uri};

#[cfg(target_os = "linux")]
use fips_core::{FipsEndpointServiceDatagram, FipsEndpointServiceReceiver};
#[cfg(target_os = "linux")]
use fips_endpoint::{FipsEndpoint, PeerIdentity};
use nostr_vpn_core::join_pubsub::NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT;
#[cfg(target_os = "linux")]
use nostr_vpn_core::join_pubsub::{
    NostrJoinFipsPubsubClient, NostrJoinFipsPubsubDatagram, approval_applied_ack_datagram,
    approval_event_datagram_matches_ack, parse_approval_applied_ack_datagram,
};
#[cfg(any(target_os = "linux", test))]
use std::io::Write as _;
#[cfg(all(unix, any(target_os = "linux", test)))]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(any(target_os = "linux", test))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::{Arc, RwLock};
#[cfg(any(target_os = "linux", test))]
const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
#[cfg(target_os = "linux")]
const PAIRING_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const BROWSER_HOST_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(target_os = "linux")]
const SERVICE_RECV_BATCH: usize = 8;
#[cfg(target_os = "linux")]
const WEBVM_APPROVAL_ROUTE_REGISTRATION: &[u8; 9] = b"NVPNPAIR1";
#[cfg(any(target_os = "linux", test))]
const WEBVM_MESH_INGRESS_HINT_MAGIC: &[u8; 9] = b"NVPNMESH1";
const DEFAULT_WEBVM_PAIRING_URI_PATH: &str = "/run/webvm/join-request";

#[cfg(target_os = "linux")]
type WebvmPeerStatusSnapshot = Arc<RwLock<Option<Vec<MeshPeerStatus>>>>;

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
    let peer_status_snapshot = Arc::new(RwLock::new(None));
    let supervisor = tokio::spawn(supervise_webvm_daemon(
        args.config.clone(),
        args.tun_interface.clone(),
        Arc::clone(&endpoint),
        Arc::clone(&peer_status_snapshot),
        shutdown_tx,
    ));
    let approval_receiver = match endpoint
        .register_service_receiver(args.join_pubsub_port)
        .await
    {
        Ok(receiver) => receiver,
        Err(error) => {
            supervisor.abort();
            let _ = supervisor.await;
            let _ = host_network.stop().await;
            let _ = endpoint.shutdown().await;
            return Err(error).context("failed to register WebVM join pubsub service");
        }
    };
    let setup_result = async {
        while app.active_network_opt().is_none() {
            match pair_over_fips(
                &args,
                &endpoint,
                &approval_receiver,
                &mut app,
                shutdown_rx.clone(),
            )
            .await
            {
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
    let result = run_tunnel(
        &args,
        app,
        shared,
        host_network,
        approval_receiver,
        peer_status_snapshot,
        shutdown_rx,
    )
    .await;
    supervisor.abort();
    let _ = supervisor.await;
    result
}

#[cfg(target_os = "linux")]
async fn supervise_webvm_daemon(
    config_path: PathBuf,
    tun_interface: String,
    endpoint: Arc<FipsEndpoint>,
    peer_status_snapshot: WebvmPeerStatusSnapshot,
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
                    &peer_status_snapshot,
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
    peer_status_snapshot: &WebvmPeerStatusSnapshot,
) -> Result<()> {
    let app = AppConfig::load(config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    let runtime_statuses = peer_status_snapshot
        .read()
        .map_err(|_| anyhow!("WebVM peer status snapshot lock poisoned"))?
        .clone();
    let peer_statuses = match runtime_statuses {
        Some(statuses) => statuses,
        None => {
            let endpoint_peers = endpoint
                .peers()
                .await
                .context("failed to inspect WebVM FIPS endpoint peers")?;
            crate::fips_private_mesh::endpoint_peer_statuses(&endpoint_peers, unix_timestamp())
        }
    };
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
    approval_receiver: &FipsEndpointServiceReceiver,
    app: &mut AppConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let browser_host = wait_for_browser_host(endpoint, shutdown.clone()).await?;
    let browser_identity = PeerIdentity::from_npub(&browser_host)
        .context("invalid WebVM browser FIPS return route")?;
    register_approval_route(endpoint, browser_identity, args.join_pubsub_port)
        .await
        .context("failed to register WebVM approval return route")?;
    let pairing_uri = webvm_pairing_uri(app, &browser_host)?;
    write_pairing_uri(&args.pairing_uri_file, &pairing_uri)?;

    println!(
        "webvm: awaiting direct approval over FIPS service {}",
        args.join_pubsub_port
    );
    let mut client = NostrJoinFipsPubsubClient::new(app)?;
    wait_for_approval(
        endpoint,
        approval_receiver,
        browser_identity,
        args.join_pubsub_port,
        &args.config,
        app,
        &mut client,
        shutdown,
    )
    .await?;
    remove_pairing_uri(&args.pairing_uri_file)?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn register_approval_route(
    endpoint: &FipsEndpoint,
    browser_identity: PeerIdentity,
    port: u16,
) -> Result<()> {
    endpoint
        .send_datagram(
            browser_identity,
            port,
            port,
            WEBVM_APPROVAL_ROUTE_REGISTRATION.to_vec(),
        )
        .await
        .context("failed to announce WebVM approval readiness")
}

#[cfg(any(target_os = "linux", test))]
fn webvm_mesh_ingress_hint(app: &AppConfig) -> Result<Vec<u8>> {
    let exit_node =
        normalize_nostr_pubkey(&app.exit_node).context("invalid WebVM mesh ingress identity")?;
    let exit_node = hex::decode(exit_node).context("invalid WebVM mesh ingress identity")?;
    let mut hint = Vec::with_capacity(WEBVM_MESH_INGRESS_HINT_MAGIC.len() + exit_node.len());
    hint.extend_from_slice(WEBVM_MESH_INGRESS_HINT_MAGIC);
    hint.extend_from_slice(&exit_node);
    Ok(hint)
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
    approval_receiver: &FipsEndpointServiceReceiver,
    browser_identity: PeerIdentity,
    port: u16,
    config_path: &Path,
    app: &mut AppConfig,
    client: &mut NostrJoinFipsPubsubClient,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let mut datagrams = Vec::<FipsEndpointServiceDatagram>::with_capacity(SERVICE_RECV_BATCH);
    let mut host_poll = tokio::time::interval_at(
        tokio::time::Instant::now() + PAIRING_HEARTBEAT_INTERVAL,
        PAIRING_HEARTBEAT_INTERVAL,
    );
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
                if let Err(error) = register_approval_route(endpoint, browser_identity, port).await {
                    eprintln!("webvm: approval readiness heartbeat failed; will retry: {error:#}");
                }
            }
            received = approval_receiver.recv_batch_into(&mut datagrams, SERVICE_RECV_BATCH) => {
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
                        let ack_datagram = approval_applied_ack_datagram(
                            &candidate.nostr_keys()?,
                            &applied,
                        )?;
                        persist_approval_ack(config_path, &ack_datagram)?;
                        candidate.save(config_path).with_context(|| {
                            format!("failed to persist approved config {}", config_path.display())
                        })?;
                        *app = candidate;
                        if let Err(error) = endpoint
                            .send_datagram(
                                datagram.source_peer,
                                port,
                                port,
                                ack_datagram.payload,
                            )
                            .await
                        {
                            eprintln!(
                                "webvm: approval applied ack send failed; duplicate approval will retry it: {error:#}"
                            );
                        }
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
    approval_receiver: FipsEndpointServiceReceiver,
    peer_status_snapshot: WebvmPeerStatusSnapshot,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let endpoint = Arc::clone(shared.endpoint());
    let browser_host = wait_for_browser_host(&endpoint, shutdown.clone()).await?;
    let browser_identity =
        PeerIdentity::from_npub(&browser_host).context("invalid WebVM browser FIPS mesh route")?;
    let mesh_ingress_hint = webvm_mesh_ingress_hint(&app)?;
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
    if let Err(error) = runtime.refresh_link_statuses().await {
        eprintln!("webvm: initial FIPS link snapshot failed: {error}");
    }
    replace_webvm_peer_status_snapshot(&peer_status_snapshot, runtime.peer_statuses())?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status = String::new();
    let approval_ack_datagram = load_approval_ack(&args.config)?;
    let approval_ack = approval_ack_datagram
        .as_ref()
        .map(parse_approval_applied_ack_datagram)
        .transpose()?;
    let mut approval_datagrams =
        Vec::<FipsEndpointServiceDatagram>::with_capacity(SERVICE_RECV_BATCH);
    let run_result = loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break Err(anyhow::Error::new(WebvmStop));
                }
            }
            _ = heartbeat.tick() => {
                if let Err(error) = endpoint
                    .send_datagram(
                        browser_identity,
                        args.join_pubsub_port,
                        args.join_pubsub_port,
                        mesh_ingress_hint.clone(),
                    )
                    .await
                {
                    eprintln!("webvm: mesh ingress hint failed; will retry: {error:#}");
                }
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
                replace_webvm_peer_status_snapshot(
                    &peer_status_snapshot,
                    runtime.peer_statuses(),
                )?;
            }
            received = approval_receiver.recv_batch_into(
                &mut approval_datagrams,
                SERVICE_RECV_BATCH,
            ) => {
                let Some(_) = received else {
                    break Err(anyhow!("WebVM join approval service closed"));
                };
                let (Some(ack_datagram), Some(ack)) =
                    (approval_ack_datagram.as_ref(), approval_ack.as_ref())
                else {
                    continue;
                };
                for datagram in &approval_datagrams {
                    let inbound = NostrJoinFipsPubsubDatagram {
                        source_port: datagram.source_port,
                        destination_port: datagram.destination_port,
                        payload: datagram.data.as_ref().to_vec(),
                    };
                    if approval_event_datagram_matches_ack(&inbound, ack)
                        && let Err(error) = endpoint
                            .send_datagram(
                                datagram.source_peer,
                                args.join_pubsub_port,
                                args.join_pubsub_port,
                                ack_datagram.payload.clone(),
                            )
                            .await
                    {
                        eprintln!("webvm: approval applied ack replay failed: {error:#}");
                    }
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
fn replace_webvm_peer_status_snapshot(
    snapshot: &WebvmPeerStatusSnapshot,
    statuses: Vec<MeshPeerStatus>,
) -> Result<()> {
    *snapshot
        .write()
        .map_err(|_| anyhow!("WebVM peer status snapshot lock poisoned"))? = Some(statuses);
    Ok(())
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

#[cfg(test)]
mod tests;
