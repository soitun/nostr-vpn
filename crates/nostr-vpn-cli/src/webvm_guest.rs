use super::*;

#[cfg(target_os = "linux")]
use fips_core::FipsEndpointServiceDatagram;
#[cfg(target_os = "linux")]
use fips_endpoint::{FipsEndpoint, PeerIdentity};
use nostr_vpn_core::join_pubsub::NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT;
#[cfg(target_os = "linux")]
use nostr_vpn_core::join_pubsub::{NostrJoinFipsPubsubClient, NostrJoinFipsPubsubDatagram};
#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(any(target_os = "linux", test))]
use std::io::Write as _;
#[cfg(all(unix, any(target_os = "linux", test)))]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(any(target_os = "linux", test))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::time::Instant;

#[cfg(any(target_os = "linux", test))]
const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
#[cfg(target_os = "linux")]
const MAX_BROWSER_FIPS_HOSTS: usize = 8;
#[cfg(target_os = "linux")]
const HOST_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const SUBSCRIBE_RETRY_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(target_os = "linux")]
const SERVICE_RECV_BATCH: usize = 8;

pub(crate) async fn run(args: WebvmGuestArgs) -> Result<()> {
    validate_args(&args)?;

    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        return Err(anyhow!("webvm-guest is supported only on Linux"));
    }

    #[cfg(target_os = "linux")]
    run_linux(args).await
}

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
    let setup_result = async {
        if app.active_network_opt().is_none() {
            pair_over_fips(&args, &endpoint, &mut app).await?;
        }
        validate_approved_config(&app)?;
        remove_pairing_uri(&args.pairing_uri_file)
    }
    .await;
    if let Err(error) = setup_result {
        let _ = host_network.stop().await;
        let _ = endpoint.shutdown().await;
        return Err(error);
    }
    run_tunnel(&args, app, shared, host_network).await
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
    let own_pubkey = app.own_nostr_pubkey_hex()?;
    let devices = app.participant_pubkeys_hex();
    if !devices.iter().any(|device| device == &own_pubkey) {
        return Err(anyhow!(
            "approved WebVM roster does not contain this device AppKey"
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

#[cfg(target_os = "linux")]
async fn pair_over_fips(
    args: &WebvmGuestArgs,
    endpoint: &FipsEndpoint,
    app: &mut AppConfig,
) -> Result<()> {
    let pairing_uri = app.pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)?;
    write_pairing_uri(&args.pairing_uri_file, &pairing_uri)?;

    endpoint
        .register_service(args.join_pubsub_port)
        .await
        .context("failed to register WebVM join pubsub service")?;

    println!(
        "webvm-guest: awaiting approval over Ethernet FIPS service {}",
        args.join_pubsub_port
    );
    let mut client = NostrJoinFipsPubsubClient::new(app)?;
    let pairing_result = wait_for_approval(endpoint, &args.config, app, &mut client).await;
    close_subscriptions(endpoint, &client, args.join_pubsub_port).await;
    pairing_result?;
    remove_pairing_uri(&args.pairing_uri_file)?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn wait_for_approval(
    endpoint: &FipsEndpoint,
    config_path: &Path,
    app: &mut AppConfig,
    client: &mut NostrJoinFipsPubsubClient,
) -> Result<()> {
    let subscribe = client.subscribe_datagram(app)?;
    let mut subscribed = HashMap::<String, (u64, Instant)>::new();
    let mut datagrams = Vec::<FipsEndpointServiceDatagram>::with_capacity(SERVICE_RECV_BATCH);
    let mut host_poll = tokio::time::interval(HOST_POLL_INTERVAL);
    host_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                return Err(anyhow!("WebVM guest pairing interrupted"));
            }
            _ = host_poll.tick() => {
                subscribe_connected_hosts(endpoint, &subscribe, &mut subscribed).await?;
            }
            received = endpoint.recv_service_datagram_batch_into(
                &mut datagrams,
                SERVICE_RECV_BATCH,
            ) => {
                let Some(_) = received else {
                    return Err(anyhow!("WebVM pairing FIPS endpoint closed"));
                };
                for datagram in &datagrams {
                    let source_npub = datagram.source_peer.npub();
                    if !subscribed.contains_key(&source_npub) {
                        return Err(anyhow!(
                            "join approval arrived from an unsubscribed FIPS peer"
                        ));
                    }
                    let inbound = NostrJoinFipsPubsubDatagram {
                        source_port: datagram.source_port,
                        destination_port: datagram.destination_port,
                        payload: datagram.data.as_ref().to_vec(),
                    };
                    println!("webvm-guest: received approval candidate over FIPS");
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
                            "webvm-guest: approval applied for network {} by {}",
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
async fn subscribe_connected_hosts(
    endpoint: &FipsEndpoint,
    subscribe: &NostrJoinFipsPubsubDatagram,
    subscribed: &mut HashMap<String, (u64, Instant)>,
) -> Result<()> {
    let mut hosts = endpoint
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
        .collect::<Vec<_>>();
    hosts.sort_by(|left, right| left.npub.cmp(&right.npub));
    if hosts.len() > MAX_BROWSER_FIPS_HOSTS {
        return Err(anyhow!(
            "too many browser FIPS hosts in discovery scope ({} > {MAX_BROWSER_FIPS_HOSTS})",
            hosts.len()
        ));
    }
    let connected = hosts
        .iter()
        .map(|peer| peer.npub.clone())
        .collect::<std::collections::HashSet<_>>();
    subscribed.retain(|npub, _| connected.contains(npub));

    for host in hosts {
        let resend = subscribed.get(&host.npub).is_none_or(|(link_id, sent_at)| {
            *link_id != host.link_id || sent_at.elapsed() >= SUBSCRIBE_RETRY_INTERVAL
        });
        if !resend {
            continue;
        }
        let remote = PeerIdentity::from_npub(&host.npub)
            .with_context(|| format!("invalid browser FIPS host npub {}", host.npub))?;
        endpoint
            .send_datagram(
                remote,
                subscribe.source_port,
                subscribe.destination_port,
                subscribe.payload.clone(),
            )
            .await
            .with_context(|| format!("failed to subscribe through FIPS host {}", host.npub))?;
        subscribed.insert(host.npub, (host.link_id, Instant::now()));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn close_subscriptions(
    endpoint: &FipsEndpoint,
    client: &NostrJoinFipsPubsubClient,
    service_port: u16,
) {
    let Ok(close) = client.close_datagram() else {
        return;
    };
    let Ok(peers) = endpoint.peers().await else {
        return;
    };
    for peer in peers.into_iter().filter(|peer| peer.connected) {
        let Ok(remote) = PeerIdentity::from_npub(&peer.npub) else {
            continue;
        };
        let _ = endpoint
            .send_datagram(remote, service_port, service_port, close.payload.clone())
            .await;
    }
}

#[cfg(target_os = "linux")]
async fn run_tunnel(
    args: &WebvmGuestArgs,
    mut app: AppConfig,
    shared: crate::fips_private_mesh::FipsSharedEndpoint,
    host_network: crate::fips_private_mesh::WebvmFipsHostNetworkRuntime,
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
    host_network.enable_vpn_dns();
    println!(
        "webvm-guest: Nostr VPN tunnel {} over Ethernet {}",
        runtime.iface(),
        args.ethernet_interface
    );

    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status = String::new();
    let run_result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break Ok(()),
            _ = heartbeat.tick() => {
                let network_id = app.effective_network_id();
                if let Err(error) = runtime.ping_peers(&network_id, unix_timestamp()).await {
                    eprintln!("webvm-guest: FIPS peer ping failed: {error}");
                }
                if let Err(error) = runtime.refresh_link_statuses().await {
                    eprintln!("webvm-guest: FIPS link snapshot failed: {error}");
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
                    Err(error) => eprintln!("webvm-guest: FIPS event handling failed: {error}"),
                }
                if let Err(error) = runtime.refresh_peer_dependent_routes().await {
                    eprintln!("webvm-guest: route refresh failed: {error}");
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
    fn first_boot_persists_one_stable_full_join_request() {
        let path = temp_path("config").with_extension("toml");
        let first = load_or_initialize_config(&path, 1_778_998_000).expect("first boot");
        let first_uri = first
            .pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
            .expect("first URI");
        let second = load_or_initialize_config(&path, 1_778_998_100).expect("second boot");
        let second_uri = second
            .pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
            .expect("second URI");
        assert_eq!(first_uri, second_uri);
        assert!(first_uri.starts_with(JOIN_REQUEST_LINK_PREFIX));
        let pending = second.pending_nostr_join_request.expect("pending request");
        assert_ne!(
            pending.request.request_pubkey,
            pending.request.device_app_key_pubkey
        );
        assert!(pending.request.request_secret.len() >= 32);

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
        app.networks[0].devices = vec![own_pubkey.clone(), exit_pubkey.clone()];
        app.networks[0].admins = vec![own_pubkey.clone()];
        app.exit_node = exit_pubkey;
        app.ensure_defaults();

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
