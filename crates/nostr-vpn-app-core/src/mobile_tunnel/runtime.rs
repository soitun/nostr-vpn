impl MobileTunnel {
    pub(crate) fn start(config_json: &str) -> Result<Self> {
        mobile_debug_log("MobileTunnel::start parse begin");
        let config: MobileTunnelConfig =
            serde_json::from_str(config_json).context("invalid mobile tunnel config JSON")?;
        mobile_debug_log(format!(
            "MobileTunnel::start parsed peers={} routes={} nostr_relays={} share_lan={} listen={}",
            config.peers.len(),
            config.route_targets.len(),
            config.nostr_relays.len(),
            config.share_local_candidates,
            config.listen_port
        ));
        if !config.error.trim().is_empty() {
            return Err(anyhow!(config.error));
        }
        let app_config = mobile_app_config(&config)?;
        let config = if config.app_config_toml.trim().is_empty() {
            let config_path = non_empty_path(&config.config_path)
                .ok_or_else(|| anyhow!("mobile tunnel config path unavailable"))?;
            MobileTunnelConfig::from_app_with_config_path(&app_config, &config_path)?
        } else {
            config
        };
        mobile_debug_log("MobileTunnel::start building tokio runtime");
        let runtime = RuntimeBuilder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("nvpn-mobile-fips")
            .build()
            .context("failed to start mobile FIPS runtime")?;
        mobile_debug_log("MobileTunnel::start entering start_async");
        let started = runtime.block_on(Self::start_async(config, app_config))?;
        mobile_debug_log("MobileTunnel::start start_async returned");
        Ok(Self {
            runtime,
            endpoint: Some(started.endpoint),
            mesh: started.mesh,
            presence: started.presence,
            config: started.config,
            app_config: started.app_config,
            app_config_dirty: started.app_config_dirty,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            outbound_tx: started.outbound_tx,
            inbound_rx: Some(started.inbound_rx),
            tasks: started.tasks,
            wg_upstream: started.wg_upstream,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            native_tun: None,
            wg_upstream_socket_fd: started.wg_upstream_socket_fd,
        })
    }

    #[allow(clippy::large_futures, clippy::too_many_lines)]
    async fn start_async(
        config: MobileTunnelConfig,
        app_config: AppConfig,
    ) -> Result<MobileTunnelStarted> {
        mobile_debug_log("MobileTunnel::start_async begin");
        let scope = format!("nostr-vpn:{}", config.network_id.trim());
        let initial_peers = config.peers.clone();
        let config_path = non_empty_path(&config.config_path);
        let local_capability_hints = mobile_endpoint_hints(&config);
        mobile_debug_log(format!(
            "MobileTunnel::start_async binding FIPS endpoint scope={} peers={} hints={}",
            scope,
            initial_peers.len(),
            local_capability_hints.len()
        ));
        let endpoint = FipsEndpoint::builder()
            .config(fips_endpoint_config(&scope, &config))
            .identity_nsec(config.identity_nsec.clone())
            .discovery_scope(scope)
            .without_system_tun()
            .bind()
            .await
            .context("failed to bind mobile FIPS endpoint")?;
        mobile_debug_log("MobileTunnel::start_async FIPS endpoint bound");
        let endpoint = Arc::new(endpoint);
        let local_routes = vec![config.local_address.clone()];
        let mesh = Arc::new(RwLock::new(FipsMeshRuntime::with_local_routes(
            initial_peers.clone(),
            local_routes,
        )));
        let peer_identities = Arc::new(RwLock::new(mobile_peer_identity_map(&initial_peers)));
        let mesh_peers = Arc::new(RwLock::new(initial_peers));
        let peer_hints = Arc::new(RwLock::new(config.peer_hints.clone()));
        let presence = Arc::new(RwLock::new(HashMap::new()));
        let config_state = Arc::new(RwLock::new(config.clone()));
        let app_config = Arc::new(RwLock::new(app_config));
        let app_config_dirty = Arc::new(AtomicBool::new(false));
        let (outbound_tx, mut outbound_rx) =
            tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::sync_channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);

        // If the user has WG upstream enabled, stand up the boringtun
        // pump alongside the FIPS endpoint. The WG runtime is fed via
        // an mpsc::channel pair: `wg_send_tx` carries plaintext that
        // should be encapsulated and sent to the upstream;
        // `wg_recv_rx` carries plaintext we got back after
        // decapsulating the upstream's reply, ready to write back to
        // the OS tun.
        let mesh_ipv4 = parse_ipv4(&config.local_address);
        let mut tasks: Vec<JoinHandle<()>> = Vec::new();
        let mut wg_runtime: Option<WgUpstreamRuntime> = None;
        let mut wg_send_tx: Option<tokio_mpsc::Sender<Vec<u8>>> = None;
        let mut wg_socket_fd: c_int = -1;
        let mut wg_address_ipv4: Option<Ipv4Addr> = None;
        if let Some(wg_config) = config.wireguard_exit.as_ref() {
            wg_address_ipv4 = parse_ipv4(&wg_config.address);
            let (send_tx, send_rx) = tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
            let (recv_tx, mut recv_rx) = tokio_mpsc::channel::<Vec<u8>>(TUNNEL_CHANNEL_CAPACITY);
            match WgUpstreamRuntime::start_with_channels(wg_config, send_rx, recv_tx).await {
                Ok(runtime) => {
                    wg_socket_fd = runtime.udp_socket_fd();
                    let upstream = runtime.upstream();
                    let handshake = runtime.handshake_observer();
                    wg_runtime = Some(runtime);
                    wg_send_tx = Some(send_tx);
                    // Forward decrypted WG packets back to the OS as
                    // inbound traffic. DNAT: rewrite the WG-side
                    // destination IP back to the mesh tun address so
                    // the OS routes the reply to the local app stack.
                    let inbound_tx_for_wg = inbound_tx.clone();
                    let wg_addr = wg_address_ipv4;
                    let mesh_addr = mesh_ipv4;
                    tasks.push(tokio::spawn(async move {
                        let mut count: u32 = 0;
                        while let Some(mut packet) = recv_rx.recv().await {
                            count = count.saturating_add(1);
                            // Log first 10 inbound packets so we can
                            // verify the DNAT / packet shape on iOS.
                            if count <= 10 && packet.len() >= 20 && packet[0] >> 4 == 4 {
                                let proto = packet[9];
                                let src = format!(
                                    "{}.{}.{}.{}",
                                    packet[12], packet[13], packet[14], packet[15]
                                );
                                let dst_before = format!(
                                    "{}.{}.{}.{}",
                                    packet[16], packet[17], packet[18], packet[19]
                                );
                                if let (Some(wg), Some(mesh)) = (wg_addr, mesh_addr) {
                                    rewrite_ipv4_destination(&mut packet, wg, mesh);
                                    nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(
                                        &mut packet,
                                    );
                                }
                                let dst_after = format!(
                                    "{}.{}.{}.{}",
                                    packet[16], packet[17], packet[18], packet[19]
                                );
                                log_pump_packet(&format!(
                                    "inbound #{count} {} bytes proto={proto} {src}:* -> {dst_before}->{dst_after}",
                                    packet.len()
                                ));
                            } else if let (Some(wg), Some(mesh)) = (wg_addr, mesh_addr) {
                                rewrite_ipv4_destination(&mut packet, wg, mesh);
                                nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(
                                    &mut packet,
                                );
                            }
                            if inbound_tx_for_wg.send(packet).is_err() {
                                break;
                            }
                        }
                    }));
                    // Watchdog: log if the handshake doesn't complete
                    // promptly, but do not block mobile tunnel startup.
                    // Android must receive the native handle first so it can
                    // call VpnService.protect(fd) on the WG UDP socket before
                    // the default VPN route traps retry traffic.
                    let timeout = DAEMON_WG_UPSTREAM_HANDSHAKE_TIMEOUT;
                    tasks.push(tokio::spawn(async move {
                        if handshake.wait_for_handshake(timeout).await {
                            tracing::info!(
                                ?upstream,
                                "wg-upstream: mobile tunnel handshake completed"
                            );
                        } else {
                            tracing::warn!(
                                ?upstream,
                                "wg-upstream: no handshake within {timeout:?} on mobile tunnel; \
                                 traffic will queue until upstream becomes reachable"
                            );
                        }
                    }));
                }
                Err(error) => {
                    // Don't fail the whole tunnel — FIPS mesh still
                    // works. Just log and continue without WG.
                    tracing::warn!(
                        ?error,
                        "wg-upstream: failed to start mobile WG runtime; continuing without WG upstream"
                    );
                }
            }
        }

        let send_task = {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let peer_identities = Arc::clone(&peer_identities);
            let wg_send_tx_for_dispatch = wg_send_tx.clone();
            let wg_addr = wg_address_ipv4;
            let mesh_addr = mesh_ipv4;
            let inbound_tx_for_dns = inbound_tx.clone();
            let app_config_for_dns = Arc::clone(&app_config);
            let dns_forwarders = mobile_magic_dns_forwarders_for_config(&config);
            tokio::spawn(async move {
                let mut outbound_count: u32 = 0;
                let mut packets = Vec::with_capacity(MOBILE_FIPS_SEND_BATCH);
                while let Some(packet) = outbound_rx.recv().await {
                    drain_mobile_outbound_ready(&mut outbound_rx, &mut packets, packet);
                    if !dispatch_mobile_outbound_packets(
                        &endpoint,
                        &mesh,
                        &peer_identities,
                        wg_send_tx_for_dispatch.as_ref(),
                        wg_addr,
                        mesh_addr,
                        &inbound_tx_for_dns,
                        &app_config_for_dns,
                        &dns_forwarders,
                        &mut outbound_count,
                        &mut packets,
                    )
                    .await
                    {
                        break;
                    }
                }
            })
        };
        tasks.push(send_task);

        let join_request_active = Arc::new(AtomicBool::new(false));
        if let Some((recipient_npub, frame)) = pending_mobile_join_request_frame(&config)? {
            let endpoint = Arc::clone(&endpoint);
            let join_request_active_for_task = Arc::clone(&join_request_active);
            join_request_active.store(true, Ordering::Relaxed);
            tasks.push(tokio::spawn(async move {
                let encoded = match encode_fips_control_frame(&frame) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        tracing::warn!(?error, "mobile: failed to encode FIPS join request");
                        return;
                    }
                };
                while join_request_active_for_task.load(Ordering::Relaxed) {
                    let _ = endpoint.send(recipient_npub.clone(), encoded.clone()).await;
                    tokio::time::sleep(Duration::from_secs(FIPS_JOIN_REQUEST_RETRY_SECS)).await;
                }
            }));
        }

        if !config.network_id.trim().is_empty() && !local_capability_hints.is_empty() {
            let endpoint = Arc::clone(&endpoint);
            let mesh_peers = Arc::clone(&mesh_peers);
            let peer_identities = Arc::clone(&peer_identities);
            let network_id = config.network_id.clone();
            tasks.push(tokio::spawn(async move {
                let mut startup_broadcasts_remaining = MOBILE_CAPABILITIES_STARTUP_BURST_COUNT;
                loop {
                    if let Err(error) = broadcast_mobile_capabilities(
                        &endpoint,
                        &mesh_peers,
                        &peer_identities,
                        &network_id,
                        local_capability_hints.clone(),
                    )
                    .await
                    {
                        tracing::warn!(?error, "mobile: failed to broadcast capabilities");
                    }
                    let sleep_duration = if startup_broadcasts_remaining > 1 {
                        startup_broadcasts_remaining -= 1;
                        Duration::from_millis(MOBILE_CAPABILITIES_STARTUP_BURST_INTERVAL_MS)
                    } else {
                        startup_broadcasts_remaining = 0;
                        Duration::from_secs(MOBILE_CAPABILITIES_BROADCAST_SECS)
                    };
                    tokio::time::sleep(sleep_duration).await;
                }
            }));
        }

        if let Some(status_path) = config_path.as_deref().and_then(mobile_runtime_state_path) {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let presence = Arc::clone(&presence);
            let status_config = Arc::clone(&config_state);
            tasks.push(tokio::spawn(async move {
                loop {
                    if let Err(error) = persist_mobile_runtime_state(
                        &status_path,
                        &endpoint,
                        &mesh,
                        &presence,
                        &status_config,
                    )
                    .await
                    {
                        tracing::warn!(?error, "mobile: failed to persist runtime state");
                    }
                    tokio::time::sleep(Duration::from_secs(MOBILE_RUNTIME_STATE_REFRESH_SECS))
                        .await;
                }
            }));
        }

        if !config.network_id.trim().is_empty() {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let peer_identities = Arc::clone(&peer_identities);
            let presence = Arc::clone(&presence);
            let network_id = config.network_id.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    if let Err(error) = mobile_ping_peers(
                        &endpoint,
                        &mesh,
                        &peer_identities,
                        &presence,
                        &network_id,
                    )
                    .await
                    {
                        tracing::warn!(?error, "mobile: failed to ping FIPS peers");
                    }
                    tokio::time::sleep(Duration::from_secs(MOBILE_RUNTIME_STATE_REFRESH_SECS))
                        .await;
                }
            }));
        }

        if let Some(config_path) = config_path.clone() {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let peer_identities = Arc::clone(&peer_identities);
            let presence = Arc::clone(&presence);
            let app_config = Arc::clone(&app_config);
            tasks.push(tokio::spawn(async move {
                let mut sent_by_peer = HashMap::<String, MobileRosterSentState>::new();
                loop {
                    if let Err(error) = sync_mobile_signed_roster_with_connected_peers(
                        &endpoint,
                        &mesh,
                        &peer_identities,
                        &presence,
                        &app_config,
                        &config_path,
                        &mut sent_by_peer,
                    )
                    .await
                    {
                        tracing::warn!(?error, "mobile: failed to sync signed roster");
                    }
                    tokio::time::sleep(Duration::from_secs(MOBILE_RUNTIME_STATE_REFRESH_SECS))
                        .await;
                }
            }));
        }

        let recv_task = {
            let endpoint = Arc::clone(&endpoint);
            let mesh = Arc::clone(&mesh);
            let mesh_peers = Arc::clone(&mesh_peers);
            let peer_identities = Arc::clone(&peer_identities);
            let peer_hints = Arc::clone(&peer_hints);
            let presence = Arc::clone(&presence);
            let config_state = Arc::clone(&config_state);
            let app_config = Arc::clone(&app_config);
            let app_config_dirty = Arc::clone(&app_config_dirty);
            let config_path = config_path.clone();
            let join_request_active = Arc::clone(&join_request_active);
            let network_id = config.network_id.clone();
            tokio::spawn(async move {
                let mut control_fragments = FipsControlFragmentBuffer::default();
                let mut messages = Vec::with_capacity(MOBILE_FIPS_RECV_BATCH);
                'recv: loop {
                    let Some(_) = endpoint
                        .recv_batch_into(&mut messages, MOBILE_FIPS_RECV_BATCH)
                        .await
                    else {
                        break;
                    };
                    for message in messages.drain(..) {
                        match handle_mobile_endpoint_message(
                            &endpoint,
                            &mesh,
                            &mesh_peers,
                            &peer_identities,
                            &peer_hints,
                            &presence,
                            &config_state,
                            &app_config,
                            &app_config_dirty,
                            config_path.as_deref(),
                            &network_id,
                            &join_request_active,
                            &mut control_fragments,
                            &inbound_tx,
                            message,
                        )
                        .await
                        {
                            Ok(true) => {}
                            Ok(false) => break 'recv,
                            Err(error) => {
                                tracing::warn!(
                                    ?error,
                                    "mobile: failed to handle FIPS control frame"
                                );
                            }
                        }
                    }
                }
            })
        };
        tasks.push(recv_task);

        Ok(MobileTunnelStarted {
            endpoint,
            mesh,
            presence,
            config: config_state,
            app_config,
            app_config_dirty,
            outbound_tx,
            inbound_rx,
            tasks,
            wg_upstream: wg_runtime,
            wg_upstream_socket_fd: wg_socket_fd,
        })
    }

    /// Raw fd of the WG upstream UDP socket, or -1 if WG upstream
    /// isn't running. On Android, the host's `VpnService` calls
    /// `protect(fd)` on this so the encrypted UDP escapes the VPN
    /// tun. iOS relies on the resolved upstream route being declared
    /// as an `excludedRoutes` entry at tunnel-establish time instead.
    pub(crate) fn wg_upstream_socket_fd(&self) -> c_int {
        self.wg_upstream_socket_fd
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn attach_tun_fd(&mut self, fd: c_int) -> Result<()> {
        if fd < 0 {
            return Err(anyhow!("invalid native tun fd"));
        }
        if self.native_tun.is_some() {
            reject_unattached_mobile_tun_fd(fd);
            return Err(anyhow!("native tun fd already attached"));
        }
        let Some(inbound_rx) = self.inbound_rx.take() else {
            reject_unattached_mobile_tun_fd(fd);
            return Err(anyhow!("mobile tunnel inbound packet receiver stopped"));
        };
        let mtu = match self.config.read() {
            Ok(config) => config.mtu,
            Err(_) => {
                self.inbound_rx = Some(inbound_rx);
                reject_unattached_mobile_tun_fd(fd);
                return Err(anyhow!("mobile FIPS config lock poisoned"));
            }
        };
        self.native_tun = Some(NativeTunRuntime::start(
            fd,
            self.outbound_tx.clone(),
            inbound_rx,
            native_tun_packet_capacity(mtu),
        )?);
        Ok(())
    }

    #[cfg(target_os = "ios")]
    pub(crate) fn attach_current_tun_fd(&mut self) -> Result<()> {
        self.attach_tun_fd(current_ios_utun_fd()?)
    }

    pub(crate) fn wg_upstream_excluded_route(&self) -> Option<String> {
        self.wg_upstream
            .as_ref()
            .and_then(|runtime| wg_upstream_excluded_route_for_addr(runtime.upstream()))
    }

    pub(crate) fn runtime_state_json(&self) -> Result<String> {
        let endpoint = self
            .endpoint
            .clone()
            .ok_or_else(|| anyhow!("mobile tunnel stopped"))?;
        let mesh = Arc::clone(&self.mesh);
        let presence = Arc::clone(&self.presence);
        let config = self
            .config
            .read()
            .map_err(|_| anyhow!("mobile FIPS config lock poisoned"))?
            .clone();
        self.runtime.block_on(async move {
            let endpoint_peers = endpoint
                .peers()
                .await
                .context("mobile FIPS peer snapshot")?;
            let relay_statuses = endpoint
                .relay_statuses()
                .await
                .context("mobile FIPS relay snapshot")?;
            let state = {
                let mesh = mesh
                    .read()
                    .map_err(|_| anyhow!("mobile FIPS mesh route table lock poisoned"))?;
                let presence = presence
                    .read()
                    .map_err(|_| anyhow!("mobile FIPS presence lock poisoned"))?;
                mobile_runtime_state(
                    &config,
                    &mesh,
                    &presence,
                    endpoint_peers,
                    relay_statuses,
                    unix_timestamp(),
                )
            };
            serde_json::to_string(&state).context("serialize mobile runtime state")
        })
    }

    pub(crate) fn take_app_config_toml(&self) -> Result<String> {
        if !self.app_config_dirty.swap(false, Ordering::Relaxed) {
            return Ok(String::new());
        }
        let app = self
            .app_config
            .read()
            .map_err(|_| anyhow!("mobile app config lock poisoned"))?;
        let config_path = self
            .config
            .read()
            .map_err(|_| anyhow!("mobile FIPS config lock poisoned"))?
            .config_path
            .clone();
        let config_path = non_empty_path(&config_path).unwrap_or_else(|| PathBuf::from(""));
        match persisted_app_config_toml(&app, &config_path) {
            Ok(toml) => Ok(toml),
            Err(error) => {
                self.app_config_dirty.store(true, Ordering::Relaxed);
                Err(error)
            }
        }
    }

}

