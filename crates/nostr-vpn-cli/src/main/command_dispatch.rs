fn main() -> Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "warn,nostr_relay_pool=off,boringtun::noise::timers=error",
        )
    });
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cli = Cli::parse();
    run_cli(cli)
}

fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        #[cfg(target_os = "windows")]
        Command::Daemon(args) if args.service => run_windows_service_dispatcher(args),
        command => run_command_on_runtime(command),
    }
}

fn run_command_on_runtime(command: Command) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let handle = thread::Builder::new()
            .name("nvpn-runtime".to_string())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || run_command_with_runtime(command))
            .context("failed to spawn nvpn runtime thread")?;
        handle.join().map_err(|panic| {
            if let Some(message) = panic.downcast_ref::<&str>() {
                anyhow!("nvpn runtime thread panicked: {message}")
            } else if let Some(message) = panic.downcast_ref::<String>() {
                anyhow!("nvpn runtime thread panicked: {message}")
            } else {
                anyhow!("nvpn runtime thread panicked")
            }
        })?
    }

    #[cfg(not(target_os = "windows"))]
    {
        run_command_with_runtime(command)
    }
}

fn run_command_with_runtime(command: Command) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    runtime.block_on(run_command(command))
}

async fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Init {
            config,
            force,
            devices,
        } => {
            let path = config.unwrap_or_else(default_config_path);
            init_config(&path, force, devices)?;
        }
        Command::Version(args) => {
            print_version(args)?;
        }
        Command::Update(args) => {
            updater::run_update(args).await?;
        }
        Command::InstallCli(args) => {
            install_cli(args)?;
        }
        Command::UninstallCli(args) => {
            uninstall_cli(args)?;
        }
        Command::Service(args) => {
            service_management::run_service_command(args)?;
        }
        Command::Start(args) => {
            start_session(args).await?;
        }
        Command::Stop(args) => {
            stop_daemon(args)?;
        }
        Command::RepairNetwork(args) => {
            repair_network(args)?;
        }
        Command::Reload(args) => {
            reload_daemon(args)?;
        }
        Command::Pause(args) => {
            control_daemon(args, DaemonControlRequest::Pause)?;
        }
        Command::Resume(args) => {
            control_daemon(args, DaemonControlRequest::Resume)?;
        }
        Command::Connect(args) => {
            connect_vpn(args).await?;
        }
        Command::Status(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let (app, network_id) =
                load_config_with_overrides(&config_path, args.network_id, args.devices)?;
            let daemon = daemon_status(&config_path)?;

            let daemon_peers: Option<Vec<DaemonPeerState>> = if daemon.running {
                daemon.state.as_ref().map(|state| {
                    state
                        .peers
                        .iter()
                        .filter(|peer| {
                            !peer.node_id.is_empty()
                                || !peer.tunnel_ip.is_empty()
                                || !peer.endpoint.is_empty()
                        })
                        .cloned()
                        .collect()
                })
            } else {
                None
            };

            let (peers, expected_peers, peer_count, mesh_ready, status_source) =
                if let Some(daemon_peers) = daemon_peers.as_ref() {
                    let state = daemon.state.as_ref().expect("daemon peers implies state");
                    let peers = daemon_peers
                        .iter()
                        .map(|peer| PeerAnnouncement {
                            node_id: if peer.node_id.is_empty() {
                                peer.participant_pubkey.clone()
                            } else {
                                peer.node_id.clone()
                            },
                            public_key: peer.public_key.clone(),
                            endpoint: peer.endpoint.clone(),
                            local_endpoint: None,
                            public_endpoint: None,
                            tunnel_ip: peer.tunnel_ip.clone(),
                            advertised_routes: peer.advertised_routes.clone(),
                            timestamp: peer.last_mesh_seen_at,
                        })
                        .collect::<Vec<_>>();
                    (
                        peers,
                        state.expected_peer_count,
                        state.connected_peer_count,
                        state.mesh_ready,
                        "daemon",
                    )
                } else {
                    let peers = configured_fips_peer_announcements(&app, &network_id);
                    let expected = expected_peer_count(&app);
                    (peers, expected, 0, false, "config")
            };

            if args.json {
                let endpoint = status_endpoint(&app, &daemon);
                let listen_port = status_listen_port(&app, &daemon);
                #[cfg(feature = "paid-exit")]
                let paid_exit_status = paid_exit_status_json(&app);
                #[cfg(not(feature = "paid-exit"))]
                let paid_exit_status = serde_json::Value::Null;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "status_source": status_source,
                        "network_id": network_id,
                        "magic_dns_suffix": app.magic_dns_suffix,
                        "autoconnect": app.autoconnect,
                        "node_id": app.node.id,
                        "tunnel_ip": runtime_local_tunnel_ip(&app, &network_id),
                        "endpoint": endpoint,
                        "configured_endpoint": app.node.endpoint,
                        "listen_port": listen_port,
                        "configured_listen_port": app.node.listen_port,
                        "exit_node": if app.exit_node.is_empty() {
                            None::<String>
                        } else {
                            Some(app.exit_node.clone())
                        },
                        "exit_node_leak_protection": app.exit_node_leak_protection,
                        "advertise_exit_node": app.node.advertise_exit_node,
                        "advertised_routes": app.node.advertised_routes,
                        "effective_advertised_routes": runtime_effective_advertised_routes(&app),
                        "fips_host_tunnel_enabled": app.fips_host_tunnel_enabled,
                        "connect_to_non_roster_fips_peers": app.connect_to_non_roster_fips_peers,
                        "fips_nostr_discovery_enabled": app.fips_nostr_discovery_enabled,
                        "fips_bootstrap_enabled": app.fips_bootstrap_enabled,
                        "fips_host_inbound_tcp_ports": app.fips_host_inbound_tcp_ports,
                        "wireguard_exit": wireguard_exit_status_json(&app),
                        "paid_exit": paid_exit_status,
                        "daemon": daemon_status_json_value(&daemon),
                        "expected_peer_count": expected_peers,
                        "peer_count": peer_count,
                        "mesh_ready": mesh_ready,
                        "peers": status_json_peers(daemon_peers.as_deref(), &peers),
                    }))?
                );
            } else {
                let endpoint = status_endpoint(&app, &daemon);
                let listen_port = status_listen_port(&app, &daemon);
                println!("network: {network_id}");
                println!("magic_dns_suffix: {}", app.magic_dns_suffix);
                println!("autoconnect: {}", app.autoconnect);
                println!("node: {}", app.node.id);
                println!("tunnel_ip: {}", runtime_local_tunnel_ip(&app, &network_id));
                println!("endpoint: {endpoint}");
                println!("listen_port: {listen_port}");
                if endpoint != app.node.endpoint {
                    println!("configured_endpoint: {}", app.node.endpoint);
                }
                if listen_port != app.node.listen_port {
                    println!("configured_listen_port: {}", app.node.listen_port);
                }
                if app.exit_node.is_empty() {
                    println!("exit_node: none");
                } else {
                    println!("exit_node: {}", app.exit_node);
                }
                println!(
                    "exit_node_leak_protection: {}",
                    app.exit_node_leak_protection
                );
                println!("advertise_exit_node: {}", app.node.advertise_exit_node);
                println!("fips_host_tunnel_enabled: {}", app.fips_host_tunnel_enabled);
                println!(
                    "connect_to_non_roster_fips_peers: {}",
                    app.connect_to_non_roster_fips_peers
                );
                println!(
                    "fips_nostr_discovery_enabled: {}",
                    app.fips_nostr_discovery_enabled
                );
                println!("fips_bootstrap_enabled: {}", app.fips_bootstrap_enabled);
                if !app.fips_host_inbound_tcp_ports.is_empty() {
                    println!(
                        "fips_host_inbound_tcp_ports: {}",
                        app.fips_host_inbound_tcp_ports
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                }
                println!(
                    "wireguard_exit: {}",
                    if app.wireguard_exit.enabled {
                        if app.wireguard_exit.configured() {
                            "enabled"
                        } else {
                            "enabled (incomplete)"
                        }
                    } else {
                        "disabled"
                    }
                );
                if app.wireguard_exit.enabled {
                    println!("wireguard_exit_interface: {}", app.wireguard_exit.interface);
                    println!("wireguard_exit_address: {}", app.wireguard_exit.address);
                    println!("wireguard_exit_endpoint: {}", app.wireguard_exit.endpoint);
                }
                #[cfg(feature = "paid-exit")]
                print_paid_exit_status(&app);
                let effective_routes = runtime_effective_advertised_routes(&app);
                if effective_routes.is_empty() {
                    println!("advertised_routes: none");
                } else {
                    println!("advertised_routes: {}", effective_routes.join(", "));
                }
                if daemon.running {
                    println!("daemon: running (pid {})", daemon.pid.unwrap_or_default());
                    if let Some(state) = daemon.state.as_ref() {
                        if !state.binary_version.is_empty() {
                            println!("daemon_version: {}", state.binary_version);
                        }
                        if !state.fips_core_version.is_empty() {
                            println!("daemon_fips_core_version: {}", state.fips_core_version);
                        }
                        println!("vpn_status: {}", state.vpn_status);
                    }
                } else {
                    println!("daemon: stopped");
                }
                println!("status_source: {status_source}");
                if expected_peers > 0 {
                    println!("mesh_progress: {}/{}", peer_count, expected_peers);
                    println!("mesh_ready: {mesh_ready}");
                }
                if let Some(daemon_peers) = daemon_peers.as_ref() {
                    let reachable_count = daemon_peers.iter().filter(|p| p.reachable).count();
                    println!(
                        "peers: {} total, {reachable_count} reachable",
                        daemon_peers.len()
                    );
                    let now = unix_timestamp();
                    for peer in daemon_peers {
                        print_daemon_peer_line(peer, now);
                    }
                } else {
                    println!("peers: {}", peers.len());
                    for peer in &peers {
                        if peer.advertised_routes.is_empty() {
                            println!("  {} {} {}", peer.node_id, peer.tunnel_ip, peer.endpoint);
                        } else {
                            println!(
                                "  {} {} {} routes={}",
                                peer.node_id,
                                peer.tunnel_ip,
                                peer.endpoint,
                                peer.advertised_routes.join(",")
                            );
                        }
                    }
                }
            }
        }
        Command::Set(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let mut app = load_or_default_config(&config_path)?;

            if let Some(value) = args.network_id {
                app.set_active_network_id(&value)?;
            }
            if let Some(value) = args.node_name {
                app.node_name = value;
            }
            if let Some(value) = args.node_id {
                app.node.id = value;
            }
            if let Some(value) = args.endpoint {
                app.node.endpoint = value;
            }
            if let Some(value) = args.tunnel_ip {
                app.node.tunnel_ip = value;
            }
            if let Some(value) = args.listen_port {
                app.node.listen_port = value;
            }
            if let Some(value) = args.exit_node {
                app.exit_node = parse_exit_node_arg(&value)?.unwrap_or_default();
            }
            if let Some(value) = args.exit_node_leak_protection {
                app.exit_node_leak_protection = value;
            }
            if let Some(value) = args.advertise_routes {
                app.node.advertised_routes = parse_advertised_routes_arg(&value)?;
            }
            if let Some(value) = args.advertise_exit_node {
                app.node.advertise_exit_node = value;
            }
            #[cfg(feature = "paid-exit")]
            {
                if let Some(value) = args.paid_exit_enabled {
                    app.paid_exit.enabled = value;
                }
                if let Some(value) = args.paid_exit_meter {
                    app.paid_exit.pricing.meter =
                        value.parse::<PaidRouteMeter>().map_err(|error| anyhow!(error))?;
                }
                if let Some(value) = args.paid_exit_upstream {
                    app.paid_exit.access.upstream = value
                        .parse::<PaidExitUpstream>()
                        .map_err(|error| anyhow!(error))?;
                }
                if let Some(value) = args.paid_exit_price_msat {
                    app.paid_exit.pricing.price_msat = value;
                }
                if let Some(value) = args.paid_exit_per_units.as_deref() {
                    app.paid_exit.pricing.per_units = paid_exit_parse_pricing_units_arg(
                        value,
                        app.paid_exit.pricing.meter,
                        "--paid-exit-per-units",
                    )?;
                }
                if let Some(value) = args.paid_exit_accepted_mints {
                    app.paid_exit.channel.accepted_mints = parse_csv_arg(&value);
                }
                if let Some(value) = args.paid_exit_country_code {
                    app.paid_exit.location.country_code = value;
                }
                if let Some(value) = args.paid_exit_region {
                    app.paid_exit.location.region = value;
                }
                if let Some(value) = args.paid_exit_asn {
                    app.paid_exit.location.asn = Some(value);
                }
                if let Some(value) = args.paid_exit_network_class {
                    app.paid_exit.location.network_class = value
                        .parse::<ExitNetworkClass>()
                        .map_err(|error| anyhow!(error))?;
                }
                if let Some(value) = args.paid_exit_ipv4 {
                    app.paid_exit.ip_support.ipv4 = value;
                }
                if let Some(value) = args.paid_exit_ipv6 {
                    app.paid_exit.ip_support.ipv6 = value;
                }
                if let Some(value) = args.paid_exit_max_channel_capacity_sat {
                    app.paid_exit.channel.max_channel_capacity_sat = value;
                }
                if let Some(value) = args.paid_exit_channel_expiry_secs {
                    app.paid_exit.channel.channel_expiry_secs = value;
                }
                if let Some(value) = args.paid_exit_free_probe_units.as_deref() {
                    app.paid_exit.channel.free_probe_units = paid_exit_parse_traffic_units_arg(
                        value,
                        app.paid_exit.pricing.meter,
                        "--paid-exit-free-probe-units",
                    )?;
                }
                if let Some(value) = args.paid_exit_grace_units.as_deref() {
                    app.paid_exit.channel.grace_units = paid_exit_parse_traffic_units_arg(
                        value,
                        app.paid_exit.pricing.meter,
                        "--paid-exit-grace-units",
                    )?;
                }
            }
            if let Some(value) = args.wireguard_exit_enabled {
                app.wireguard_exit.enabled = value;
            }
            if args.wireguard_exit_config.is_some() && args.wireguard_exit_config_file.is_some() {
                return Err(anyhow!(
                    "use either --wireguard-exit-config or --wireguard-exit-config-file, not both"
                ));
            }
            if let Some(value) = args.wireguard_exit_config {
                let enabled = app.wireguard_exit.enabled;
                let mut parsed = parse_wireguard_exit_config(&value)?;
                parsed.enabled = enabled;
                app.wireguard_exit = parsed;
            }
            if let Some(path) = args.wireguard_exit_config_file {
                let value = fs::read_to_string(&path).with_context(|| {
                    format!("failed to read WireGuard config {}", path.display())
                })?;
                let enabled = app.wireguard_exit.enabled;
                let mut parsed = parse_wireguard_exit_config(&value)?;
                parsed.enabled = enabled;
                app.wireguard_exit = parsed;
            }
            if let Some(value) = args.wireguard_exit_interface {
                app.wireguard_exit.interface = value;
            }
            if let Some(value) = args.wireguard_exit_address {
                app.wireguard_exit.address = value;
            }
            if let Some(value) = args.wireguard_exit_private_key {
                app.wireguard_exit.private_key = value;
            }
            if let Some(value) = args.wireguard_exit_peer_public_key {
                app.wireguard_exit.peer_public_key = value;
            }
            if let Some(value) = args.wireguard_exit_peer_preshared_key {
                app.wireguard_exit.peer_preshared_key = value;
            }
            if let Some(value) = args.wireguard_exit_endpoint {
                app.wireguard_exit.endpoint = value;
            }
            if let Some(value) = args.wireguard_exit_allowed_ips {
                app.wireguard_exit.allowed_ips = parse_advertised_routes_arg(&value)?;
            }
            if let Some(value) = args.wireguard_exit_dns {
                app.wireguard_exit.dns = parse_csv_arg(&value);
            }
            if let Some(value) = args.wireguard_exit_mtu {
                app.wireguard_exit.mtu = value;
            }
            if let Some(value) = args.wireguard_exit_keepalive {
                app.wireguard_exit.persistent_keepalive_secs = value;
            }
            if let Some(value) = args.autoconnect {
                app.autoconnect = value;
            }
            if let Some(value) = args.join_requests_enabled {
                let network_id = app
                    .active_network_opt()
                    .ok_or_else(|| anyhow!("activate a network before changing join requests"))?
                    .id
                    .clone();
                app.set_network_join_requests_enabled(&network_id, value)?;
            }
            if let Some(value) = args.fips_advertise_public_endpoint {
                app.fips_advertise_public_endpoint = value;
            }
            if let Some(value) = args.fips_host_tunnel_enabled {
                app.fips_host_tunnel_enabled = value;
            }
            if let Some(value) = args.connect_to_non_roster_fips_peers {
                app.connect_to_non_roster_fips_peers = value;
            }
            if let Some(value) = args.fips_nostr_discovery_enabled {
                app.fips_nostr_discovery_enabled = value;
            }
            if let Some(value) = args.fips_bootstrap_enabled {
                app.fips_bootstrap_enabled = value;
            }
            if let Some(value) = args.fips_host_inbound_tcp_ports {
                app.fips_host_inbound_tcp_ports = parse_tcp_ports_arg(&value)?;
            }
            if !args.fips_peer_endpoints.is_empty() {
                app.fips_peer_endpoints = parse_fips_peer_endpoint_args(&args.fips_peer_endpoints)?;
            }
            apply_devices_override(&mut app, args.devices)?;
            app.ensure_defaults();
            maybe_autoconfigure_node(&mut app);
            app.save(&config_path)?;
            maybe_reload_running_daemon(&config_path);

            if args.json {
                println!("{}", serde_json::to_string_pretty(&app)?);
            } else {
                println!("saved {}", config_path.display());
                println!("network_id={}", app.effective_network_id());
                println!("node_id={}", app.node.id);
            }
        }
        Command::CreateInvite(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let app = load_or_default_config(&config_path)?;
            let invite = active_network_invite_code(&app)?;

            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "network_id": app.effective_network_id(),
                        "invite": invite,
                    }))?
                );
            } else {
                println!("{invite}");
            }
        }
        Command::ImportInvite(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let mut app = load_or_default_config(&config_path)?;
            let invite = parse_network_invite(&args.invite)?;
            apply_network_invite_to_active_network(&mut app, &invite)?;
            let join_request_queued = queue_active_network_join_request(&mut app)?;
            app.ensure_defaults();
            maybe_autoconfigure_node(&mut app);
            app.save(&config_path)?;
            maybe_reload_running_daemon(&config_path);

            if args.json {
                println!("{}", serde_json::to_string_pretty(&app)?);
            } else {
                println!("saved {}", config_path.display());
                println!("network_id={}", app.effective_network_id());
                println!("invite_imported={}", app.active_network().name);
                println!("join_request_queued={join_request_queued}");
            }
        }
        Command::InviteBroadcast(args) => {
            run_invite_broadcast(args)?;
        }
        Command::Discover(args) => {
            run_discover(args)?;
        }
        Command::AddDevice(args) => {
            update_active_network_roster(args, RosterEditAction::AddDevice).await?;
        }
        Command::RemoveDevice(args) => {
            update_active_network_roster(args, RosterEditAction::RemoveDevice).await?;
        }
        Command::AddAdmin(args) => {
            update_active_network_roster(args, RosterEditAction::AddAdmin).await?;
        }
        Command::RemoveAdmin(args) => {
            update_active_network_roster(args, RosterEditAction::RemoveAdmin).await?;
        }
        Command::Ping(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let (app, network_id) =
                load_config_with_overrides(&config_path, args.network_id, args.devices)?;
            let peers = configured_fips_peer_announcements(&app, &network_id);

            let target = resolve_ping_target(&args.target, &peers).ok_or_else(|| {
                anyhow!("target '{}' did not match an IP or known peer", args.target)
            })?;

            run_ping(&target, args.count, args.timeout_secs)?;
        }
        Command::Doctor(args) => {
            run_doctor(args).await?;
        }
        Command::Ip(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let (app, network_id) =
                load_config_with_overrides(&config_path, args.network_id, args.devices)?;

            if !args.peer {
                let tunnel_ip = runtime_local_tunnel_ip(&app, &network_id);
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "node_id": app.node.id,
                            "tunnel_ip": tunnel_ip,
                            "ip": strip_cidr(&tunnel_ip),
                        }))?
                    );
                } else {
                    println!("{}", strip_cidr(&tunnel_ip));
                }
            } else {
                let peer_ips = runtime_peer_tunnel_ips(&app, &network_id);
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&peer_ips)?);
                } else {
                    for ip in peer_ips {
                        println!("{}", strip_cidr(&ip));
                    }
                }
            }
        }
        Command::Whois(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            let (app, network_id) =
                load_config_with_overrides(&config_path, args.network_id, args.devices)?;
            let peers = configured_fips_peer_announcements(&app, &network_id);

            let found = peers
                .iter()
                .find(|peer| {
                    peer.node_id == args.query
                        || peer.public_key == args.query
                        || peer.tunnel_ip == args.query
                        || strip_cidr(&peer.tunnel_ip) == args.query
                })
                .cloned();

            let Some(peer) = found else {
                return Err(anyhow!("no peer found for '{}'", args.query));
            };

            if args.json {
                println!("{}", serde_json::to_string_pretty(&peer)?);
            } else {
                println!("node_id={}", peer.node_id);
                println!("public_key={}", peer.public_key);
                println!("tunnel_ip={}", peer.tunnel_ip);
                println!("endpoint={}", peer.endpoint);
                println!("timestamp={}", peer.timestamp);
            }
        }
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        Command::WgUpstreamTest(args) => {
            run_wg_upstream_test(args).await?;
        }
        #[cfg(feature = "paid-exit")]
        Command::PaidExit(args) => {
            run_paid_exit_command(args).await?;
        }
        Command::ApplyConfig(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            apply_config_file(&args.source, &config_path)?;
        }
        Command::ApplyConfigDaemon(args) => {
            let config_path = args.config.unwrap_or_else(default_config_path);
            apply_config_via_running_daemon(&args.source, &config_path)?;
        }
        Command::Daemon(args) => daemon_vpn(args).await?,
    }

    Ok(())
}

fn status_json_peers(
    daemon_peers: Option<&[DaemonPeerState]>,
    configured_peers: &[PeerAnnouncement],
) -> serde_json::Value {
    match daemon_peers {
        Some(peers) => serde_json::json!(peers),
        None => serde_json::json!(configured_peers),
    }
}
