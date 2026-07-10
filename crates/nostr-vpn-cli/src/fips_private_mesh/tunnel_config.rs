#[cfg(feature = "paid-exit")]
pub(crate) fn fips_paid_route_admission_from_seller_admission(
    network_id: &str,
    admission: PaidRouteSellerAdmission,
    destination_allowed_ips: &[String],
) -> FipsPaidRouteAdmission {
    let mut admission = FipsPaidRouteAdmission::from(admission);
    admission.allowed_ips = derive_mesh_tunnel_ip(network_id, &admission.participant_pubkey)
        .map(|tunnel_ip| vec![format!("{}/32", strip_cidr(&tunnel_ip))])
        .unwrap_or_default();
    admission.destination_allowed_ips = destination_allowed_ips.to_vec();
    admission
}

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

        #[cfg(feature = "paid-exit")]
        if let Some(public_paid_exit) = app.public_paid_exit_node_pubkey_hex()
            && Some(public_paid_exit.as_str()) != own_pubkey
        {
            let exit_routes = crate::runtime_exit_node_default_routes();
            route_targets.extend(exit_routes.iter().cloned());
            route_by_participant
                .entry(public_paid_exit)
                .or_default()
                .extend(exit_routes);
        }

        let mut route_participants = app.active_network_signal_pubkeys_hex();
        #[cfg(feature = "paid-exit")]
        if let Some(public_paid_exit) = app.public_paid_exit_node_pubkey_hex() {
            route_participants.push(public_paid_exit);
        }
        route_participants.sort();
        route_participants.dedup();

        for participant in route_participants
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
        #[cfg(feature = "paid-exit")]
        let desired_endpoint_hint_npubs = {
            let mut desired_endpoint_hint_npubs = desired_endpoint_hint_npubs;
            if let Some(public_paid_exit) = app.public_paid_exit_node_pubkey_hex()
                && Some(public_paid_exit.as_str()) != own_pubkey
            {
                desired_endpoint_hint_npubs
                    .insert(normalize_fips_endpoint_npub(&public_paid_exit));
            }
            desired_endpoint_hint_npubs
        };
        let nostr_discovery_policy = fips_nostr_discovery_policy_from_app(app);
        let allow_non_roster_transit = nostr_discovery_policy == NostrDiscoveryPolicy::Open;
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
        if allow_non_roster_transit {
            let bootstrap_transit = filter_static_tunnel_endpoints(
                app.fips_bootstrap_peer_endpoints(),
                &tunnel_endpoint_hosts,
                &local_private_subnets,
            );
            operator_static.extend(cap_static_non_roster_transit_endpoints(
                bootstrap_transit,
                &desired_endpoint_hint_npubs,
                FIPS_STATIC_NON_ROSTER_TRANSIT_MAX_SEEDS,
            ));
        }
        let static_non_roster_transit_seeds = if allow_non_roster_transit {
            non_roster_endpoint_group_count(&operator_static, &desired_endpoint_hint_npubs)
        } else {
            0
        };
        let open_discovery_max_pending = if allow_non_roster_transit {
            open_discovery_limit_after_transit_seeds(static_non_roster_transit_seeds)
        } else {
            0
        };
        // With discovery disabled, configured static endpoints are the whole path;
        // stamped recent/live hints can redirect a deterministic direct setup.
        let stamped_endpoint_hints_enabled = app.fips_nostr_discovery_enabled;
        let mut recent_peer_endpoints = if stamped_endpoint_hints_enabled {
            recent_peers
                .map(|cache| cache.as_static_peer_endpoints_with_seen_at())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
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
            if allow_non_roster_transit {
                FIPS_RECENT_NON_ROSTER_TRANSIT_MAX_SEEDS
            } else {
                0
            },
        );
        // Live capability hints are accepted only for network signal peers because
        // they are claims carried by that peer. The disk cache above is
        // different: it records peers this endpoint already authenticated.
        if stamped_endpoint_hints_enabled {
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
                    desired_endpoint_hint_npubs
                        .contains(&normalize_fips_endpoint_npub(participant))
                }),
            );
        }
        let mut endpoint_peers =
            fips_endpoint_peers_from_mesh(&peers, operator_static, recent_peer_endpoints);
        if !stamped_endpoint_hints_enabled {
            for peer in &mut endpoint_peers {
                if peer.auto_reconnect {
                    peer.discovery_fallback_transit = false;
                }
            }
        }
        route_targets.sort();
        route_targets.dedup();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let fips_host = FipsHostTunnelConfig::from_app(app)?;
        #[cfg(target_os = "linux")]
        let control_plane_bypass_hosts =
            if crate::route_targets_require_endpoint_bypass(&route_targets) {
                crate::control_plane_bypass_ipv4_hosts(app)
            } else {
                Vec::new()
            };

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
            nostr_pubsub: app.nostr.pubsub.clone(),
            control_pubsub_store_path: PathBuf::new(),
            share_local_candidates: app.lan_discovery_enabled,
            peers,
            endpoint_peers,
            route_targets,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            fips_host,
            local_advertised_routes: crate::runtime_effective_advertised_routes(app),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            local_exit_forwarding_routes: crate::runtime_local_exit_forwarding_routes(app),
            paid_route_admissions: Vec::new(),
            #[cfg(feature = "paid-exit")]
            paid_route_accounting_peers: app
                .public_paid_exit_node_pubkey_hex()
                .and_then(|participant_pubkey| {
                    FipsPaidRouteAccountingPeer::parse(
                        &participant_pubkey,
                        FipsPaidRouteAccountingRole::LocalBuyer,
                    )
                })
                .into_iter()
                .collect(),
            #[cfg(feature = "paid-exit")]
            paid_exit: app.paid_exit.clone(),
            #[cfg(feature = "paid-exit")]
            paid_route_store_path: PathBuf::new(),
            #[cfg(feature = "paid-exit")]
            paid_route_wallet_data_dir: PathBuf::new(),
            #[cfg(feature = "paid-exit")]
            paid_route_payment_relays: Vec::new(),
            wireguard_exit: app.wireguard_exit.clone(),
            #[cfg(target_os = "linux")]
            exit_node_leak_protection: app.exit_node_leak_protection,
            nostr_discovery_enabled: app.fips_nostr_discovery_enabled,
            nostr_discovery_policy,
            open_discovery_max_pending,
            mesh_mtu: private_mesh_mtu_from_app(Some(app)),
            #[cfg(target_os = "linux")]
            control_plane_bypass_hosts,
            local_ethernet_underlay: None,
        })
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn use_local_ethernet_only(
        &mut self,
        interface: impl Into<String>,
        discovery_scope: impl Into<String>,
    ) {
        self.local_ethernet_underlay = Some(FipsLocalEthernetUnderlayConfig {
            interface: interface.into(),
            discovery_scope: discovery_scope.into(),
        });
        self.advertised_endpoint.clear();
        self.advertise_public_endpoint = false;
        self.stun_servers.clear();
        self.nostr_relays.clear();
        self.share_local_candidates = false;
        self.nostr_discovery_enabled = false;
        self.nostr_discovery_policy = NostrDiscoveryPolicy::ConfiguredOnly;
        self.open_discovery_max_pending = 0;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            self.fips_host = None;
        }
        #[cfg(target_os = "linux")]
        {
            self.control_plane_bypass_hosts.clear();
        }
    }

    pub(crate) fn clamp_mesh_mtu_to_underlay_interface_mtu(
        &mut self,
        underlay_interface_mtu: Option<u32>,
    ) {
        self.mesh_mtu =
            clamp_mesh_mtu_to_underlay_interface_mtu(self.mesh_mtu, underlay_interface_mtu);
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

    fn local_tunnel_ips(&self) -> Vec<IpAddr> {
        let mut ips = self
            .interface_addresses()
            .into_iter()
            .filter_map(|address| strip_cidr(&address).parse::<IpAddr>().ok())
            .collect::<Vec<_>>();
        ips.sort();
        ips.dedup();
        ips
    }

    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    fn endpoint_hint_ipv4_hosts(&self) -> Vec<Ipv4Addr> {
        let mut hosts = self
            .endpoint_peers
            .iter()
            .flat_map(|peer| peer.addresses.iter())
            .filter_map(|hint| endpoint_transport_ipv4_host(&hint.addr))
            .filter(|host| !route_targets_include_ipv4_host(&self.route_targets, *host))
            .collect::<Vec<_>>();
        hosts.sort_unstable();
        hosts.dedup();
        hosts
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn route_targets_include_ipv4_host(route_targets: &[String], host: Ipv4Addr) -> bool {
    route_targets.iter().any(|route| {
        let Some((target, bits)) = route.split_once('/') else {
            return route.parse::<Ipv4Addr>() == Ok(host);
        };
        bits == "32" && target.parse::<Ipv4Addr>() == Ok(host)
    })
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

fn fips_tunnel_requires_endpoint_restart(
    current: &FipsPrivateTunnelConfig,
    next: &FipsPrivateTunnelConfig,
) -> bool {
    // `endpoint_peers` is deliberately not in this list. Its addresses are fed
    // from recent-peer and live hint refreshes, so treating hint drift as a
    // restart requirement creates endpoint flap loops. Peer roster changes
    // still propagate through `apply_config` -> `mesh.replace_peers`.
    current.identity_nsec != next.identity_nsec
        || current.network_id != next.network_id
        || current.listen_port != next.listen_port
        || current.advertised_endpoint != next.advertised_endpoint
        || current.advertise_public_endpoint != next.advertise_public_endpoint
        || current.stun_servers != next.stun_servers
        || current.nostr_relays != next.nostr_relays
        || current.nostr_pubsub != next.nostr_pubsub
        || current.control_pubsub_store_path != next.control_pubsub_store_path
        || current.nostr_discovery_enabled != next.nostr_discovery_enabled
        || current.share_local_candidates != next.share_local_candidates
        || current.nostr_discovery_policy != next.nostr_discovery_policy
        || current.open_discovery_max_pending != next.open_discovery_max_pending
        || current.mesh_mtu.underlay_udp != next.mesh_mtu.underlay_udp
        || current.local_ethernet_underlay != next.local_ethernet_underlay
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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
    control_pubsub: Option<crate::control_pubsub_runtime::ControlPubsubFipsRuntime>,
    config: FipsPrivateTunnelConfig,
    _tun: Arc<SystemTun>,
    fips_host: Option<crate::fips_host_tunnel::FipsHostTunnelRuntime>,
    tun_send_worker: FipsTunSendWorker,
    mesh_recv_worker: FipsMeshRecvWorker,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    #[cfg(target_os = "linux")]
    endpoint_bypass_routes: Vec<String>,
    #[cfg(target_os = "macos")]
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
