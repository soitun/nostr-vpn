const DEFAULT_MOBILE_MTU: u16 = nostr_vpn_core::MESH_TUNNEL_MTU;
const TUNNEL_CHANNEL_CAPACITY: usize = 1024;
#[cfg(test)]
const FIPS_NOSTR_DISCOVERY_APP: &str = "fips-overlay-v1";
const FIPS_DISCOVERY_BACKOFF_BASE_SECS: u64 = 30;
const FIPS_DISCOVERY_BACKOFF_MAX_SECS: u64 = 300;
const FIPS_DISCOVERY_FORWARD_MIN_INTERVAL_SECS: u64 = 30;
const MOBILE_NOSTR_OPEN_DISCOVERY_MAX_PENDING: usize = 4;
const MOBILE_NOSTR_FAILURE_STREAK_THRESHOLD: u32 = 2;
const FIPS_NOSTR_STARTUP_SWEEP_MAX_AGE_SECS: u64 = 300;

/// Authenticated FIPS peer cap on mobile. fips's default is 128, which is
/// fine on AC-powered desktops but wasteful on phones once Open discovery
/// starts pulling in random nvpn nodes who have nothing to say to us at
/// the data plane (the roster gate drops their packets anyway).
const MOBILE_MAX_FIPS_PEERS: usize = 32;
/// Pre-handshake connection cap on mobile (~2x peer cap matches fips's
/// default ratio of 256:128).
const MOBILE_MAX_FIPS_CONNECTIONS: usize = 64;
/// Active-link cap on mobile (matches `MOBILE_MAX_FIPS_CONNECTIONS`).
const MOBILE_MAX_FIPS_LINKS: usize = 64;
const MOBILE_CAPABILITIES_BROADCAST_SECS: u64 = 60;
const MOBILE_CAPABILITIES_STARTUP_BURST_COUNT: usize = 4;
const MOBILE_CAPABILITIES_STARTUP_BURST_INTERVAL_MS: u64 = 750;
const MOBILE_RUNTIME_STATE_REFRESH_SECS: u64 = 10;
const MOBILE_ROSTER_RESEND_SECS: u64 = 60;
const MOBILE_RUNTIME_STATE_FILE: &str = "mobile-runtime-state.json";
const MOBILE_PEER_ONLINE_GRACE_SECS: u64 = 45;
const MOBILE_PEER_MAX_FUTURE_SKEW_SECS: u64 = 2;
const MOBILE_PEER_ACTIVE_PING_INTERVAL_SECS: u64 = 30;
// Nostr subscriptions and minute-cadence LAN scans provide the fast paths;
// this is only the battery-safe fallback for peers that remain offline.
const MOBILE_PEER_DISCOVERY_PROBE_INTERVAL_SECS: u64 = 300;
const MOBILE_CONTROL_RTT_MAX_ACCEPT_MS: u128 = 10_000;
const MOBILE_HANDSHAKE_RESEND_INTERVAL_MS: u64 = 300;
const MOBILE_HANDSHAKE_RESEND_BACKOFF: f64 = 1.5;
// Bounded burst receive: enough to amortize endpoint wakeups without letting
// mobile packet delivery monopolize the small runtime.
const MOBILE_FIPS_RECV_BATCH: usize = 64;
// Mirror the receive bound on outbound ready-burst draining. Consecutive
// packets to the same resolved peer can share one endpoint command batch.
const MOBILE_FIPS_SEND_BATCH: usize = 64;
const MOBILE_TUN_OUTBOUND_BATCH_CHANNEL_CAPACITY: usize =
    TUNNEL_CHANNEL_CAPACITY / MOBILE_FIPS_SEND_BATCH;
const MOBILE_TUN_INBOUND_BATCH_CHANNEL_CAPACITY: usize =
    TUNNEL_CHANNEL_CAPACITY / MOBILE_FIPS_RECV_BATCH;
