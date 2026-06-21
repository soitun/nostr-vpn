#[derive(Debug, Clone)]
struct FipsEndpointTransportConfig {
    listen_port: u16,
    advertised_endpoint: String,
    advertise_public_endpoint: bool,
    /// Find/advertise peers over Nostr relays. When false, the endpoint still
    /// dials static/bootstrap peers and does LAN discovery, but does not use
    /// relays for endpoint discovery or advertising.
    nostr_discovery_enabled: bool,
    stun_servers: Vec<String>,
    nostr_relays: Vec<String>,
    share_local_candidates: bool,
}

/// Address hint carried through nvpn's intermediate config types before
/// being lowered into a fips `PeerAddress`. `seen_at_ms` is the
/// most-recent observation timestamp (Unix ms) when we have one — set for
/// recent-peers cache entries, `None` for operator-supplied static hints.
/// fips's dialer uses this field as a recency tiebreaker inside the same
/// priority tier.
const FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY: u8 = 100;
const FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY: u8 = 100;
const FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY: u8 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FipsPeerAddressHint {
    pub(crate) addr: String,
    pub(crate) seen_at_ms: Option<u64>,
    pub(crate) priority: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FipsEndpointPeerTransportConfig {
    pub(crate) npub: String,
    pub(crate) addresses: Vec<FipsPeerAddressHint>,
    pub(crate) auto_reconnect: bool,
    pub(crate) discovery_fallback_transit: bool,
}

fn fips_peer_address_from_hint(hint: &FipsPeerAddressHint) -> PeerAddress {
    let (transport, addr) = split_peer_transport_addr(&hint.addr);
    let mut peer_address = PeerAddress::with_priority(
        transport,
        addr,
        peer_address_hint_effective_priority(hint),
    );
    if let Some(seen_at_ms) = hint.seen_at_ms {
        peer_address = peer_address.with_seen_at_ms(seen_at_ms);
    }
    peer_address
}

fn operator_static_endpoint_priority(addr: &str) -> u8 {
    endpoint_hint_priority(addr, FIPS_PUBLIC_PEER_ENDPOINT_PRIORITY)
}

fn dynamic_endpoint_priority(addr: &str) -> u8 {
    endpoint_hint_priority(addr, FIPS_DYNAMIC_PEER_ENDPOINT_PRIORITY)
}

fn peer_address_hint_effective_priority(hint: &FipsPeerAddressHint) -> u8 {
    endpoint_hint_priority(&hint.addr, hint.priority)
}

fn endpoint_hint_priority(addr: &str, normal_priority: u8) -> u8 {
    if endpoint_addr_is_private_or_local(addr) {
        FIPS_PRIVATE_STATIC_PEER_ENDPOINT_PRIORITY
    } else {
        normal_priority
    }
}

fn endpoint_addr_is_private_or_local(addr: &str) -> bool {
    endpoint_addr_ip(addr).is_some_and(endpoint_ip_is_private_or_local)
}

fn endpoint_ip_is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ipv4_is_cgnat_addr(ip)
                || ip.is_link_local()
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ipv4_is_benchmark_addr(ip)
        }
        IpAddr::V6(ip) => {
            ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
        }
    }
}

fn ipv4_is_benchmark_addr(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    octets[0] == 198 && (18..=19).contains(&octets[1])
}

fn fips_endpoint_config(
    peers: &[FipsEndpointPeerTransportConfig],
    transport: Option<&FipsEndpointTransportConfig>,
    mesh_mtu: MeshMtu,
    nostr_discovery_policy: NostrDiscoveryPolicy,
) -> Config {
    fips_endpoint_config_with_open_discovery_limit(
        peers,
        transport,
        mesh_mtu,
        nostr_discovery_policy,
        FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING,
        None,
    )
}

