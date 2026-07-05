#[cfg(target_os = "windows")]
impl FipsPrivateTunnelRuntime {
    pub(crate) async fn start(config: FipsPrivateTunnelConfig) -> Result<Self> {
        crate::pipeline_profile::maybe_spawn_reporter();
        let scope = config
            .nostr_discovery_enabled
            .then(|| fips_lan_discovery_scope(&config.network_id));
        let transport = FipsEndpointTransportConfig {
            listen_port: config.listen_port,
            advertised_endpoint: config.advertised_endpoint.clone(),
            advertise_public_endpoint: config.advertise_public_endpoint,
            nostr_discovery_enabled: config.nostr_discovery_enabled,
            stun_servers: config.stun_servers.clone(),
            nostr_relays: config.nostr_relays.clone(),
            share_local_candidates: config.share_local_candidates,
        };
        let endpoint_config = fips_endpoint_config_with_open_discovery_limit(
            &config.endpoint_peers,
            Some(&transport),
            config.mesh_mtu,
            config.nostr_discovery_policy,
            config.open_discovery_max_pending,
        );
        let mesh = Arc::new(
            FipsPrivateMeshRuntime::bind_with_config_scoped(
                config.identity_nsec.clone(),
                scope,
                config.peers.clone(),
                endpoint_config,
                config.local_allowed_ips(),
                config.local_tunnel_ips(),
                config.paid_route_admissions.clone(),
            )
            .await?,
        );
        #[cfg(feature = "paid-exit")]
        mesh.set_paid_route_accounting_peers(config.paid_route_accounting_peers.clone())?;
        let (session, iface, interface_index) = start_windows_fips_wintun(&config)?;
        let route_targets =
            crate::windows_tunnel::apply_windows_routes(interface_index, &config.route_targets)?;

        let stop = Arc::new(AtomicBool::new(false));
        let (event_tx, event_rx) = mpsc::channel::<FipsPrivateMeshEvent>(1024);
        let tun_read_thread = spawn_windows_fips_tun_read_thread(
            stop.clone(),
            session.clone(),
            Arc::clone(&mesh),
        );
        let mesh_recv_task =
            spawn_windows_fips_mesh_recv_task(Arc::clone(&mesh), session.clone(), event_tx);

        let mut runtime = Self {
            iface,
            mesh,
            config: config.clone(),
            session,
            stop,
            tun_read_thread,
            mesh_recv_task,
            event_rx,
            interface_index,
            route_targets,
            wg_upstream: None,
        };
        // Reconcile the WG upstream against the initial config. Same
        // safe-by-construction guarantee as macOS: if the WG handshake
        // doesn't complete within the watchdog window, the routing
        // table stays untouched.
        runtime
            .reconcile_windows_wg_upstream(&config.wireguard_exit)
            .await;
        Ok(runtime)
    }

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

    pub(crate) async fn update_relays(&self, relays: &[String]) -> Result<()> {
        self.mesh.update_relays(relays).await
    }

    pub(crate) fn requires_endpoint_restart(&self, config: &FipsPrivateTunnelConfig) -> bool {
        self.config.iface != config.iface
            || self.config.local_address != config.local_address
            || fips_tunnel_requires_endpoint_restart(&self.config, config)
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
        if self.config.route_targets != config.route_targets {
            crate::windows_tunnel::remove_windows_routes(self.interface_index, &self.route_targets)
                .context("failed to remove stale Windows FIPS routes")?;
            self.route_targets = crate::windows_tunnel::apply_windows_routes(
                self.interface_index,
                &config.route_targets,
            )
            .context("failed to apply Windows FIPS routes")?;
        }
        self.reconcile_windows_wg_upstream(&config.wireguard_exit)
            .await;
        self.config = config;
        Ok(())
    }

    pub(crate) async fn refresh_peer_dependent_routes(&mut self) -> Result<()> {
        Ok(())
    }

    /// Same shape as the macOS reconcile: a no-op if the existing
    /// tunnel already matches, teardown-then-rebuild on config change,
    /// just teardown on disable. Handshake-first, watchdog-protected:
    /// the routing table is only modified after a successful WG
    /// handshake.
    async fn reconcile_windows_wg_upstream(&mut self, wg_config: &WireGuardExitConfig) {
        let want_up = wg_config.enabled && wg_config.configured();
        if want_up
            && self
                .wg_upstream
                .as_ref()
                .is_some_and(|existing| existing.matches(wg_config))
        {
            return;
        }
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
                eprintln!("fips: WG upstream not started: {error}");
            }
        }
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

    pub(crate) async fn broadcast_capabilities(
        &self,
        network_id: &str,
        capabilities: PeerCapabilities,
    ) -> Result<usize> {
        self.mesh
            .broadcast_capabilities(network_id, capabilities)
            .await
    }

    pub(crate) fn peer_advertised_routes(&self, participant: &str) -> Vec<String> {
        self.mesh.peer_advertised_routes(participant)
    }

    pub(crate) fn drain_events(&mut self) -> Vec<FipsPrivateMeshEvent> {
        drain_event_batch(&mut self.event_rx, FIPS_MESH_EVENT_DRAIN_LIMIT)
    }

    pub(crate) async fn stop(self) -> Result<()> {
        let mut runtime = self;
        // Tear the WG upstream down BEFORE the FIPS bits so the route
        // revert lands while we still have a sane working tree.
        if let Some(handle) = runtime.wg_upstream.take() {
            handle.cleanup().await;
        }
        runtime.stop.store(true, Ordering::Relaxed);
        let _ = runtime.session.shutdown();
        if let Err(error) = crate::windows_tunnel::remove_windows_routes(
            runtime.interface_index,
            &runtime.route_targets,
        ) {
            eprintln!("fips: failed to remove Windows FIPS routes: {error}");
        }
        let _ = runtime.tun_read_thread.join();
        runtime.mesh_recv_task.abort();
        let _ = runtime.mesh_recv_task.await;
        if let Ok(mesh) = Arc::try_unwrap(runtime.mesh) {
            mesh.shutdown()
                .await
                .context("failed to stop FIPS endpoint")?;
        }
        Ok(())
    }
}
