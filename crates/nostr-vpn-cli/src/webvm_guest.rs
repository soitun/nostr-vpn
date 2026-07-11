use super::*;

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(any(target_os = "linux", test))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::Arc;

pub(crate) async fn run(args: WebvmGuestArgs) -> Result<()> {
    validate_args(&args)?;

    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        Err(anyhow!("webvm-guest is supported only on Linux"))
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

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebvmGuestMode {
    FipsOnly,
    Vpn,
}

#[cfg(any(target_os = "linux", test))]
fn webvm_guest_mode(app: &AppConfig) -> WebvmGuestMode {
    if app.active_network_opt().is_some() {
        WebvmGuestMode::Vpn
    } else {
        WebvmGuestMode::FipsOnly
    }
}

#[cfg(target_os = "linux")]
async fn run_linux(args: WebvmGuestArgs) -> Result<()> {
    validate_ethernet_underlay_is_layer2_only(args.ethernet_interface.trim())?;
    let app = load_or_initialize_config(&args.config)?;
    let mode = webvm_guest_mode(&app);
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
    if mode == WebvmGuestMode::FipsOnly {
        return run_fips_only(shared, host_network).await;
    }
    run_tunnel(&args, app, shared, host_network).await
}

#[cfg(target_os = "linux")]
async fn run_fips_only(
    shared: crate::fips_private_mesh::FipsSharedEndpoint,
    host_network: crate::fips_private_mesh::WebvmFipsHostNetworkRuntime,
) -> Result<()> {
    let endpoint = Arc::clone(shared.endpoint());
    println!("webvm-guest: waiting for a Nostr VPN invite; .fips remains active");
    let run_result = tokio::signal::ctrl_c()
        .await
        .context("failed to wait for WebVM guest shutdown");
    let host_result = host_network.stop().await;
    let endpoint_result = endpoint.shutdown().await;
    drop(shared);

    run_result?;
    host_result.context("failed to stop WebVM .fips host network")?;
    endpoint_result.context("failed to stop WebVM FIPS endpoint")
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
fn load_or_initialize_config(path: &Path) -> Result<AppConfig> {
    let exists = path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    let mut app = if exists {
        AppConfig::load(path).with_context(|| format!("failed to load {}", path.display()))?
    } else {
        AppConfig::generated_without_networks()
    };
    app.ensure_defaults();

    let changed = app.clear_pending_nostr_join_request();
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

#[cfg(target_os = "linux")]
async fn run_tunnel(
    args: &WebvmGuestArgs,
    mut app: AppConfig,
    shared: crate::fips_private_mesh::FipsSharedEndpoint,
    host_network: crate::fips_private_mesh::WebvmFipsHostNetworkRuntime,
) -> Result<()> {
    let endpoint = Arc::clone(shared.endpoint());
    if app.exit_node.trim().is_empty()
        && let Some(inviter) = app
            .active_network_opt()
            .map(|network| network.invite_inviter.clone())
            .filter(|inviter| !inviter.is_empty())
    {
        app.exit_node = inviter;
        app.save(&args.config)?;
    }
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
    let mut approved = validate_approved_config(&app).is_ok();
    if approved {
        if let Err(error) = host_network.enable_vpn_dns(&app.exit_node) {
            let _ = host_network.stop().await;
            let _ = runtime.stop().await;
            return Err(error);
        }
        println!(
            "webvm-guest: Nostr VPN tunnel {} over Ethernet {}",
            runtime.iface(),
            args.ethernet_interface
        );
    } else {
        println!("webvm-guest: awaiting signed roster over FIPS");
    }

    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status = String::new();
    let mut sent_join_requests = HashMap::new();
    let run_result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break Ok(()),
            _ = heartbeat.tick() => {
                let network_id = app.effective_network_id();
                let now = unix_timestamp();
                if let Err(error) = send_pending_fips_join_requests(
                    &runtime,
                    &app,
                    &mut sent_join_requests,
                    now,
                ).await {
                    eprintln!("webvm-guest: FIPS join request failed: {error}");
                }
                if let Err(error) = runtime.ping_peers(&network_id, now).await {
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
                        if let Err(error) = validate_approved_config(&app) {
                            eprintln!("webvm-guest: signed roster did not complete approval: {error}");
                            continue;
                        }
                        tunnel = match build_tunnel_config(args, &app) {
                            Ok(tunnel) => tunnel,
                            Err(error) => break Err(error),
                        };
                        if let Err(error) = runtime.apply_config(tunnel.clone()).await {
                            break Err(error).context("failed to apply updated WebVM roster");
                        }
                        if !approved {
                            if let Err(error) = host_network.enable_vpn_dns(&app.exit_node) {
                                break Err(error).context("failed to enable approved WebVM DNS");
                            }
                            approved = true;
                            println!("webvm-guest: signed roster accepted over FIPS");
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
    fn first_boot_does_not_create_transport_specific_approval_state() {
        let path = temp_path("config").with_extension("toml");
        let first = load_or_initialize_config(&path).expect("first boot");
        let second = load_or_initialize_config(&path).expect("second boot");
        assert!(first.networks.is_empty());
        assert!(second.networks.is_empty());
        assert!(first.pending_nostr_join_request.is_none());
        assert!(second.pending_nostr_join_request.is_none());
        assert_eq!(webvm_guest_mode(&first), WebvmGuestMode::FipsOnly);
        assert_eq!(webvm_guest_mode(&second), WebvmGuestMode::FipsOnly);

        AppConfig::delete_persisted_secrets_for_path(&path).expect("delete secrets");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_webvm_arguments_are_rejected_before_networking() {
        let args = WebvmGuestArgs {
            config: PathBuf::from("/tmp/config.toml"),
            ethernet_interface: "eth0".to_string(),
            discovery_scope: "fips-overlay-v1".to_string(),
            tun_interface: "eth0".to_string(),
        };
        assert!(
            validate_args(&args)
                .expect_err("same TUN and Ethernet interface")
                .to_string()
                .contains("must differ")
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
