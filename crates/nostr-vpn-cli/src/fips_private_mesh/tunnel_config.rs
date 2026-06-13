impl FipsPrivateTunnelConfig {
    pub(crate) fn from_app(
        app: &AppConfig,
        network_id: &str,
        iface: impl Into<String>,
        own_pubkey: Option<&str>,
        recent_peers: Option<&nostr_vpn_core::recent_peers::RecentPeerEndpoints>,
        live_peer_endpoints: &[(String, Vec<(String, u64)>)],
    ) -> Result<Self> {
        let mut peers = Vec::new();
        let mut route_targets = Vec::new();
        let participants = app.participant_pubkeys_hex();
        let mut route_by_participant = HashMap::<String, Vec<String>>::new();
        for participant in participants {
            if Some(participant.as_str()) == own_pubkey {
                continue;
            }
            let Some(tunnel_ip) = derive_mesh_tunnel_ip(network_id, &participant) else {
                continue;
            };
            let allowed_ip = format!("{}/32", strip_cidr(&tunnel_ip));
            route_targets.push(allowed_ip.clone());
            route_by_participant
                .entry(participant.clone())
                .or_default()
                .push(allowed_ip);
            if app.exit_node == participant {
                let exit_routes = crate::runtime_exit_node_default_routes();
                route_targets.extend(exit_routes.iter().cloned());
                route_by_participant
                    .entry(participant)
                    .or_default()
                    .extend(exit_routes);
            }
        }

        for participant in app
            .active_network_signal_pubkeys_hex()
            .into_iter()
            .filter(|participant| Some(participant.as_str()) != own_pubkey)
        {
            let mut allowed_ips = route_by_participant
                .remove(&participant)
                .unwrap_or_default();
            allowed_ips.sort();
            allowed_ips.dedup();
            peers.push(FipsMeshPeerConfig::from_participant_pubkey(
                participant,
                allowed_ips,
            )?);
        }
        peers.sort_by(|left, right| left.participant_pubkey.cmp(&right.participant_pubkey));
        peers.dedup_by(|left, right| left.participant_pubkey == right.participant_pubkey);
        // Address hints feed into fips's unified `PeerConfig.addresses`:
        //   * operator-configured `fips_peer_endpoints` (unstamped)
        //   * recent-peers cache entries (stamped with `last_success_at`)
        // Persisted private static hints come from old invites and only make
        // sense while we are still on that LAN, so drop them before fips gives
        // configured addresses first shot over discovery/NAT candidates.
        let desired_endpoint_hint_npubs = app
            .active_network_signal_pubkeys_hex()
            .into_iter()
            .filter(|participant| Some(participant.as_str()) != own_pubkey)
            .map(|participant| normalize_fips_endpoint_npub(&participant))
            .collect::<std::collections::HashSet<_>>();
        let tunnel_endpoint_hosts = fips_tunnel_endpoint_hosts(app, network_id);
        let local_private_subnets = local_private_ipv4_subnets();
        // In static-only mode, the configured endpoint is the user's only path.
        // Keep routed private addresses such as VM/container host-only networks
        // even when they are not on a directly attached subnet.
        let mut operator_static = filter_static_tunnel_endpoints_with_policy(
            app.fips_static_peer_endpoints(),
            &tunnel_endpoint_hosts,
            &local_private_subnets,
            !app.fips_nostr_discovery_enabled,
        );
        // Built-in public bootstrap nodes as fallback transit. They share the
        // same `discovery_fallback_transit` path as operator-configured static
        // peers, so they ferry frames when direct traversal fails but never
        // become roster route targets.
        operator_static.extend(filter_static_tunnel_endpoints(
            app.fips_bootstrap_peer_endpoints(),
            &tunnel_endpoint_hosts,
            &local_private_subnets,
        ));
        let static_non_roster_transit_seeds =
            non_roster_endpoint_group_count(&operator_static, &desired_endpoint_hint_npubs);
        let open_discovery_max_pending =
            open_discovery_limit_after_transit_seeds(static_non_roster_transit_seeds);
        let mut recent_peer_endpoints = recent_peers
            .map(|cache| cache.as_static_peer_endpoints_with_seen_at())
            .unwrap_or_default();
        recent_peer_endpoints = filter_stamped_tunnel_endpoints(
            recent_peer_endpoints,
            &tunnel_endpoint_hosts,
            &local_private_subnets,
        );
        // Roster/admin endpoint hints are part of the user's network and are
        // always retained. Recent authenticated non-roster peers are only
        // ambient transit seeds; cap them before handing the list to fips so
        // they don't bypass the open-discovery queue limit as configured peers.
        recent_peer_endpoints = cap_recent_non_roster_transit_endpoints(
            recent_peer_endpoints,
            &desired_endpoint_hint_npubs,
            FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS,
        );
        // Live capability hints are accepted only for network signal peers because
        // they are claims carried by that peer. The disk cache above is
        // different: it records peers this endpoint already authenticated.
        recent_peer_endpoints.extend(
            filter_stamped_tunnel_endpoints(
                live_peer_endpoints
                    .iter()
                    .filter(|(participant, _)| {
                        desired_endpoint_hint_npubs
                            .contains(&normalize_fips_endpoint_npub(participant))
                    })
                    .cloned()
                    .collect(),
                &tunnel_endpoint_hosts,
                &local_private_subnets,
            )
            .into_iter()
            .filter(|(participant, _)| {
                desired_endpoint_hint_npubs.contains(&normalize_fips_endpoint_npub(participant))
            }),
        );
        let endpoint_peers =
            fips_endpoint_peers_from_mesh(&peers, operator_static, recent_peer_endpoints);
        route_targets.sort();
        route_targets.dedup();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let fips_host = FipsHostTunnelConfig::from_app(app)?;

        Ok(Self {
            identity_nsec: app.nostr.secret_key.clone(),
            network_id: network_id.to_string(),
            iface: iface.into(),
            local_address: own_pubkey
                .and_then(|pubkey| derive_mesh_tunnel_ip(network_id, pubkey))
                .map(|tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip))
                .unwrap_or_else(|| local_interface_address_for_tunnel(&app.node.tunnel_ip)),
            listen_port: app.node.listen_port,
            advertised_endpoint: app.node.endpoint.clone(),
            advertise_public_endpoint: app.fips_advertise_public_endpoint,
            stun_servers: app.nat.stun_servers.clone(),
            nostr_relays: app.nostr.relays.clone(),
            share_local_candidates: app.lan_discovery_enabled,
            peers,
            endpoint_peers,
            route_targets,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            fips_host,
            local_advertised_routes: crate::runtime_effective_advertised_routes(app),
            wireguard_exit: app.wireguard_exit.clone(),
            exit_node_leak_protection: app.exit_node_leak_protection,
            connected_udp: app.node.connected_udp.clone(),
            nostr_discovery_enabled: app.fips_nostr_discovery_enabled,
            nostr_discovery_policy: fips_nostr_discovery_policy_from_app(app),
            open_discovery_max_pending,
            mesh_mtu: private_mesh_mtu_from_app(Some(app)),
            #[cfg(target_os = "linux")]
            control_plane_bypass_hosts: crate::control_plane_bypass_ipv4_hosts(app),
        })
    }

    fn local_allowed_ips(&self) -> Vec<String> {
        let mut routes = vec![self.local_address.clone()];
        routes.extend(self.local_advertised_routes.iter().cloned());
        routes.sort();
        routes.dedup();
        routes
    }

    fn interface_addresses(&self) -> Vec<String> {
        let mut addresses = vec![self.local_address.clone()];
        addresses.sort();
        addresses.dedup();
        addresses
    }

    fn interface_route_targets(&self) -> Vec<String> {
        let mut targets = self.route_targets.clone();
        targets.sort();
        targets.dedup();
        targets
    }
}