fn fips_endpoint_config_with_open_discovery_limit(
    peers: &[FipsEndpointPeerTransportConfig],
    transport: Option<&FipsEndpointTransportConfig>,
    mesh_mtu: MeshMtu,
    nostr_discovery_policy: NostrDiscoveryPolicy,
    open_discovery_max_pending: usize,
    connected_udp: Option<&ConnectedUdpConfig>,
) -> Config {
    let mut config = Config::new();
    apply_connected_udp_config(&mut config, connected_udp);
    config.node.control.enabled = false;
    // App mesh peers may be routable only through already-connected
    // neighbors when direct NAT traversal fails. Reply-learned routing lets
    // first-contact EndpointData trigger discovery through those neighbors.
    config.node.routing.mode = RoutingMode::ReplyLearned;
    config.node.retry.base_interval_secs = FIPS_RECONNECT_BACKOFF_BASE_SECS;
    config.node.retry.max_backoff_secs = FIPS_RECONNECT_BACKOFF_MAX_SECS;
    config.node.heartbeat_interval_secs = FIPS_ENDPOINT_HEARTBEAT_INTERVAL_SECS;
    config.node.link_dead_timeout_secs = FIPS_ENDPOINT_LINK_DEAD_TIMEOUT_SECS;
    config.node.fast_link_dead_timeout_secs = FIPS_ENDPOINT_FAST_LINK_DEAD_TIMEOUT_SECS;
    config.node.session.idle_timeout_secs = FIPS_ENDPOINT_SESSION_IDLE_TIMEOUT_SECS;
    config.node.session.pending_packets_per_dest = FIPS_ENDPOINT_PENDING_PACKETS_PER_DEST;
    config.node.rekey.after_secs = FIPS_ENDPOINT_REKEY_AFTER_SECS;
    config.dns.enabled = false;
    // nvpn keeps public/open discovery available as a fallback, but it should
    // be polite to public transit nodes when stale roster peers or cached
    // adverts cannot be reached.
    config.node.discovery.backoff_base_secs = FIPS_DISCOVERY_BACKOFF_BASE_SECS;
    config.node.discovery.backoff_max_secs = FIPS_DISCOVERY_BACKOFF_MAX_SECS;
    config.node.discovery.forward_min_interval_secs = FIPS_DISCOVERY_FORWARD_MIN_INTERVAL_SECS;
    let advertise_public_endpoint = transport
        .map(|transport| transport.advertise_public_endpoint)
        .unwrap_or(false);
    // The "find peers over Nostr relays" toggle. When off we neither advertise
    // nor stream adverts/DMs, but static + bootstrap peers (config.peers below)
    // are still dialed directly and LAN discovery still runs.
    let nostr_discovery_enabled = transport
        .map(|transport| transport.nostr_discovery_enabled)
        .unwrap_or(true);
    let advertise_on_nostr = nostr_discovery_enabled && transport.is_some();
    let nostr_enabled = nostr_discovery_enabled && (transport.is_some() || !peers.is_empty());
    config.node.discovery.nostr.enabled = nostr_enabled;
    config.node.discovery.nostr.advertise = advertise_on_nostr;
    // Open discovery by default (unless the user opts into configured-only
    // discovery) so we can FIPS-handshake with any nvpn node we see on relays,
    // not just configured roster peers. This is what lets us route app-mesh
    // traffic through transit hops that aren't in our network roster (a
    // friend-of-a-friend nvpn node can ferry our packets when direct
    // traversal fails). Security boundary: the FIPS handshake can be open; the
    // per-network data plane is NOT. `FipsMeshRuntime::receive_endpoint_data*`
    // drops every inbound packet whose source npub doesn't own the inner
    // source IP per our roster, so a non-roster transit peer can carry frames
    // but cannot inject anything that surfaces on the tun. See the
    // `inbound_endpoint_data_*` tests in `nostr-vpn-core::fips_mesh`.
    // Headless e2e meshes can force configured-only discovery to avoid
    // contending with ambient public relay traffic.
    config.node.discovery.nostr.policy = nostr_discovery_policy;
    config.node.discovery.nostr.open_discovery_max_pending = open_discovery_max_pending;
    config.node.discovery.nostr.failure_streak_threshold = FIPS_NOSTR_FAILURE_STREAK_THRESHOLD;
    config.node.discovery.nostr.extended_cooldown_secs = FIPS_NOSTR_EXTENDED_COOLDOWN_SECS;
    config.node.discovery.nostr.startup_sweep_max_age_secs = FIPS_NOSTR_STARTUP_SWEEP_MAX_AGE_SECS;
    config.node.discovery.nostr.share_local_candidates = transport
        .map(|transport| transport.share_local_candidates)
        .unwrap_or(false);
    config.node.discovery.lan.enabled = transport
        .map(|transport| transport.share_local_candidates)
        .unwrap_or(false);
    // Leave the relay-side `app` at fips-core's default ("fips-overlay-v1").
    // We deliberately do NOT bake the per-network mesh id into it: the relay
    // `protocol` tag is publicly visible, so per-network apps would let any
    // observer count members of each private network. The builder receives a
    // hashed per-network LAN discovery scope separately; that scope is carried
    // only in mDNS TXT records on the local link, while the private data plane
    // still enforces roster ownership before packets reach the tun.
    let bind_addr = transport.map(fips_udp_bind_addr);
    let external_addr = transport
        .filter(|_| advertise_public_endpoint)
        .and_then(fips_udp_external_addr);
    if let Some(transport) = transport {
        config.node.discovery.nostr.stun_servers = transport.stun_servers.clone();
        if !transport.nostr_relays.is_empty() {
            config.node.discovery.nostr.advert_relays = transport.nostr_relays.clone();
            config.node.discovery.nostr.dm_relays = transport.nostr_relays.clone();
        }
    }
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr,
        advertise_on_nostr: Some(advertise_on_nostr),
        public: Some(advertise_public_endpoint),
        external_addr,
        outbound_only: Some(transport.is_none()),
        accept_connections: Some(transport.is_some()),
        // The safe default remains IPv6-minimum sized for NAT traversal and
        // nested tunnels. Clean-LAN tests must opt into a larger paired budget
        // through config or NVPN_MESH_* env overrides.
        mtu: Some(mesh_mtu.underlay_udp),
        send_buf_size: fips_udp_send_buf_size(),
        ..UdpConfig::default()
    });
    // Outbound TCP transport so peers reachable only over tcp:443 (e.g. on
    // networks that block UDP outright) can still be dialed. bind_addr=None keeps
    // it outbound-only — no listener.
    let needs_tcp = peers.iter().any(|peer| {
        peer.addresses
            .iter()
            .any(|hint| split_peer_transport_addr(&hint.addr).0 == "tcp")
    });
    if needs_tcp {
        // Default = outbound-only (no bind_addr). Inferred type keeps this the
        // exact `TcpConfig` of `config.transports`, which matters under the e2e's
        // [patch.crates-io] where a second fips-core version can be in the graph,
        // so we deliberately keep `Default::default()` over `TcpConfig::default()`.
        #[allow(clippy::default_trait_access)]
        {
            config.transports.tcp = TransportInstances::Single(Default::default());
        }
    }
    config.peers = peers
        .iter()
        .map(|peer| FipsPeerConfig {
            npub: peer.npub.clone(),
            alias: None,
            addresses: peer
                .addresses
                .iter()
                .map(fips_peer_address_from_hint)
                .collect(),
            connect_policy: ConnectPolicy::AutoConnect,
            auto_reconnect: peer.auto_reconnect,
            discovery_fallback_transit: peer.discovery_fallback_transit,
        })
        .collect();
    config
}