struct MobileTunnelStarted {
    endpoint: Arc<FipsEndpoint>,
    mesh: Arc<RwLock<FipsMeshRuntime>>,
    presence: Arc<RwLock<HashMap<String, MobilePeerPresence>>>,
    config: Arc<RwLock<MobileTunnelConfig>>,
    app_config: Arc<RwLock<AppConfig>>,
    app_config_dirty: Arc<AtomicBool>,
    #[cfg_attr(
        not(any(test, target_os = "android", target_os = "ios")),
        allow(dead_code)
    )]
    outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    tasks: Vec<JoinHandle<()>>,
    wg_upstream: Option<WgUpstreamRuntime>,
    wg_upstream_socket_fd: c_int,
}

impl Drop for MobileTunnel {
    fn drop(&mut self) {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let mut native_tun = self.native_tun.take();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if let Some(tun) = native_tun.as_mut() {
            tun.stop();
        }
        let _ = self.inbound_rx.take();
        for task in &self.tasks {
            task.abort();
        }
        let tasks = std::mem::take(&mut self.tasks);
        let endpoint = self.endpoint.take();
        let wg_upstream = self.wg_upstream.take();
        self.runtime.block_on(async move {
            for task in tasks {
                let _ = task.await;
            }
            if let Some(wg) = wg_upstream {
                wg.shutdown().await;
            }
            if let Some(endpoint) = endpoint
                && let Ok(endpoint) = Arc::try_unwrap(endpoint)
            {
                let _ = endpoint.shutdown().await;
            }
        });
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if let Some(mut tun) = native_tun {
            tun.join();
        }
    }
}
