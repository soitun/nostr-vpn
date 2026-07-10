#[cfg(any(target_os = "linux", target_os = "macos"))]
impl FipsPrivateTunnelRuntime {
    pub(crate) async fn start(config: FipsPrivateTunnelConfig) -> Result<Self> {
        let scope = config
            .local_ethernet_underlay
            .is_none()
            .then(|| {
                config
                    .nostr_discovery_enabled
                    .then(|| fips_lan_discovery_scope(&config.network_id))
            })
            .flatten();
        let transport = FipsEndpointTransportConfig {
            listen_port: config.listen_port,
            advertised_endpoint: config.advertised_endpoint.clone(),
            advertise_public_endpoint: config.advertise_public_endpoint,
            nostr_discovery_enabled: config.nostr_discovery_enabled,
            stun_servers: config.stun_servers.clone(),
            nostr_relays: config.nostr_relays.clone(),
            share_local_candidates: config.share_local_candidates,
        };
        let endpoint_config = if let Some(ethernet) = config.local_ethernet_underlay.as_ref() {
            fips_endpoint_config_for_local_ethernet(
                &config.endpoint_peers,
                ethernet,
                config.mesh_mtu,
            )
        } else {
            fips_endpoint_config_with_open_discovery_limit(
                &config.endpoint_peers,
                Some(&transport),
                config.mesh_mtu,
                config.nostr_discovery_policy,
                config.open_discovery_max_pending,
            )
        };
        let local_allowed_ips = config.local_allowed_ips();
        let local_tunnel_ips = config.local_tunnel_ips();
        let mesh = Arc::new(
            FipsPrivateMeshRuntime::bind_with_config_scoped(
                config.identity_nsec.clone(),
                scope,
                config.peers.clone(),
                endpoint_config,
                local_allowed_ips,
                local_tunnel_ips,
                config.paid_route_admissions.clone(),
            )
            .await?,
        );
        Self::start_with_mesh(config, mesh, true).await
    }

    #[cfg(target_os = "linux")]
    pub(crate) async fn start_with_shared_endpoint(
        config: FipsPrivateTunnelConfig,
        shared: FipsSharedEndpoint,
    ) -> Result<Self> {
        if config.local_ethernet_underlay.is_none() {
            return Err(anyhow!(
                "shared FIPS endpoint requires a local-Ethernet-only tunnel config"
            ));
        }
        let local_allowed_ips = config.local_allowed_ips();
        let local_tunnel_ips = config.local_tunnel_ips();
        let mesh = Arc::new(FipsPrivateMeshRuntime::from_shared_endpoint(
            shared,
            config.peers.clone(),
            local_allowed_ips,
            local_tunnel_ips,
            config.paid_route_admissions.clone(),
        ));
        // The WebVM host network owns the exit-DNS service receiver before
        // pairing on this shared endpoint.
        Self::start_with_mesh(config, mesh, false).await
    }

