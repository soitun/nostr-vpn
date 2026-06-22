pub(crate) async fn daemon_vpn(args: DaemonArgs) -> Result<()> {
    if args.iface.trim().is_empty() {
        return Err(anyhow!("--iface must not be empty"));
    }

    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    if args.service
        && let Err(error) = redirect_stdio_to_daemon_log(&config_path)
    {
        eprintln!("daemon: failed to redirect service log: {error}");
    }
    if let Err(error) = compact_daemon_log_if_needed(&config_path) {
        eprintln!("daemon: failed to compact service log: {error}");
    }
    #[cfg(any(target_os = "macos", test))]
    crate::ensure_macos_connect_privileges(&config_path)?;
    ensure_no_other_daemon_processes_for_config(&config_path, std::process::id())?;
    #[cfg(target_os = "macos")]
    if let Err(error) = repair_saved_network_state(&config_path) {
        eprintln!("daemon: failed to repair saved macOS network state: {error}");
    }
    let pid_file = daemon_pid_file_path(&config_path);
    if let Err(error) = write_daemon_pid_record(
        &pid_file,
        &DaemonPidRecord {
            pid: std::process::id(),
            config_path: config_path.display().to_string(),
            started_at: unix_timestamp(),
        },
    ) {
        eprintln!(
            "daemon: failed to write pid file {}: {error}",
            pid_file.display()
        );
    }
    let network_override = args.network_id.clone();
    let participants_override = args.devices.clone();
    let (mut app, mut network_id) = load_config_with_overrides(
        &config_path,
        network_override.clone(),
        participants_override.clone(),
    )?;
    let mut own_pubkey = app.own_nostr_pubkey_hex().ok();
    let mut expected_peers = expected_peer_count(&app);
    #[cfg(not(feature = "embedded-fips"))]
    {
        return Err(anyhow!(
            "embedded FIPS private mesh requires building nvpn with the embedded-fips feature"
        ));
    }
    let state_file = daemon_state_file_path(&config_path);
    #[cfg(feature = "embedded-fips")]
    crate::fips_private_mesh::purge_legacy_fips_endpoint_cache(&config_path);
    let _ = fs::remove_file(daemon_control_file_path(&config_path));
    #[cfg(feature = "embedded-fips")]
    let recent_peers_path = crate::recent_peers_store::recent_peers_file_path(&config_path);
    #[cfg(feature = "embedded-fips")]
    let mut recent_peers =
        match crate::recent_peers_store::load_recent_peers(&recent_peers_path, unix_timestamp()) {
            Ok(state) => state,
            Err(error) => {
                eprintln!(
                    "daemon: failed to load recent peers cache {}: {error}",
                    recent_peers_path.display()
                );
                nostr_vpn_core::recent_peers::RecentPeerEndpoints::default()
            }
        };
    #[cfg(feature = "embedded-fips")]
    let mut fips_join_request_sends: HashMap<String, u64> = HashMap::new();
    #[cfg(feature = "embedded-fips")]
    let mut pending_fips_roster_recipients: HashSet<String> = HashSet::new();
    #[cfg(feature = "embedded-fips")]
    let mut fips_roster_sync_state = FipsRosterSyncState::default();
    #[cfg(feature = "embedded-fips")]
    let mut last_fips_stale_participant_restart_at: Option<u64> = None;
    #[cfg(feature = "embedded-fips")]
    let mut fips_pending_roster_restart_state = FipsPendingRosterRestartState::default();
    let iface = args.iface.clone();
    let mut tunnel_runtime = CliTunnelRuntime::new(iface.clone());
    let mut network_snapshot = capture_network_snapshot();
    let mut network_changed_at = Some(unix_timestamp());
    let timeout = network_probe_timeout(&app);
    let mut captive_portal = detect_captive_portal(timeout).await;
    #[cfg(target_os = "macos")]
    {
        if macos_underlay_route_repair_allowed(captive_portal) {
            match ensure_macos_underlay_default_route_for_daemon().await {
                Ok(true) => {
                    eprintln!("daemon: restored missing macOS underlay default route");
                    network_snapshot = capture_network_snapshot_for_daemon().await;
                    network_changed_at = Some(unix_timestamp());
                    captive_portal = detect_captive_portal(timeout).await;
                }
                Ok(false) => {}
                Err(error) => {
                    eprintln!("daemon: failed to ensure macOS underlay default route: {error}")
                }
            }
        } else {
            eprintln!(
                "daemon: deferring macOS underlay default route repair while captive portal is detected"
            );
        }
    }
    let mut port_mapping_runtime = PortMappingRuntime::default();
    let mut vpn_enabled = daemon_start_vpn_enabled(&app, args.paused);
    if daemon_vpn_active(vpn_enabled, expected_peers) {
        refresh_port_mapping(
            &app,
            &network_snapshot,
            app.node.listen_port,
            &mut port_mapping_runtime,
        )
        .await;
    }
    #[cfg(feature = "embedded-fips")]
    let (mut fips_tunnel_runtime, mut last_fips_endpoint_peer_signature) =
        if fips_private_runtime_active(&app, vpn_enabled, expected_peers) {
            let config = match fips_tunnel_config_from_app(
                &app,
                &config_path,
                &network_id,
                iface.clone(),
                own_pubkey.as_deref(),
                Some(&recent_peers),
                &[],
            ) {
                Ok(config) => config,
                Err(error) => {
                    let network = network_snapshot.summary(network_changed_at, captive_portal);
                    let port_mapping = port_mapping_runtime.status();
                    persist_daemon_startup_failure_state(
                        &state_file,
                        &app,
                        vpn_enabled,
                        expected_peers,
                        &tunnel_runtime,
                        DaemonStartupFailureContext {
                            network: &network,
                            port_mapping: &port_mapping,
                        },
                        &format!("FIPS private mesh config failed ({error})"),
                    );
                    return Err(error);
                }
            };
            let seeded_endpoint_count = config
                .endpoint_peers
                .iter()
                .flat_map(|peer| peer.addresses.iter())
                .filter(|addr| addr.seen_at_ms.is_some())
                .count();
            let endpoint_peer_signature = endpoint_peer_signature(&config.endpoint_peers);
            let runtime =
                match crate::fips_private_mesh::FipsPrivateTunnelRuntime::start(config).await {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let network = network_snapshot.summary(network_changed_at, captive_portal);
                        let port_mapping = port_mapping_runtime.status();
                        persist_daemon_startup_failure_state(
                            &state_file,
                            &app,
                            vpn_enabled,
                            expected_peers,
                            &tunnel_runtime,
                            DaemonStartupFailureContext {
                                network: &network,
                                port_mapping: &port_mapping,
                            },
                            &format!("FIPS private mesh startup failed ({error})"),
                        );
                        return Err(error);
                    }
                };
            eprintln!(
                "daemon: FIPS private mesh on {} (seeded {} recently-connected peer endpoint(s))",
                runtime.iface(),
                seeded_endpoint_count,
            );
            (Some(runtime), endpoint_peer_signature)
        } else {
            (None, Vec::new())
        };
    let magic_dns_runtime = ConnectMagicDnsRuntime::start(&app);

    let mesh_refresh_interval = Duration::from_secs(args.mesh_refresh_interval_secs.max(5));
    let mut announce_interval = tokio::time::interval(mesh_refresh_interval);
    announce_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recent_peer_refresh_interval = tokio::time::interval(mesh_refresh_interval);
    recent_peer_refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut state_interval = tokio::time::interval(Duration::from_secs(1));
    state_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tunnel_heartbeat_interval = tokio::time::interval(Duration::from_secs(2));
    tunnel_heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut network_interval =
        tokio::time::interval(Duration::from_secs(DAEMON_NETWORK_REFRESH_INTERVAL_SECS));
    network_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut platform_network_change_rx = spawn_platform_network_change_monitor();

    #[cfg(unix)]
    let mut terminate_signal =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;
    #[cfg(unix)]
    let terminate_wait = async move {
        let _ = terminate_signal.recv().await;
    };
    #[cfg(not(unix))]
    let terminate_wait = std::future::pending::<()>();
    tokio::pin!(terminate_wait);

    let mut vpn_status = if !daemon_vpn_active(vpn_enabled, expected_peers) {
        daemon_vpn_idle_status(vpn_enabled, expected_peers, app.join_requests_enabled()).to_string()
    } else {
        "VPN on".to_string()
    };
    let mut last_network_check_at = WallTimeJumpObserver::new(unix_timestamp());
    let mut last_log_compact_check = Instant::now();
    #[cfg(feature = "embedded-fips")]
    let fips_peer_statuses = fips_tunnel_runtime
        .as_ref()
        .map(|runtime| runtime.peer_statuses())
        .unwrap_or_default();
    #[cfg(not(feature = "embedded-fips"))]
    let fips_peer_statuses = Vec::new();
    let fips_relay_statuses = current_fips_relay_statuses(&fips_tunnel_runtime).await;
    let fips_advertised_routes = current_fips_advertised_routes!(fips_tunnel_runtime, &app);
    write_daemon_state(
        &state_file,
        &build_daemon_runtime_state(
            &app,
            vpn_enabled,
            daemon_vpn_active(vpn_enabled, expected_peers),
            expected_peers,
            &tunnel_runtime,
            &fips_peer_statuses,
            &fips_relay_statuses,
            &fips_advertised_routes,
            &vpn_status,
            &network_snapshot.summary(network_changed_at, captive_portal),
            &port_mapping_runtime.status(),
        ),
    )?;
    let mut last_state_persisted_at = Instant::now();
    let daemon_state_persist_interval = Duration::from_secs(DAEMON_STATE_PERSIST_INTERVAL_SECS);
    #[cfg(target_os = "macos")]
    let mut last_macos_underlay_route_check_at =
        Instant::now() - Duration::from_secs(MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS);

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let supervised_service_executable = if args.service {
        Some(current_executable_fingerprint()?)
    } else {
        None
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let supervised_service_executable: Option<(PathBuf, ExecutableFingerprint)> = None;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = &mut terminate_wait => {
                break;
            }
            _ = announce_interval.tick() => {
                #[cfg(feature = "embedded-fips")]
                if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                    if let Err(error) = publish_fips_active_network_roster(
                        runtime,
                        &app,
                        &config_path,
                        &mut pending_fips_roster_recipients,
                    ).await {
                        eprintln!("fips: roster publish failed: {error}");
                    }
                    if let Err(error) = broadcast_local_fips_capabilities(runtime, &app).await {
                        eprintln!("fips: capabilities broadcast failed: {error}");
                    }
                }
            }
            _ = recent_peer_refresh_interval.tick() => {
                #[cfg(feature = "embedded-fips")]
                if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                    update_recent_peers_from_runtime(
                        runtime,
                        &app,
                        &network_id,
                        own_pubkey.as_deref(),
                        RecentPeerRefresh {
                            recent_peers: &mut recent_peers,
                            recent_peers_path: &recent_peers_path,
                            last_endpoint_peer_signature: &mut last_fips_endpoint_peer_signature,
                        },
                        unix_timestamp(),
                    )
                    .await;
                }
            }
            _ = tunnel_heartbeat_interval.tick() => {
                if !daemon_vpn_active(vpn_enabled, expected_peers) {
                    #[cfg(feature = "embedded-fips")]
                    if fips_private_runtime_active(&app, vpn_enabled, expected_peers) {
                        let now = unix_timestamp();
                        if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                            if let Err(error) = runtime.ping_peers(&network_id, now).await {
                                eprintln!("fips: peer ping failed: {error}");
                            }
                            if let Err(error) = runtime.refresh_link_statuses().await {
                                eprintln!("fips: peer link snapshot failed: {error}");
                            }
                        }
                        match restart_fips_tunnel_runtime_after_stale_participants(
                            &mut fips_tunnel_runtime,
                            FipsRestartContext {
                                app: &app,
                                config_path: &config_path,
                                network_id: &network_id,
                                fallback_iface: &iface,
                                own_pubkey: own_pubkey.as_deref(),
                                recent_peers: Some(&recent_peers),
                                last_endpoint_peer_signature:
                                    &mut last_fips_endpoint_peer_signature,
                            },
                            &mut last_fips_stale_participant_restart_at,
                            now,
                        )
                        .await
                        {
                            Ok(true) => fips_roster_sync_state = FipsRosterSyncState::default(),
                            Ok(false) => {}
                            Err(error) => {
                                eprintln!("fips: stale participant recovery failed: {error}")
                            }
                        }
                        match restart_fips_tunnel_runtime_after_pending_roster_links(
                            &mut fips_tunnel_runtime,
                            FipsRestartContext {
                                app: &app,
                                config_path: &config_path,
                                network_id: &network_id,
                                fallback_iface: &iface,
                                own_pubkey: own_pubkey.as_deref(),
                                recent_peers: Some(&recent_peers),
                                last_endpoint_peer_signature:
                                    &mut last_fips_endpoint_peer_signature,
                            },
                            expected_peers,
                            &mut fips_pending_roster_restart_state,
                            now,
                        )
                        .await
                        {
                            Ok(true) => fips_roster_sync_state = FipsRosterSyncState::default(),
                            Ok(false) => {}
                            Err(error) => {
                                eprintln!("fips: pending roster recovery failed: {error}")
                            }
                        }
                        if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                            if let Err(error) = sync_fips_roster_with_connected_peers(
                                runtime,
                                &app,
                                &config_path,
                                &mut fips_roster_sync_state,
                            )
                            .await
                            {
                                eprintln!("fips: roster peer sync failed: {error}");
                            }
                            flush_pending_fips_roster_recipients(
                                runtime,
                                &app,
                                &config_path,
                                &mut pending_fips_roster_recipients,
                            )
                            .await;
                            if let Err(error) = send_pending_fips_join_requests(
                                runtime,
                                &app,
                                &mut fips_join_request_sends,
                                now,
                            )
                            .await
                            {
                                eprintln!("fips: join request send failed: {error}");
                            }
                        }
                    }
                    continue;
                }

                #[cfg(feature = "embedded-fips")]
                if fips_tunnel_runtime.is_some() {
                    let now = unix_timestamp();
                    if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                        if let Err(error) = runtime.ping_peers(&network_id, now).await {
                            eprintln!("fips: peer ping failed: {error}");
                        }
                        if let Err(error) = runtime.refresh_link_statuses().await {
                            eprintln!("fips: peer link snapshot failed: {error}");
                        }
                    }
                    match restart_fips_tunnel_runtime_after_stale_participants(
                        &mut fips_tunnel_runtime,
                        FipsRestartContext {
                            app: &app,
                            config_path: &config_path,
                            network_id: &network_id,
                            fallback_iface: &iface,
                            own_pubkey: own_pubkey.as_deref(),
                            recent_peers: Some(&recent_peers),
                            last_endpoint_peer_signature: &mut last_fips_endpoint_peer_signature,
                        },
                        &mut last_fips_stale_participant_restart_at,
                        now,
                    )
                    .await
                    {
                        Ok(true) => fips_roster_sync_state = FipsRosterSyncState::default(),
                        Ok(false) => {}
                        Err(error) => eprintln!("fips: stale participant recovery failed: {error}"),
                    }
                    match restart_fips_tunnel_runtime_after_pending_roster_links(
                        &mut fips_tunnel_runtime,
                        FipsRestartContext {
                            app: &app,
                            config_path: &config_path,
                            network_id: &network_id,
                            fallback_iface: &iface,
                            own_pubkey: own_pubkey.as_deref(),
                            recent_peers: Some(&recent_peers),
                            last_endpoint_peer_signature: &mut last_fips_endpoint_peer_signature,
                        },
                        expected_peers,
                        &mut fips_pending_roster_restart_state,
                        now,
                    )
                    .await
                    {
                        Ok(true) => fips_roster_sync_state = FipsRosterSyncState::default(),
                        Ok(false) => {}
                        Err(error) => eprintln!("fips: pending roster recovery failed: {error}"),
                    }
                    if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                        if let Err(error) = sync_fips_roster_with_connected_peers(
                            runtime,
                            &app,
                            &config_path,
                            &mut fips_roster_sync_state,
                        )
                        .await
                        {
                            eprintln!("fips: roster peer sync failed: {error}");
                        }
                        flush_pending_fips_roster_recipients(
                            runtime,
                            &app,
                            &config_path,
                            &mut pending_fips_roster_recipients,
                        )
                        .await;
                        if let Err(error) = send_pending_fips_join_requests(
                            runtime,
                            &app,
                            &mut fips_join_request_sends,
                            now,
                        )
                        .await
                        {
                            eprintln!("fips: join request send failed: {error}");
                        }
                    }
                }
            }
            platform_network_change = recv_platform_network_change(&mut platform_network_change_rx) => {
                if platform_network_change.is_none() {
                    platform_network_change_rx = None;
                    continue;
                }
                drain_platform_network_changes(&mut platform_network_change_rx);
                network_interval.reset_after(Duration::from_millis(
                    DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS,
                ));
            }
            _ = network_interval.tick() => {
                let now = unix_timestamp();
                let observed_at = Instant::now();
                let resumed_after_sleep = observe_wall_time_jump(
                    &mut last_network_check_at,
                    now,
                    observed_at,
                    MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
                );
                if resumed_after_sleep {
                    eprintln!("daemon: sleep/wake detected; refreshing FIPS endpoint state");
                }
                let mut latest_snapshot = prefer_nonself_tunnel_snapshot(
                    &tunnel_runtime,
                    &network_snapshot,
                    capture_network_snapshot_for_daemon().await,
                );
                let mut network_changed = latest_snapshot.changed_since(&network_snapshot);
                if network_changed || resumed_after_sleep {
                    captive_portal = detect_captive_portal(timeout).await;
                }
                #[cfg(target_os = "macos")]
                let underlay_repaired = maybe_ensure_macos_underlay_default_route_for_daemon(
                    &mut last_macos_underlay_route_check_at,
                    network_changed,
                    resumed_after_sleep,
                    observed_at,
                    captive_portal,
                )
                .await;
                #[cfg(not(target_os = "macos"))]
                let underlay_repaired = false;
                if underlay_repaired {
                    latest_snapshot = prefer_nonself_tunnel_snapshot(
                        &tunnel_runtime,
                        &network_snapshot,
                        capture_network_snapshot_for_daemon().await,
                    );
                    network_changed = latest_snapshot.changed_since(&network_snapshot);
                    captive_portal = detect_captive_portal(timeout).await;
                }
                let runtime_listen_port =
                    tunnel_runtime.active_listen_port.unwrap_or(app.node.listen_port);
                let vpn_active = daemon_vpn_active(vpn_enabled, expected_peers);
                let endpoint_changed = if network_changed {
                    network_snapshot = latest_snapshot.clone();
                    network_changed_at = Some(unix_timestamp());
                    if vpn_active {
                        refresh_port_mapping(
                            &app,
                            &network_snapshot,
                            runtime_listen_port,
                            &mut port_mapping_runtime,
                        )
                        .await;
                        true
                    } else {
                        port_mapping_runtime.stop().await;
                        false
                    }
                } else if resumed_after_sleep {
                    network_changed_at = Some(now);
                    if vpn_active {
                        refresh_port_mapping(
                            &app,
                            &network_snapshot,
                            runtime_listen_port,
                            &mut port_mapping_runtime,
                        )
                        .await;
                        true
                    } else {
                        port_mapping_runtime.stop().await;
                        false
                    }
                } else if vpn_active {
                    match port_mapping_runtime
                        .renew_if_due(&network_snapshot, runtime_listen_port, timeout)
                        .await
                    {
                        Ok(changed) => changed,
                        Err(error) => {
                            eprintln!("daemon: port mapping renew failed: {error}");
                            false
                        }
                    }
                } else {
                    false
                };

                if !network_changed && !endpoint_changed && !underlay_repaired && !resumed_after_sleep {
                    continue;
                }

                #[cfg(feature = "embedded-fips")]
                let fips_refresh = fips_link_event_refresh(
                    network_changed,
                    endpoint_changed,
                    underlay_repaired,
                    resumed_after_sleep,
                );
                #[cfg(feature = "embedded-fips")]
                let seed_recent_fips_peers =
                    fips_link_event_should_seed_recent_peers(fips_refresh);

                if network_changed || underlay_repaired || resumed_after_sleep || endpoint_changed {
                    let refresh_reason = if network_changed {
                        "network change"
                    } else if resumed_after_sleep {
                        "sleep/wake"
                    } else if underlay_repaired {
                        "macOS underlay repair"
                    } else {
                        "endpoint change"
                    };
                    if network_changed {
                        network_snapshot = latest_snapshot;
                        network_changed_at = Some(unix_timestamp());
                        eprintln!("daemon: network change detected; refreshing FIPS endpoint state");
                    } else if resumed_after_sleep {
                        network_snapshot = latest_snapshot;
                        network_changed_at = Some(now);
                        eprintln!("daemon: sleep/wake detected; refreshing FIPS endpoint state");
                    } else if underlay_repaired {
                        network_snapshot = latest_snapshot;
                        eprintln!("daemon: refreshing tunnel after macOS underlay repair");
                    } else {
                        eprintln!("daemon: endpoint changed; refreshing FIPS endpoint state");
                    }
                    if underlay_repaired {
                        reset_tunnel_runtime_after_macos_underlay_repair(&mut tunnel_runtime);
                    }
                    #[cfg(feature = "embedded-fips")]
                    let fips_result = match fips_refresh {
                        FipsLinkEventRefresh::RefreshPaths => {
                            if fips_tunnel_runtime.is_some()
                                || fips_private_runtime_active(&app, vpn_enabled, expected_peers)
                            {
                                refresh_fips_tunnel_runtime_after_link_event(
                                    &mut fips_tunnel_runtime,
                                    FipsRestartContext {
                                        app: &app,
                                        config_path: &config_path,
                                        network_id: &network_id,
                                        fallback_iface: &iface,
                                        own_pubkey: own_pubkey.as_deref(),
                                        // A link event means the old underlay or NAT mapping just
                                        // changed. Keep the cache on disk, but make this runtime earn
                                        // direct paths from fresh evidence on the current network.
                                        recent_peers: seed_recent_fips_peers
                                            .then_some(&recent_peers),
                                        last_endpoint_peer_signature:
                                            &mut last_fips_endpoint_peer_signature,
                                    },
                                    refresh_reason,
                                )
                                .await
                            } else {
                                Ok(())
                            }
                        }
                        FipsLinkEventRefresh::None => Ok(()),
                    };
                    #[cfg(feature = "embedded-fips")]
                    if let Err(error) = fips_result {
                        vpn_status = format!("Network route refresh failed ({error})");
                    } else {
                        #[cfg(feature = "embedded-fips")]
                        if let Some(runtime) = fips_tunnel_runtime.as_ref() {
                            if let Err(error) = runtime.ping_peers(&network_id, now).await {
                                eprintln!("fips: peer ping failed after network refresh: {error}");
                            }
                            if let Err(error) = runtime.refresh_link_statuses().await {
                                eprintln!(
                                    "fips: peer link snapshot failed after network refresh: {error}"
                                );
                            }
                            update_recent_peers_from_runtime(
                                runtime,
                                &app,
                                &network_id,
                                own_pubkey.as_deref(),
                                RecentPeerRefresh {
                                    recent_peers: &mut recent_peers,
                                    recent_peers_path: &recent_peers_path,
                                    last_endpoint_peer_signature:
                                        &mut last_fips_endpoint_peer_signature,
                                },
                                now,
                            )
                            .await;
                        }
                        vpn_status = if daemon_vpn_active(vpn_enabled, expected_peers) {
                            "Connected (network refresh)".to_string()
                        } else {
                            daemon_vpn_idle_status(
                                vpn_enabled,
                                expected_peers,
                                app.join_requests_enabled(),
                            )
                            .to_string()
                        };
                    }
                    #[cfg(feature = "embedded-fips")]
                    if let Some(runtime) = fips_tunnel_runtime.as_ref()
                        && let Err(error) = broadcast_local_fips_capabilities(runtime, &app).await
                    {
                        eprintln!("fips: capabilities broadcast failed after network refresh: {error}");
                    }
                }
            }
            _ = state_interval.tick() => {
                if daemon_log_compact_check_due(&mut last_log_compact_check)
                    && let Err(error) = compact_daemon_log_if_needed(&config_path)
                {
                    eprintln!("daemon: failed to compact service log: {error}");
                }
                #[cfg(feature = "embedded-fips")]
                if let Some(runtime) = fips_tunnel_runtime.as_mut() {
                    match drain_fips_mesh_events(
                        runtime,
                        &mut app,
                        &config_path,
                        &mut vpn_status,
                    ) {
                        Ok(drained) => {
                            if drained.roster_changed {
                                let reload = build_daemon_reload_config(
                                    app.clone(),
                                    app.effective_network_id(),
                                );
                                app = reload.app;
                                network_id = reload.network_id;
                                expected_peers = reload.expected_peers;
                                own_pubkey = reload.own_pubkey;

                                fips_join_request_sends.clear();
                                if let Err(error) = refresh_fips_tunnel_config(
                                    runtime,
                                    &app,
                                    &config_path,
                                    &network_id,
                                    own_pubkey.as_deref(),
                                )
                                .await
                                {
                                    vpn_status =
                                        format!("Roster applied, but FIPS reload failed ({error})");
                                }
                                if let Some(rt) = magic_dns_runtime.as_ref() {
                                    rt.refresh_records(&app);
                                }
                            }
                            if !drained.endpoint_hint_participants.is_empty()
                                && let Err(error) =
                                    refresh_fips_tunnel_runtime_peer_paths_in_place(
                                        runtime,
                                        FipsRestartContext {
                                            app: &app,
                                            config_path: &config_path,
                                            network_id: &network_id,
                                            fallback_iface: &iface,
                                            own_pubkey: own_pubkey.as_deref(),
                                            recent_peers: Some(&recent_peers),
                                            last_endpoint_peer_signature:
                                                &mut last_fips_endpoint_peer_signature,
                                        },
                                        &drained.endpoint_hint_participants,
                                        "fresh endpoint capability",
                                    )
                                    .await
                            {
                                vpn_status =
                                    format!("FIPS endpoint hint refresh failed ({error})");
                            }
                        }
                        Err(error) => {
                            vpn_status = format!("FIPS event handling failed ({error})");
                        }
                    }
                    if let Err(error) = runtime.refresh_peer_dependent_routes().await {
                        vpn_status = format!("FIPS route refresh failed ({error})");
                    }
                }

                if let Some(request) = take_daemon_control_request(&config_path) {
                    let publish_fips_roster_after_control =
                        matches!(request, DaemonControlRequest::Reload | DaemonControlRequest::Resume);
                    let control_result = match request {
                        DaemonControlRequest::Stop => break,
                        DaemonControlRequest::Pause => {
                            vpn_enabled = false;
                            let persist_result =
                                persist_desired_daemon_vpn_enabled_in_config(
                                    &mut app,
                                    &config_path,
                                    vpn_enabled,
                                );
                            let join_requests_active = app.join_requests_enabled();
                            port_mapping_runtime.stop().await;
                            vpn_status = daemon_vpn_idle_status(
                                vpn_enabled,
                                expected_peers,
                                join_requests_active,
                            )
                            .to_string();
                            persist_result.map(|_| ())
                        }
                        DaemonControlRequest::Resume => {
                            vpn_enabled = true;
                            let persist_result =
                                persist_desired_daemon_vpn_enabled_in_config(
                                    &mut app,
                                    &config_path,
                                    vpn_enabled,
                                );
                            if daemon_vpn_active(vpn_enabled, expected_peers) {
                                let runtime_listen_port = tunnel_runtime
                                    .active_listen_port
                                    .unwrap_or(app.node.listen_port);
                                refresh_port_mapping(
                                    &app,
                                    &network_snapshot,
                                    runtime_listen_port,
                                    &mut port_mapping_runtime,
                                )
                                .await;
                                vpn_status = "VPN on".to_string();
                            } else {
                                port_mapping_runtime.stop().await;
                                vpn_status = daemon_vpn_idle_status(
                                    vpn_enabled,
                                    expected_peers,
                                    app.join_requests_enabled(),
                                )
                                .to_string();
                            }
                            persist_result.map(|_| ())
                        }
                        DaemonControlRequest::Reload => {
                            match update_daemon_config_from_staged_request(&config_path) {
                                Ok(staged_config_applied) => {
                                    match load_config_with_overrides(
                                        &config_path,
                                        network_override.clone(),
                                        participants_override.clone(),
                                    ) {
                                        Ok((reloaded_app, reloaded_network_id)) => {
                                            let reload = build_daemon_reload_config(
                                                reloaded_app,
                                                reloaded_network_id,
                                            );
                                            app = reload.app;
                                            network_id = reload.network_id;
                                            expected_peers = reload.expected_peers;
                                            own_pubkey = reload.own_pubkey;
                                            if let Some(rt) = magic_dns_runtime.as_ref() {
                                                rt.refresh_records(&app);
                                            }

                                            let join_requests_active = app.join_requests_enabled();
                                            let vpn_active =
                                                daemon_vpn_active(vpn_enabled, expected_peers);
                                            vpn_status = if vpn_active {
                                                "Config reloaded".to_string()
                                            } else if vpn_enabled {
                                                daemon_vpn_idle_status(
                                                    vpn_enabled,
                                                    expected_peers,
                                                    join_requests_active,
                                                )
                                                .to_string()
                                            } else {
                                                "Config reloaded (paused)".to_string()
                                            };

                                            if vpn_active {
                                                let runtime_listen_port = tunnel_runtime
                                                    .active_listen_port
                                                    .unwrap_or(app.node.listen_port);
                                                refresh_port_mapping(
                                                    &app,
                                                    &network_snapshot,
                                                    runtime_listen_port,
                                                    &mut port_mapping_runtime,
                                                )
                                                .await;
                                            }
                                            Ok(())
                                        }
                                        Err(error) => {
                                            vpn_status = if staged_config_applied {
                                                format!("Config apply failed (reload: {error})")
                                            } else {
                                                format!("Config reload failed ({error})")
                                            };
                                            Err(error)
                                        }
                                    }
                                }
                                Err(error) => {
                                    vpn_status = format!("Config apply failed ({error})");
                                    Err(error)
                                }
                            }
                        }
                    };
                    let _ = write_daemon_control_result(&config_path, request, control_result);
                    #[cfg(feature = "embedded-fips")]
                    let pre_sync_fips_roster_recipients = if publish_fips_roster_after_control {
                        fips_tunnel_runtime
                            .as_ref()
                            .map(|runtime| runtime.peer_pubkeys())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    #[cfg(feature = "embedded-fips")]
                    if publish_fips_roster_after_control
                        && let Some(runtime) = fips_tunnel_runtime.as_ref()
                        && let Err(error) = publish_fips_active_network_roster_to(
                            runtime,
                            &app,
                            &config_path,
                            &pre_sync_fips_roster_recipients,
                            &mut pending_fips_roster_recipients,
                        )
                        .await
                    {
                        eprintln!(
                            "fips: roster publish failed before peer-set refresh: {error}"
                        );
                    }
                    #[cfg(feature = "embedded-fips")]
                    if let Err(error) = sync_fips_private_runtime(
                        &mut fips_tunnel_runtime,
                        SyncFipsPrivateRuntimeContext {
                            app: &app,
                            config_path: &config_path,
                            network_id: &network_id,
                            iface: &iface,
                            own_pubkey: own_pubkey.as_deref(),
                            vpn_enabled,
                            expected_peers,
                        },
                    )
                    .await
                    {
                        vpn_status = format!("FIPS private mesh update failed ({error})");
                    }
                    #[cfg(feature = "embedded-fips")]
                    if publish_fips_roster_after_control
                        && let Some(runtime) = fips_tunnel_runtime.as_ref()
                    {
                        if let Err(error) = publish_fips_active_network_roster(
                            runtime,
                            &app,
                            &config_path,
                            &mut pending_fips_roster_recipients,
                        )
                        .await
                        {
                            eprintln!(
                                "fips: roster publish failed after control request: {error}"
                            );
                        }
                        if let Err(error) = broadcast_local_fips_capabilities(runtime, &app).await {
                            eprintln!(
                                "fips: capabilities broadcast failed after control request: {error}"
                            );
                        }
                    }
                    if persist_daemon_runtime_and_cleanup_state(
                        &state_file,
                        &config_path,
                        &app,
                        vpn_enabled,
                        expected_peers,
                        &tunnel_runtime,
                        &current_fips_peer_statuses!(fips_tunnel_runtime),
                        &current_fips_relay_statuses(&fips_tunnel_runtime).await,
                        &current_fips_advertised_routes!(fips_tunnel_runtime, &app),
                        &vpn_status,
                        &network_snapshot.summary(network_changed_at, captive_portal),
                        &port_mapping_runtime.status(),
                    ) {
                        last_state_persisted_at = Instant::now();
                    }
                }
                if let Some((executable, launched_fingerprint)) =
                    supervised_service_executable.as_ref()
                {
                    match service_supervisor_restart_due(executable, launched_fingerprint) {
                        Ok(true) => {
                            eprintln!(
                                "daemon: service executable changed on disk; exiting so the supervisor restarts the updated binary"
                            );
                            break;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            eprintln!(
                                "daemon: failed to check service executable fingerprint: {error}"
                            );
                        }
                    }
                }
                if vpn_status == "Connected (network refresh)"
                    && daemon_vpn_active(vpn_enabled, expected_peers)
                {
                    vpn_status = "VPN on".to_string();
                }
                if last_state_persisted_at.elapsed() >= daemon_state_persist_interval
                    && persist_daemon_runtime_and_cleanup_state(
                        &state_file,
                        &config_path,
                        &app,
                        vpn_enabled,
                        expected_peers,
                        &tunnel_runtime,
                        &current_fips_peer_statuses!(fips_tunnel_runtime),
                        &current_fips_relay_statuses(&fips_tunnel_runtime).await,
                        &current_fips_advertised_routes!(fips_tunnel_runtime, &app),
                        &vpn_status,
                        &network_snapshot.summary(network_changed_at, captive_portal),
                        &port_mapping_runtime.status(),
                    )
                {
                    last_state_persisted_at = Instant::now();
                }
            }
        }
    }

    port_mapping_runtime.stop().await;
    #[cfg(feature = "embedded-fips")]
    if let Some(runtime) = fips_tunnel_runtime
        && let Err(error) = runtime.stop().await
    {
        eprintln!("daemon: failed to stop FIPS private mesh: {error}");
    }
    tunnel_runtime.stop();
    if let Err(error) = persist_daemon_network_cleanup_state(&config_path, &tunnel_runtime) {
        eprintln!("daemon: failed to clear network cleanup state: {error}");
    }
    let final_state = disconnected_daemon_runtime_state(
        expected_peers,
        &network_snapshot.summary(network_changed_at, captive_portal),
    );
    let _ = write_daemon_state(&state_file, &final_state);
    remove_current_daemon_pid_record(&pid_file);
    Ok(())
}