fn local_interface_address_for_tunnel(tunnel_ip: &str) -> String {
    let tunnel_ip = tunnel_ip.trim();
    if tunnel_ip.is_empty() {
        return "10.44.0.1/32".to_string();
    }
    if tunnel_ip.contains('/') {
        return tunnel_ip.to_string();
    }
    format!("{}/32", strip_cidr(tunnel_ip))
}

fn strip_cidr(value: &str) -> &str {
    value.split('/').next().unwrap_or(value)
}

fn tag_authenticated_transport_addr(
    addr: Option<String>,
    transport_type: Option<String>,
) -> Option<String> {
    let addr = addr?;
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }

    let (addr_transport, host_port) = split_peer_transport_addr(addr);
    let transport = transport_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(addr_transport.as_str())
        .to_ascii_lowercase();

    match transport.as_str() {
        "udp" => Some(host_port),
        "tcp" => Some(format!("tcp:{host_port}")),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn endpoint_transport_ipv4_host(addr: &str) -> Option<Ipv4Addr> {
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return match socket_addr.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            std::net::IpAddr::V6(_) => None,
        };
    }

    let (host, _) = crate::split_host_port(addr, 0)?;
    host.parse::<Ipv4Addr>().ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct FipsPrivateTunnelRuntime {
    iface: String,
    mesh: Arc<FipsPrivateMeshRuntime>,
    config: FipsPrivateTunnelConfig,
    _tun: Arc<SystemTun>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    fips_host: Option<crate::fips_host_tunnel::FipsHostTunnelRuntime>,
    tun_read_task: JoinHandle<()>,
    mesh_send_task: JoinHandle<()>,
    mesh_recv_worker: FipsMeshRecvWorker,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    #[cfg(target_os = "linux")]
    endpoint_bypass_routes: Vec<String>,
    #[cfg(target_os = "linux")]
    original_default_route: Option<String>,
    #[cfg(target_os = "linux")]
    original_default_ipv6_route: Option<String>,
    #[cfg(target_os = "linux")]
    exit_node_runtime: crate::LinuxExitNodeRuntime,
    #[cfg(target_os = "macos")]
    exit_node_runtime: crate::MacosExitNodeRuntime,
    /// Userspace WG upstream tunnel (Mullvad/Proton-style). Owned for
    /// the lifetime of "WG upstream is enabled in config"; dropped on
    /// disable. Populated by `reconcile_macos_wg_upstream` after a
    /// successful handshake — `None` means either WG upstream is
    /// disabled in the config or the most recent reconcile attempt
    /// could not complete a handshake (in which case the routing
    /// table was deliberately left untouched).
    #[cfg(target_os = "macos")]
    wg_upstream: Option<crate::wg_upstream_runtime::DaemonWgUpstream>,
}