    async fn start_with_mesh(
        config: FipsPrivateTunnelConfig,
        mesh: Arc<FipsPrivateMeshRuntime>,
        register_exit_dns: bool,
    ) -> Result<Self> {
        crate::pipeline_profile::maybe_spawn_reporter();
        #[cfg(target_os = "linux")]
        ensure_linux_tun_permissions(&config.iface)?;
        #[cfg(feature = "paid-exit")]
        mesh.set_paid_route_accounting_peers(config.paid_route_accounting_peers.clone())?;
        let exit_dns = if register_exit_dns {
            Some(
                crate::exit_dns_runtime::ExitDnsFipsRuntime::start_tunnel(
                    Arc::clone(mesh.endpoint()),
                    config.exit_dns_service_enabled(),
                    &config.active_roster_endpoint_npubs,
                )
                .await?,
            )
        } else {
            None
        };
        let control_pubsub = crate::control_pubsub_runtime::ControlPubsubFipsRuntime::start(
            Arc::clone(mesh.endpoint()),
            config.nostr_pubsub.clone(),
            config.nostr_relays.clone(),
            Some(config.control_pubsub_store_path.clone()),
        )
        .await?;
        let tun = Arc::new(
            SystemTun::new(&config.iface)
                .with_context(|| fips_tun_create_context(&config.iface))?
                .set_non_blocking()
                .context("failed to set FIPS tunnel nonblocking")?,
        );
        let iface = tun.name().context("failed to read FIPS tunnel name")?;
        let tun_fd = BorrowedTunFd::new(tun.as_raw_fd());

        let (event_tx, event_rx) = mpsc::channel::<FipsPrivateMeshEvent>(1024);
        let tun_send_worker = spawn_tun_send_worker(Arc::clone(&tun), Arc::clone(&mesh));
        let mesh_recv_worker = spawn_mesh_recv_worker(Arc::clone(&mesh), tun_fd, event_tx);

        let mut runtime = Self {
            iface,
            mesh,
            control_pubsub,
            exit_dns,
            config: config.clone(),
            _tun: tun,
            fips_host: None,
            tun_send_worker,
            mesh_recv_worker,
            event_rx,
            #[cfg(target_os = "linux")]
            endpoint_bypass_routes: Vec::new(),
            #[cfg(target_os = "macos")]
            endpoint_bypass_routes: Vec::new(),
            #[cfg(target_os = "linux")]
            original_default_route: None,
            #[cfg(target_os = "linux")]
            original_default_ipv6_route: None,
            #[cfg(target_os = "linux")]
            exit_node_runtime: crate::LinuxExitNodeRuntime::default(),
            #[cfg(target_os = "macos")]
            exit_node_runtime: crate::MacosExitNodeRuntime::default(),
            #[cfg(target_os = "macos")]
            wg_upstream: None,
        };
        runtime.apply_interface_config(&config).await?;
        runtime
            .reconcile_fips_host_runtime(config.fips_host.clone())
            .await?;
        Ok(runtime)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn endpoint_bypass_ipv4_hosts(
        &self,
        config: &FipsPrivateTunnelConfig,
    ) -> Result<Vec<Ipv4Addr>> {
        let mut hosts = config.endpoint_hint_ipv4_hosts();
        hosts.extend(
            self.mesh
                .peer_transport_ipv4_hosts()
                .await?
                .into_iter()
                .filter(|host| !route_targets_include_ipv4_host(&config.route_targets, *host)),
        );
        hosts.sort_unstable();
        hosts.dedup();
        Ok(hosts)
    }

    pub(crate) fn requires_endpoint_restart(&self, config: &FipsPrivateTunnelConfig) -> bool {
        fips_tunnel_requires_endpoint_restart(&self.config, config)
    }

    pub(crate) async fn apply_config(&mut self, config: FipsPrivateTunnelConfig) -> Result<()> {
        self.mesh
            .replace_peers(
                config.peers.clone(),
                config.local_allowed_ips(),
                config.paid_route_admissions.clone(),
            )?;
        #[cfg(feature = "paid-exit")]
        self.mesh
            .set_paid_route_accounting_peers(config.paid_route_accounting_peers.clone())?;
        if let Err(error) = self.mesh.update_peers(&config.endpoint_peers).await {
            eprintln!("fips: update_peers during apply_config failed: {error}");
        }
        if self.config.nostr_relays != config.nostr_relays {
            self.mesh.update_relays(&config.nostr_relays).await?;
        }
        if let Some(exit_dns) = self.exit_dns.as_ref() {
            exit_dns.reconfigure(
                config.exit_dns_service_enabled(),
                &config.active_roster_endpoint_npubs,
            );
        }
        self.apply_interface_config(&config).await?;
        self.reconcile_fips_host_runtime(config.fips_host.clone())
            .await?;
        self.config = config;
        Ok(())
    }

    pub(crate) async fn refresh_peer_dependent_routes(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if !linux_route_targets_require_ip_endpoint_bypass(
                &self.config,
                &self.config.route_targets,
            ) {
                return Ok(());
            }

            let config = self.config.clone();
            return self.apply_interface_config(&config).await;
        }

        #[cfg(target_os = "macos")]
        {
            if !crate::route_targets_require_endpoint_bypass(&self.config.route_targets) {
                return Ok(());
            }

            let config = self.config.clone();
            return self.apply_interface_config(&config).await;
        }
    }