fn apply_connected_udp_config(config: &mut Config, connected_udp: Option<&ConnectedUdpConfig>) {
    let Some(connected_udp) = connected_udp else {
        return;
    };
    if let Some(enabled) = connected_udp.enabled {
        config.node.connected_udp.enabled = enabled;
    }
    if let Some(fd_reserve) = connected_udp.fd_reserve {
        config.node.connected_udp.fd_reserve = fd_reserve;
    }
}

fn fips_endpoint_peers_from_mesh(
    mesh_peers: &[FipsMeshPeerConfig],
    operator_static_endpoints: Vec<(String, Vec<String>)>,
    recent_peer_endpoints: Vec<(String, Vec<(String, u64)>)>,
) -> Vec<FipsEndpointPeerTransportConfig> {
    let mut peers = HashMap::<String, FipsEndpointPeerTransportConfig>::new();
    for peer in mesh_peers {
        let npub = normalize_fips_endpoint_npub(&peer.endpoint_npub);
        peers
            .entry(npub.clone())
            .or_insert_with(|| FipsEndpointPeerTransportConfig {
                npub,
                addresses: Vec::new(),
                auto_reconnect: true,
                discovery_fallback_transit: !peer.advertises_default_route(),
            });
    }

    // Operator-configured hints have no freshness signal. If a duplicate
    // address later appears in the recent cache, keep the operator hint static.
    for (npub, addresses) in operator_static_endpoints {
        let npub = normalize_fips_endpoint_npub(&npub);
        let peer = peers
            .entry(npub.clone())
            .or_insert_with(|| FipsEndpointPeerTransportConfig {
                npub,
                addresses: Vec::new(),
                auto_reconnect: false,
                discovery_fallback_transit: true,
            });
        for raw in addresses {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let priority = operator_static_endpoint_priority(trimmed);
            if let Some(existing) = peer.addresses.iter_mut().find(|hint| hint.addr == trimmed) {
                existing.seen_at_ms = None;
                existing.priority = existing.priority.min(priority);
                continue;
            }
            peer.addresses.push(FipsPeerAddressHint {
                addr: trimmed.to_string(),
                seen_at_ms: None,
                priority,
            });
        }
    }

    // Recent-peers cache entries arrive with `last_success_at_ms` so fips
    // can prefer fresher hints within the same priority tier. Authenticated
    // non-roster entries are kept
    // too: those are overlay transit peers we successfully handshook with
    // before, and reseeding them as fallback transit keeps the FIPS overlay
    // useful before relay discovery catches up.
    for (npub, addresses) in recent_peer_endpoints {
        let npub = normalize_fips_endpoint_npub(&npub);
        let peer = peers
            .entry(npub.clone())
            .or_insert_with(|| FipsEndpointPeerTransportConfig {
                npub,
                addresses: Vec::new(),
                auto_reconnect: false,
                discovery_fallback_transit: true,
            });
        for (addr, seen_at_ms) in addresses {
            let trimmed = addr.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Same (npub, addr) from multiple dynamic sources: keep the
            // freshest timestamp. If an operator static hint already owns this
            // socket, do not stamp it as recent; fips uses that distinction to
            // keep the configured LAN path preferred during retries.
            if let Some(existing) = peer.addresses.iter_mut().find(|hint| hint.addr == trimmed) {
                if let Some(existing_seen_at_ms) = existing.seen_at_ms {
                    let priority = dynamic_endpoint_priority(trimmed);
                    existing.seen_at_ms = Some(existing_seen_at_ms.max(seen_at_ms));
                    existing.priority = existing.priority.min(priority);
                }
                continue;
            }
            peer.addresses.push(FipsPeerAddressHint {
                addr: trimmed.to_string(),
                seen_at_ms: Some(seen_at_ms),
                priority: dynamic_endpoint_priority(trimmed),
            });
        }
    }

    let mut peers = peers.into_values().collect::<Vec<_>>();
    for peer in &mut peers {
        peer.addresses.sort_by(|a, b| a.addr.cmp(&b.addr));
        peer.addresses.dedup_by(|a, b| a.addr == b.addr);
    }
    peers.sort_by(|left, right| left.npub.cmp(&right.npub));
    peers
}