const MOBILE_EXIT_NODE_DEFAULT_ROUTES: &[&str] = &["0.0.0.0/0"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MobileTunCounters {
    packets_read: u64,
    bytes_read: u64,
    packets_written: u64,
    bytes_written: u64,
    packets_dropped: u64,
}

#[derive(Default)]
struct MobileTunAtomicCounters {
    packets_read: std::sync::atomic::AtomicU64,
    bytes_read: std::sync::atomic::AtomicU64,
    packets_written: std::sync::atomic::AtomicU64,
    bytes_written: std::sync::atomic::AtomicU64,
    packets_dropped: std::sync::atomic::AtomicU64,
}

impl MobileTunAtomicCounters {
    fn snapshot(&self) -> MobileTunCounters {
        MobileTunCounters {
            packets_read: self.packets_read.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            packets_written: self.packets_written.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            packets_dropped: self.packets_dropped.load(Ordering::Relaxed),
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn note_read(&self, len: usize) {
        self.packets_read.fetch_add(1, Ordering::Relaxed);
        self.bytes_read
            .fetch_add(u64::try_from(len).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn note_write(&self, len: usize) {
        self.packets_written.fetch_add(1, Ordering::Relaxed);
        self.bytes_written
            .fetch_add(u64::try_from(len).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn note_drop(&self) {
        self.packets_dropped.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct MobileTunnelConfig {
    #[serde(default)]
    pub(crate) config_path: String,
    #[serde(default)]
    pub(crate) app_config_toml: String,
    pub(crate) identity_nsec: String,
    #[serde(default)]
    pub(crate) node_name: String,
    pub(crate) network_id: String,
    #[serde(default)]
    pub(crate) invite_secret: String,
    pub(crate) local_address: String,
    #[serde(default)]
    pub(crate) advertised_endpoint: String,
    #[serde(default)]
    pub(crate) listen_port: u16,
    pub(crate) mtu: u16,
    pub(crate) peers: Vec<FipsMeshPeerConfig>,
    #[serde(default)]
    peer_hints: HashMap<String, Vec<FipsPeerAddressHint>>,
    /// Bootstrap/transit peers (npub -> transport-tagged hints), kept separate
    /// from learned `peer_hints` because these are dialed as fallback transit
    /// (fanout), whereas learned hints only seed direct reconnects.
    #[serde(default)]
    bootstrap_peers: HashMap<String, Vec<FipsPeerAddressHint>>,
    pub(crate) route_targets: Vec<String>,
    #[serde(default)]
    pub(crate) nostr_relays: Vec<String>,
    #[serde(default)]
    pub(crate) stun_servers: Vec<String>,
    #[serde(default)]
    pub(crate) share_local_candidates: bool,
    #[serde(default)]
    pub(crate) connect_to_non_roster_fips_peers: bool,
    /// Find/advertise peers over Nostr relays. When false, the tunnel still
    /// dials static + bootstrap peers and (when allowed) LAN, but does not use
    /// relays for endpoint discovery or advertising.
    #[serde(default = "default_true")]
    pub(crate) nostr_discovery_enabled: bool,
    /// Browser-compatible WebRTC is opt-in on native/mobile. Relay discovery
    /// still discovers UDP/TCP endpoints while this is false.
    #[serde(default)]
    pub(crate) webrtc_enabled: bool,
    /// When the user has WG upstream enabled + configured, the OS-side
    /// (`NEPacketTunnelProvider` on iOS, `VpnService` on Android) is
    /// expected to:
    ///   * include `0.0.0.0/0` in the tunnel's includedRoutes (so all
    ///     non-mesh outbound traffic enters the tun and we can forward
    ///     it to boringtun)
    ///   * route every IP in `excluded_routes` outside the tunnel so
    ///     the encrypted UDP can actually reach the WG upstream
    ///     endpoint (iOS does this via `NEIPv4Settings.excludedRoutes`
    ///     and also asks the running tunnel for the resolved endpoint
    ///     route; on Android the host calls `VpnService.protect(socket_fd)`
    ///     instead, see `MobileTunnel::wg_upstream_socket_fd`).
    #[serde(default)]
    pub(crate) excluded_routes: Vec<String>,
    /// Local in-tunnel DNS address installed on the OS-side tunnel. Public
    /// queries are resolved through authenticated DNS-over-HTTPS by Rust.
    #[serde(default)]
    pub(crate) dns_servers: Vec<String>,
    /// In-tunnel local DNS responder. Empty only when neither an exit nor
    /// `MagicDNS` requires nvpn to own DNS resolution.
    #[serde(default)]
    pub(crate) magic_dns_server: String,
    /// The WG upstream config to drive boringtun against. None when
    /// the user hasn't enabled WG upstream — in which case the mobile
    /// tunnel runs in pure FIPS-mesh mode.
    #[serde(default)]
    pub(crate) wireguard_exit: Option<WireGuardExitConfig>,
    #[serde(default)]
    pub(crate) join_requests_enabled: bool,
    #[serde(default)]
    pub(crate) pending_join_request_recipient: String,
    #[serde(default)]
    pub(crate) pending_join_invite_secret: String,
    #[serde(default)]
    pub(crate) pending_join_requested_at: u64,
    #[serde(default)]
    pub(crate) error: String,
}

fn default_true() -> bool {
    true
}

fn preserve_mobile_pending_exit_default_routes(
    route_targets: &mut Vec<String>,
    exit_node: &str,
    leak_protection: bool,
    internet_source: Option<&str>,
) {
    if !exit_node.trim().is_empty() || !leak_protection {
        return;
    }
    if !matches!(
        internet_source,
        Some("private_vpn" | "paid_automatic" | "paid_manual")
    ) {
        return;
    }
    route_targets.extend(
        MOBILE_EXIT_NODE_DEFAULT_ROUTES
            .iter()
            .map(|route| (*route).to_string()),
    );
}

fn serialized_mobile_internet_source(app: &AppConfig) -> Option<String> {
    toml::Value::try_from(app)
        .ok()?
        .get("internet_source")?
        .as_str()
        .map(str::to_string)
}

fn active_mobile_wireguard_dns_servers(config: &MobileTunnelConfig) -> Vec<Ipv4Addr> {
    let Some(wireguard) = config
        .wireguard_exit
        .as_ref()
        .filter(|wireguard| wireguard.enabled && wireguard.configured())
    else {
        return Vec::new();
    };
    let local_dns = parse_ipv4(nostr_vpn_core::MESH_MAGIC_DNS_SERVER);
    wireguard
        .dns_server_ips()
        .into_iter()
        .filter_map(|server| match server {
            IpAddr::V4(server) if Some(server) != local_dns => Some(server),
            IpAddr::V4(_) | IpAddr::V6(_) => None,
        })
        .collect()
}

impl MobileTunnelConfig {
    pub(crate) fn from_data_dir(data_dir: &str) -> Result<Self> {
        let config_path = native_config_path(data_dir);
        let mut app = if config_path.exists() {
            AppConfig::migrate_persisted_secrets(&config_path)?;
            AppConfig::load(&config_path)?
        } else {
            let generated = AppConfig::generated_without_networks();
            generated.save(&config_path)?;
            generated
        };
        app.ensure_defaults();
        maybe_autoconfigure_node(&mut app);
        app.save(&config_path)?;
        Self::from_app_with_config_path(&app, &config_path)
    }

    #[cfg(test)]
    fn from_app(app: &AppConfig) -> Result<Self> {
        Self::from_app_with_config_path(app, Path::new(""))
    }

    #[allow(clippy::too_many_lines)]
    fn from_app_with_config_path(app: &AppConfig, config_path: &Path) -> Result<Self> {
        let own_pubkey = app.own_nostr_pubkey_hex()?;
        let network_id = app.effective_network_id();
        let mut peers = Vec::new();
        let mut route_targets = Vec::new();
        let participant_pubkeys = app
            .participant_pubkeys_hex()
            .into_iter()
            .collect::<HashSet<_>>();

        for participant in app
            .active_network_signal_pubkeys_hex()
            .into_iter()
            .filter(|participant| participant != &own_pubkey)
        {
            let mut allowed_ips = if participant_pubkeys.contains(&participant) {
                let Some(tunnel_ip) = derive_mesh_tunnel_ip(&network_id, &participant) else {
                    continue;
                };
                let route = format!("{}/32", strip_cidr(&tunnel_ip));
                route_targets.push(route.clone());
                vec![route]
            } else {
                Vec::new()
            };
            if app.exit_node == participant {
                let exit_routes = MOBILE_EXIT_NODE_DEFAULT_ROUTES
                    .iter()
                    .map(|route| (*route).to_string())
                    .collect::<Vec<_>>();
                route_targets.extend(exit_routes.iter().cloned());
                allowed_ips.extend(exit_routes);
            }
            allowed_ips.sort();
            allowed_ips.dedup();
            peers.push(FipsMeshPeerConfig::from_participant_pubkey(
                participant,
                allowed_ips,
            )?);
        }

        if !network_id.trim().is_empty()
            && !route_targets
                .iter()
                .any(|route| route == MESH_TUNNEL_IPV4_CIDR)
        {
            route_targets.push(MESH_TUNNEL_IPV4_CIDR.to_string());
        }
        let internet_source = serialized_mobile_internet_source(app);
        preserve_mobile_pending_exit_default_routes(
            &mut route_targets,
            &app.exit_node,
            app.exit_node_leak_protection,
            internet_source.as_deref(),
        );
        peers.sort_by(|left, right| left.participant_pubkey.cmp(&right.participant_pubkey));
        peers.dedup_by(|left, right| left.participant_pubkey == right.participant_pubkey);
        route_targets.sort();
        route_targets.dedup();

        let local_address = derive_mesh_tunnel_ip(&network_id, &own_pubkey).map_or_else(
            || local_interface_address_for_tunnel(&app.node.tunnel_ip),
            |tunnel_ip| local_interface_address_for_tunnel(&tunnel_ip),
        );

        // WireGuard upstream: when the user has enabled it AND the
        // config is fully populated, expand the tunnel's route set to
        // 0.0.0.0/0 (all outbound traffic should enter the tun) and
        // ask the host platform to keep the WG endpoint outside the
        // tunnel via `excluded_routes`.
        let selected_peer_exit = route_targets.iter().any(|route| route == "0.0.0.0/0");
        let (wireguard_exit, excluded_routes) =
            if app.wireguard_exit.enabled && app.wireguard_exit.configured() {
                let mut excluded = Vec::new();
                if let Some(ip) = wireguard_endpoint_host_ip(&app.wireguard_exit.endpoint) {
                    excluded.push(format!("{ip}/32"));
                }
                if !route_targets.iter().any(|route| route == "0.0.0.0/0") {
                    route_targets.push("0.0.0.0/0".to_string());
                }
                route_targets.sort();
                route_targets.dedup();
                // Force a 25s persistent keepalive on mobile so
                // boringtun keeps its session fresh against Mullvad's
                // server-side timeouts even when the device is idle.
                // Without this, the session goes stale, Mullvad
                // rotates indices on its side, and decap starts
                // returning WrongIndex.
                let mut wg = app.wireguard_exit.clone();
                if wg.persistent_keepalive_secs == 0 {
                    wg.persistent_keepalive_secs = 25;
                }
                (Some(wg), excluded)
            } else {
                (None, Vec::new())
            };
        let local_dns_required = wireguard_exit.is_some()
            || selected_peer_exit
            || !app.magic_dns_suffix.trim().is_empty();
        let (dns_servers, magic_dns_server) = if local_dns_required {
            let server = nostr_vpn_core::MESH_MAGIC_DNS_SERVER.to_string();
            (vec![server.clone()], server)
        } else {
            (Vec::new(), String::new())
        };
        let (pending_join_request_recipient, pending_join_invite_secret, pending_join_requested_at) =
            app.active_network_opt()
                .and_then(|network| {
                    network.outbound_join_request.as_ref().map(|request| {
                        (
                            request.recipient.clone(),
                            network.invite_secret.clone(),
                            request.requested_at,
                        )
                    })
                })
                .unwrap_or_default();

        Ok(Self {
            config_path: config_path.to_string_lossy().to_string(),
            app_config_toml: plaintext_app_config_toml(app)?,
            identity_nsec: app.nostr.secret_key.clone(),
            node_name: app.node_name.trim().to_string(),
            network_id,
            invite_secret: app
                .active_network_opt()
                .map_or_else(String::new, |network| network.invite_secret.clone()),
            local_address,
            advertised_endpoint: app.node.endpoint.trim().to_string(),
            listen_port: app.node.listen_port,
            mtu: DEFAULT_MOBILE_MTU,
            peers,
            peer_hints: mobile_static_peer_hints(app),
            bootstrap_peers: mobile_bootstrap_peer_hints(app),
            route_targets,
            nostr_relays: app.nostr.relays.clone(),
            stun_servers: app.nat.stun_servers.clone(),
            share_local_candidates: app.lan_discovery_enabled,
            connect_to_non_roster_fips_peers: app.connect_to_non_roster_fips_peers,
            nostr_discovery_enabled: app.fips_nostr_discovery_enabled,
            webrtc_enabled: app.fips_webrtc_enabled,
            excluded_routes,
            dns_servers,
            magic_dns_server,
            wireguard_exit,
            join_requests_enabled: app.join_requests_enabled(),
            pending_join_request_recipient,
            pending_join_invite_secret,
            pending_join_requested_at,
            error: String::new(),
        })
    }

    fn redact_for_launch_configuration(&mut self) {
        self.app_config_toml.clear();
        self.identity_nsec.clear();
        self.invite_secret.clear();
        self.pending_join_invite_secret.clear();
        if let Some(wireguard_exit) = self.wireguard_exit.as_mut() {
            wireguard_exit.private_key.clear();
            wireguard_exit.peer_preshared_key.clear();
        }
    }

    fn detach_from_persisted_config_path(&mut self) {
        self.config_path.clear();
    }
}

/// Pull just the IP literal out of an `Endpoint = host:port` string.
/// Returns None if the host is a DNS name (we can't pre-resolve here
/// — the OS-side glue would need to do that). Mullvad / Proton ship
/// configs with literal IPs, so this is fine for the common case.
fn wireguard_endpoint_host_ip(endpoint: &str) -> Option<std::net::IpAddr> {
    let trimmed = endpoint.trim();
    let host = trimmed.rsplit_once(':').map_or(trimmed, |(h, _)| h);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse().ok()
}

fn mobile_app_config(config: &MobileTunnelConfig) -> Result<AppConfig> {
    if !config.app_config_toml.trim().is_empty() {
        let mut app: AppConfig =
            toml::from_str(&config.app_config_toml).context("failed to parse mobile app config")?;
        app.ensure_defaults();
        return Ok(app);
    }

    let config_path = non_empty_path(&config.config_path)
        .ok_or_else(|| anyhow!("mobile app config unavailable"))?;
    AppConfig::migrate_persisted_secrets(&config_path)?;
    let mut app = AppConfig::load(&config_path)?;
    app.ensure_defaults();
    Ok(app)
}

fn plaintext_app_config_toml(app: &AppConfig) -> Result<String> {
    app.plaintext_toml()
        .context("failed to encode mobile app config TOML")
}

fn persisted_app_config_toml(app: &AppConfig, config_path: &Path) -> Result<String> {
    if config_path.as_os_str().is_empty() {
        return plaintext_app_config_toml(app);
    }
    app.persisted_toml_for_path(config_path)
        .context("failed to encode mobile app config TOML")
}

pub(crate) fn tunnel_config_json(data_dir: &str) -> String {
    let mut config =
        MobileTunnelConfig::from_data_dir(data_dir).unwrap_or_else(|error| MobileTunnelConfig {
            error: error.to_string(),
            ..empty_config()
        });
    config.redact_for_launch_configuration();
    serde_json::to_string(&config).unwrap_or_else(|error| {
        format!(
            r#"{{"error":"{}"}}"#,
            error.to_string().replace(['\\', '"'], "")
        )
    })
}

pub(crate) fn tunnel_provider_options_config_json(data_dir: &str) -> String {
    let mut config =
        MobileTunnelConfig::from_data_dir(data_dir).unwrap_or_else(|error| MobileTunnelConfig {
            error: error.to_string(),
            ..empty_config()
        });
    config.detach_from_persisted_config_path();
    serde_json::to_string(&config).unwrap_or_else(|error| {
        format!(
            r#"{{"error":"{}"}}"#,
            error.to_string().replace(['\\', '"'], "")
        )
    })
}

type MobileMesh = Arc<RwLock<Arc<FipsMeshRuntime>>>;

fn new_mobile_mesh(runtime: FipsMeshRuntime) -> MobileMesh {
    Arc::new(RwLock::new(Arc::new(runtime)))
}

fn mobile_mesh_snapshot(mesh: &MobileMesh) -> Result<Arc<FipsMeshRuntime>> {
    mesh.read()
        .map(|mesh| Arc::clone(&*mesh))
        .map_err(|_| anyhow!("mobile FIPS mesh route table lock poisoned"))
}

fn replace_mobile_mesh(mesh: &MobileMesh, mut runtime: FipsMeshRuntime) -> Result<()> {
    let mut mesh = mesh
        .write()
        .map_err(|_| anyhow!("mobile FIPS mesh route table lock poisoned"))?;
    runtime.inherit_exit_flows(&mesh);
    *mesh = Arc::new(runtime);
    Ok(())
}

pub(crate) struct MobileTunnel {
    runtime: Runtime,
    endpoint: Option<Arc<FipsEndpoint>>,
    mesh: MobileMesh,
    presence: Arc<RwLock<HashMap<String, MobilePeerPresence>>>,
    config: Arc<RwLock<MobileTunnelConfig>>,
    app_config: Arc<RwLock<AppConfig>>,
    app_config_dirty: Arc<AtomicBool>,
    tun_counters: Arc<MobileTunAtomicCounters>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    outbound_tx: tokio_mpsc::Sender<Vec<Vec<u8>>>,
    inbound_rx: Option<tokio_mpsc::Receiver<Vec<Vec<u8>>>>,
    tasks: Vec<JoinHandle<()>>,
    wg_upstream: Option<WgUpstreamRuntime>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    native_tun: Option<NativeTunRuntime>,
    /// Raw fd of the boringtun UDP socket. Android reads this and calls
    /// `VpnService.protect(fd)` so encrypted UDP escapes the VPN tun.
    #[cfg(target_os = "android")]
    wg_upstream_socket_fd: c_int,
}