    pub(crate) async fn stop(self) -> Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut runtime = self;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let runtime = self;
        #[cfg(target_os = "linux")]
        runtime.cleanup_linux_network_state();
        #[cfg(target_os = "macos")]
        runtime.cleanup_macos_network_state();
        #[cfg(target_os = "macos")]
        runtime.cleanup_macos_exit_node_forwarding();
        #[cfg(target_os = "macos")]
        if let Some(handle) = runtime.wg_upstream.take() {
            handle.cleanup().await;
        }
        runtime.stop_fips_host_runtime().await;
        if let Some(exit_dns) = runtime.exit_dns.take() {
            exit_dns.stop().await;
        }
        if let Some(control_pubsub) = runtime.control_pubsub.take() {
            control_pubsub.stop().await;
        }
        stop_tun_send_worker(runtime.tun_send_worker).await;
        stop_mesh_recv_worker(runtime.mesh_recv_worker, &runtime.mesh).await;
        if let Ok(mesh) = Arc::try_unwrap(runtime.mesh) {
            mesh.shutdown()
                .await
                .context("failed to stop FIPS endpoint")?;
        }
        Ok(())
    }

    async fn apply_interface_config(&mut self, config: &FipsPrivateTunnelConfig) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.apply_linux_network_state(config).await?;
        }
        #[cfg(target_os = "macos")]
        {
            self.apply_macos_network_state(config).await?;
            self.reconcile_macos_wg_upstream(&config.wireguard_exit)
                .await;
            self.reconcile_macos_exit_node_forwarding(
                &config.local_address,
                &config.local_exit_forwarding_routes,
            );
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    async fn apply_macos_network_state(&mut self, config: &FipsPrivateTunnelConfig) -> Result<()> {
        let mut route_targets = config.route_targets.clone();
        let requested_ipv4_exit = route_targets.iter().any(|route| route == "0.0.0.0/0");
        let original_route_targets_require_bypass =
            crate::route_targets_require_endpoint_bypass(&route_targets);

        if original_route_targets_require_bypass {
            let peer_endpoint_hosts = self.endpoint_bypass_ipv4_hosts(config).await?;
            if requested_ipv4_exit && peer_endpoint_hosts.is_empty() {
                eprintln!(
                    "fips: withholding macOS default route until the selected exit peer underlay endpoint is known"
                );
                route_targets.retain(|route| !crate::is_exit_node_route(route));
                self.reconcile_macos_endpoint_bypass_routes(&[], None);
            } else {
                match crate::macos_underlay_default_route_from_system() {
                    Ok(Some(underlay)) => {
                        let endpoint_bypass_routes =
                            crate::macos_network::macos_endpoint_bypass_targets_for_hosts(
                                &peer_endpoint_hosts,
                            );
                        self.reconcile_macos_endpoint_bypass_routes(
                            &endpoint_bypass_routes,
                            Some(&underlay),
                        );
                    }
                    Ok(None) => {
                        eprintln!(
                            "fips: withholding macOS default route because no underlay default route is available"
                        );
                        route_targets.retain(|route| !crate::is_exit_node_route(route));
                        self.reconcile_macos_endpoint_bypass_routes(&[], None);
                    }
                    Err(error) => {
                        eprintln!(
                            "fips: withholding macOS default route because underlay route lookup failed: {error}"
                        );
                        route_targets.retain(|route| !crate::is_exit_node_route(route));
                        self.reconcile_macos_endpoint_bypass_routes(&[], None);
                    }
                }
            }
        } else {
            self.reconcile_macos_endpoint_bypass_routes(&[], None);
        }

        let active_ipv4_exit = route_targets.iter().any(|route| route == "0.0.0.0/0");
        if !active_ipv4_exit
            && let Err(error) = crate::delete_macos_default_route_for_interface(&self.iface)
            && !crate::daemon_runtime::macos_route_delete_error_is_absent(&error.to_string())
        {
            eprintln!(
                "fips: failed to remove stale macOS default routes on {}: {}",
                self.iface, error
            );
        }

        route_targets.sort();
        route_targets.dedup();
        // FIPS mesh peer routes go in first. They're /32s for each peer's
        // tunnel IP, so even when we install split defaults below, mesh traffic
        // still wins on longest-prefix-match and stays inside the FIPS tunnel.
        crate::apply_local_interface_network_with_mtu_and_addresses(
            &self.iface,
            &config.interface_addresses(),
            &route_targets,
            config.mesh_mtu.tunnel,
        )
        .with_context(|| format!("failed to configure FIPS tunnel interface {}", self.iface))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn reconcile_macos_endpoint_bypass_routes(
        &mut self,
        routes: &[String],
        underlay: Option<&crate::MacosRouteSpec>,
    ) {
        let desired = routes
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        let stale = self
            .endpoint_bypass_routes
            .iter()
            .filter(|route| !desired.contains(*route))
            .cloned()
            .collect::<Vec<_>>();
        for route in stale {
            if let Err(error) = crate::delete_macos_managed_route(&route, None, None)
                && !crate::daemon_runtime::macos_route_delete_error_is_absent(&error.to_string())
            {
                eprintln!("fips: failed to remove macOS endpoint bypass route {route}: {error}");
            }
        }

        if let Some(underlay) = underlay {
            for route in routes {
                if let Err(error) =
                    crate::apply_macos_route_spec(route, underlay.gateway.as_deref(), None)
                {
                    eprintln!(
                        "fips: failed to install macOS endpoint bypass route {}: {}",
                        route, error
                    );
                }
            }
        }

        self.endpoint_bypass_routes = desired.into_iter().collect();
        self.endpoint_bypass_routes.sort();
    }

    #[cfg(target_os = "macos")]
    fn cleanup_macos_network_state(&mut self) {
        self.reconcile_macos_endpoint_bypass_routes(&[], None);
        if let Err(error) = crate::delete_macos_default_route_for_interface(&self.iface)
            && !crate::daemon_runtime::macos_route_delete_error_is_absent(&error.to_string())
        {
            eprintln!(
                "fips: failed to remove macOS default routes on {}: {}",
                self.iface, error
            );
        }
    }

    async fn reconcile_fips_host_runtime(
        &mut self,
        config: Option<FipsHostTunnelConfig>,
    ) -> Result<()> {
        let needs_restart = match (&self.fips_host, &config) {
            (Some(runtime), Some(config)) => runtime.requires_restart(config),
            (Some(_), None) => true,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if needs_restart {
            self.stop_fips_host_runtime().await;
        }

        match config {
            Some(config) if self.fips_host.is_none() => {
                let runtime = crate::fips_host_tunnel::FipsHostTunnelRuntime::start(config).await?;
                eprintln!("fips-host: .fips IPv6 resolver active");
                self.fips_host = Some(runtime);
            }
            None => {
                crate::fips_host_tunnel::FipsHostTunnelRuntime::cleanup_disabled_artifacts();
            }
            Some(_) => {}
        }
        Ok(())
    }

    async fn stop_fips_host_runtime(&mut self) {
        if let Some(runtime) = self.fips_host.take()
            && let Err(error) = runtime.stop().await
        {
            eprintln!("fips-host: failed to stop .fips runtime: {error}");
        }
    }

    /// Bring the WG upstream tunnel up / down to match `wireguard_exit`.
    ///
    /// Called on every `apply_interface_config` (which fires on
    /// startup, on every config change, and on the periodic
    /// peer-dependent route refresh). The function is idempotent: a
    /// no-op if the existing tunnel already matches the config, a
    /// teardown-then-bring-up if the config changed, just a teardown
    /// if WG is now disabled.
    ///
    /// **Safe-by-construction**: if the WG handshake doesn't complete
    /// within the watchdog window (10s), nothing modifies the routing
    /// table. The host's default route only ever swaps to the WG tun
    /// after we've seen a real handshake from the upstream.
    #[cfg(target_os = "macos")]
    async fn reconcile_macos_wg_upstream(&mut self, wg_config: &WireGuardExitConfig) {
        let want_up = wg_config.enabled && wg_config.configured();

        // Already up with matching config → nothing to do.
        if want_up
            && self
                .wg_upstream
                .as_ref()
                .is_some_and(|existing| existing.matches(wg_config))
        {
            return;
        }

        // If we have a stale tunnel (config changed, or now disabled),
        // tear it down before doing anything else. This restores the
        // original default route + deletes the bypass.
        if let Some(existing) = self.wg_upstream.take() {
            existing.cleanup().await;
        }

        if !want_up {
            return;
        }

        match crate::wg_upstream_runtime::apply_daemon_wg_upstream(
            wg_config,
            crate::wg_upstream_runtime::DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT,
        )
        .await
        {
            Ok(handle) => {
                eprintln!(
                    "fips: WG upstream up on {} via {} (default route swapped)",
                    handle.iface, handle.upstream
                );
                self.wg_upstream = Some(handle);
            }
            Err(error) => {
                // The watchdog fired or another error occurred. The
                // routing table was deliberately left untouched, so
                // the host's internet is still fine — surface the
                // error for the GUI / status page and try again on
                // the next reconcile tick.
                eprintln!("fips: WG upstream not started: {error}");
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn reconcile_macos_exit_node_forwarding(&mut self, local_address: &str, routes: &[String]) {
        let route_families = crate::linux_exit_node_default_route_families(routes);
        if !route_families.ipv4 {
            self.cleanup_macos_exit_node_forwarding();
            return;
        }
        if route_families.ipv6 {
            eprintln!(
                "fips: IPv6 exit-node forwarding is disabled on macOS until nvpn has IPv6 PF source filtering"
            );
        }

        let Some(tunnel_source_cidr) = crate::linux_exit_node_source_cidr(local_address) else {
            eprintln!("fips: invalid IPv4 tunnel address '{local_address}'");
            self.cleanup_macos_exit_node_forwarding();
            return;
        };

        let outbound_iface = match crate::macos_underlay_default_route_from_system() {
            Ok(Some(route)) => route.interface,
            Ok(None) => {
                eprintln!("fips: failed to resolve macOS underlay default route for exit NAT");
                self.cleanup_macos_exit_node_forwarding();
                return;
            }
            Err(error) => {
                eprintln!("fips: failed to resolve macOS underlay default route: {error}");
                self.cleanup_macos_exit_node_forwarding();
                return;
            }
        };

        let already_configured = self.exit_node_runtime.outbound_iface.as_deref()
            == Some(outbound_iface.as_str())
            && self.exit_node_runtime.tunnel_source_cidr.as_deref()
                == Some(tunnel_source_cidr.as_str());
        if already_configured {
            return;
        }

        self.cleanup_macos_exit_node_forwarding();
        match crate::macos_pf_enabled() {
            Ok(enabled) => {
                self.exit_node_runtime.pf_was_enabled = Some(enabled);
                if !enabled && let Err(error) = crate::enable_macos_pf() {
                    eprintln!("fips: failed to enable macOS PF for exit NAT: {error}");
                    self.cleanup_macos_exit_node_forwarding();
                    return;
                }
            }
            Err(error) => {
                eprintln!("fips: failed to read macOS PF state: {error}");
                self.cleanup_macos_exit_node_forwarding();
                return;
            }
        }

        if let Err(error) =
            crate::apply_macos_exit_node_pf_rules(&self.iface, &outbound_iface, &tunnel_source_cidr)
        {
            eprintln!("fips: failed to install macOS exit PF rules: {error}");
            self.cleanup_macos_exit_node_forwarding();
            return;
        }

        self.exit_node_runtime.outbound_iface = Some(outbound_iface);
        self.exit_node_runtime.tunnel_source_cidr = Some(tunnel_source_cidr);
    }

    #[cfg(target_os = "macos")]
    fn cleanup_macos_exit_node_forwarding(&mut self) {
        if self.exit_node_runtime.pf_was_enabled.is_some()
            && let Err(error) = crate::cleanup_macos_pf_nat()
        {
            eprintln!("fips: failed to remove macOS exit PF rules: {error}");
        }

        if self.exit_node_runtime.pf_was_enabled == Some(false)
            && let Err(error) = crate::run_checked(ProcessCommand::new("pfctl").arg("-d"))
        {
            eprintln!("fips: failed to restore macOS PF enabled state: {error}");
        }

        self.exit_node_runtime = crate::MacosExitNodeRuntime::default();
    }

}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl FipsPrivateTunnelRuntime {
    pub(crate) fn iface(&self) -> &str {
        &self.iface
    }

    pub(crate) fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        self.mesh.peer_statuses()
    }

    #[cfg(feature = "paid-exit")]
    pub(crate) fn drain_paid_route_usage(&self, participant: &str) -> Result<PaidRouteUsage> {
        self.mesh.drain_paid_route_usage(participant)
    }

    pub(crate) fn stale_participants_needing_path_refresh(&self, now: u64) -> Vec<String> {
        self.mesh.stale_participants_needing_path_refresh(now)
    }

    pub(crate) async fn relay_statuses(&self) -> Result<Vec<FipsRelayStatus>> {
        self.mesh.relay_statuses().await
    }

    pub(crate) async fn local_advertised_endpoints(&self) -> Result<Vec<OverlayEndpointAdvert>> {
        self.mesh.local_advertised_endpoints().await
    }

    pub(crate) fn peer_pubkeys(&self) -> Vec<String> {
        self.mesh.peer_pubkeys()
    }

    pub(crate) async fn authenticated_peer_transport_addrs(&self) -> Result<Vec<(String, String)>> {
        self.mesh.authenticated_peer_transport_addrs().await
    }

    pub(crate) fn peer_endpoint_hints(&self) -> Vec<(String, Vec<(String, u64)>)> {
        self.mesh.peer_endpoint_hints()
    }

    /// Forward a refreshed peer roster + address hints to fips without
    /// restarting the endpoint. Daemon heartbeat path: when the
    /// recent-peers cache or active-network roster changes, build the
    /// merged hint list and call this so fips can diff + apply.
    pub(crate) async fn update_peers(
        &self,
        endpoint_peers: &[FipsEndpointPeerTransportConfig],
    ) -> Result<fips_endpoint::UpdatePeersOutcome> {
        self.mesh.update_peers(endpoint_peers).await
    }

    pub(crate) async fn refresh_peer_paths(
        &self,
        endpoint_peers: &[FipsEndpointPeerTransportConfig],
    ) -> Result<usize> {
        self.mesh.refresh_peer_paths(endpoint_peers).await
    }

    pub(crate) async fn ping_peers(&self, network_id: &str, now: u64) -> Result<usize> {
        self.mesh.ping_peers(network_id, now).await
    }

    pub(crate) async fn refresh_link_statuses(&self) -> Result<()> {
        self.mesh.refresh_link_statuses().await
    }

    pub(crate) async fn send_join_request(
        &self,
        participant: &str,
        requested_at: u64,
        request: MeshJoinRequest,
    ) -> Result<()> {
        self.mesh
            .send_join_request(participant, requested_at, request)
            .await
    }

    pub(crate) async fn send_roster(
        &self,
        participant: &str,
        signed_roster: SignedRoster,
    ) -> Result<()> {
        self.mesh.send_roster(participant, signed_roster).await
    }

    pub(crate) async fn send_capabilities(
        &self,
        participant: &str,
        network_id: &str,
        capabilities: PeerCapabilities,
    ) -> Result<()> {
        self.mesh
            .send_capabilities(participant, network_id, capabilities)
            .await
    }

    pub(crate) fn peer_advertised_routes(&self, participant: &str) -> Vec<String> {
        self.mesh.peer_advertised_routes(participant)
    }

    pub(crate) fn drain_events(&mut self) -> Vec<FipsPrivateMeshEvent> {
        drain_event_batch(&mut self.event_rx, FIPS_MESH_EVENT_DRAIN_LIMIT)
    }
}