fn normalize_fips_endpoint_npub(value: &str) -> String {
    let trimmed = value.trim();
    normalize_nostr_pubkey(trimmed)
        .ok()
        .and_then(|pubkey| {
            PublicKey::from_hex(&pubkey)
                .ok()
                .and_then(|public_key| public_key.to_bech32().ok())
        })
        .unwrap_or_else(|| trimmed.to_string())
}

fn participant_pubkey_bytes(value: &str) -> Option<ParticipantPubkeyBytes> {
    PublicKey::parse(value.trim())
        .ok()
        .map(|pubkey| *pubkey.as_bytes())
}

fn fips_udp_bind_addr(transport: &FipsEndpointTransportConfig) -> String {
    SocketAddr::V4(SocketAddrV4::new(
        std::net::Ipv4Addr::UNSPECIFIED,
        transport.listen_port,
    ))
    .to_string()
}

fn fips_udp_external_addr(transport: &FipsEndpointTransportConfig) -> Option<String> {
    let endpoint = transport.advertised_endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    let parsed = endpoint.parse::<SocketAddr>().ok()?;
    if endpoint_ip_is_private_or_local(parsed.ip()) {
        return None;
    }
    Some(parsed.to_string())
}

#[derive(Debug, Clone)]
pub(crate) struct FipsPrivateTunnelConfig {
    pub(crate) identity_nsec: String,
    pub(crate) network_id: String,
    pub(crate) iface: String,
    pub(crate) local_address: String,
    pub(crate) listen_port: u16,
    pub(crate) advertised_endpoint: String,
    pub(crate) advertise_public_endpoint: bool,
    pub(crate) stun_servers: Vec<String>,
    pub(crate) nostr_relays: Vec<String>,
    pub(crate) share_local_candidates: bool,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    pub(crate) endpoint_peers: Vec<FipsEndpointPeerTransportConfig>,
    pub(crate) route_targets: Vec<String>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fips_host: Option<FipsHostTunnelConfig>,
    pub(crate) local_advertised_routes: Vec<String>,
    pub(crate) paid_route_admissions: Vec<FipsPaidRouteAdmission>,
    pub(crate) paid_exit: PaidExitConfig,
    pub(crate) paid_route_store_path: PathBuf,
    pub(crate) paid_route_wallet_data_dir: PathBuf,
    pub(crate) paid_route_payment_relays: Vec<String>,
    pub(crate) wireguard_exit: WireGuardExitConfig,
    pub(crate) exit_node_leak_protection: bool,
    connected_udp: ConnectedUdpConfig,
    nostr_discovery_enabled: bool,
    nostr_discovery_policy: NostrDiscoveryPolicy,
    open_discovery_max_pending: usize,
    mesh_mtu: MeshMtu,
    #[cfg(target_os = "linux")]
    pub(crate) control_plane_bypass_hosts: Vec<Ipv4Addr>,
}
