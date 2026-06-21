#[cfg(all(
    not(target_os = "linux"),
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn is_default_exit_node_route(route: &str) -> bool {
    matches!(route, "0.0.0.0/0" | "::/0")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonPidRecord {
    pid: u32,
    config_path: String,
    started_at: u64,
}

#[derive(Debug, Clone)]
struct DaemonStatus {
    running: bool,
    pid: Option<u32>,
    pid_file: PathBuf,
    log_file: PathBuf,
    state_file: PathBuf,
    state: Option<DaemonRuntimeState>,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceStatusView {
    supported: bool,
    installed: bool,
    disabled: bool,
    loaded: bool,
    running: bool,
    pid: Option<u32>,
    label: String,
    plist_path: String,
    #[serde(default)]
    binary_path: String,
    #[serde(default)]
    binary_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionInfoView {
    version: String,
    #[serde(default)]
    fips_core_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DaemonRuntimeState {
    updated_at: u64,
    #[serde(default)]
    binary_version: String,
    #[serde(default)]
    fips_core_version: String,
    #[serde(default)]
    local_endpoint: String,
    #[serde(default)]
    advertised_endpoint: String,
    #[serde(default)]
    listen_port: u16,
    vpn_enabled: bool,
    vpn_active: bool,
    vpn_status: String,
    expected_peer_count: usize,
    connected_peer_count: usize,
    #[serde(default)]
    fips_direct_roster_peer_count: usize,
    #[serde(default)]
    fips_other_peer_count: usize,
    mesh_ready: bool,
    #[serde(default)]
    health: Vec<HealthIssue>,
    #[serde(default)]
    network: NetworkSummary,
    #[serde(default)]
    port_mapping: PortMappingStatus,
    #[serde(default)]
    relays: Vec<DaemonRelayState>,
    #[serde(default)]
    peers: Vec<DaemonPeerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct DaemonRelayState {
    url: String,
    status: String,
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct MacosNetworkCleanupState {
    #[serde(default)]
    iface: String,
    #[serde(default)]
    endpoint_bypass_routes: Vec<String>,
    #[serde(default)]
    managed_routes: Vec<MacosManagedRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_default_route: Option<MacosRouteSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ipv4_forward_was_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pf_was_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonPeerState {
    participant_pubkey: String,
    node_id: String,
    tunnel_ip: String,
    endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fips_endpoint_npub: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fips_transport_addr: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fips_transport_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fips_srtt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fips_srtt_age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    fips_packets_sent: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    fips_packets_recv: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    fips_bytes_sent: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    fips_bytes_recv: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    fips_rekey_in_progress: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    fips_rekey_draining: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fips_current_k_bit: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fips_last_outbound_route: String,
    #[serde(default, skip_serializing_if = "is_false")]
    direct_probe_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direct_probe_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    direct_probe_retry_count: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    direct_probe_auto_reconnect: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    direct_probe_expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    fips_nostr_traversal_failures: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    fips_nostr_traversal_in_cooldown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fips_nostr_traversal_cooldown_until_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fips_nostr_traversal_last_observed_skew_ms: Option<i64>,
    #[serde(default)]
    tx_bytes: u64,
    #[serde(default)]
    rx_bytes: u64,
    public_key: String,
    advertised_routes: Vec<String>,
    last_mesh_seen_at: u64,
    last_fips_seen_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_fips_control_seen_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_fips_data_seen_at: Option<u64>,
    reachable: bool,
    last_handshake_at: Option<u64>,
    error: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableFingerprint {
    len: u64,
    modified_unix_nanos: Option<u128>,
}

struct DaemonReloadConfig {
    app: AppConfig,
    network_id: String,
    expected_peers: usize,
    own_pubkey: Option<String>,
}

fn load_config_with_overrides(
    path: &Path,
    network_id: Option<String>,
    participants: Vec<String>,
) -> Result<(AppConfig, String)> {
    let mut app = load_or_default_config(path)?;
    apply_participants_override(&mut app, participants)?;
    if let Some(network_id) = network_id {
        app.set_active_network_id(&network_id)?;
    }
    maybe_autoconfigure_node(&mut app);

    let network_id = app.effective_network_id();
    Ok((app, network_id))
}

fn build_daemon_reload_config(app: AppConfig, network_id: String) -> DaemonReloadConfig {
    let expected_peers = expected_peer_count(&app);
    let own_pubkey = app.own_nostr_pubkey_hex().ok();

    DaemonReloadConfig {
        app,
        network_id,
        expected_peers,
        own_pubkey,
    }
}

struct ConnectMagicDnsRuntime {
    suffix: String,
    resolver_installed: bool,
    server: MagicDnsServer,
}

impl ConnectMagicDnsRuntime {
    fn start(app: &AppConfig) -> Option<Self> {
        let records = build_magic_dns_records(app);
        if records.is_empty() {
            println!("magicdns: skipped (no configured alias records)");
            return None;
        }

        let server = match MagicDnsServer::start(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, MAGIC_DNS_PORT)),
            records.clone(),
        ) {
            Ok(server) => server,
            Err(error) => {
                eprintln!(
                    "magicdns: preferred port {MAGIC_DNS_PORT} unavailable ({error}); trying random local port"
                );
                match MagicDnsServer::start(
                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                    records.clone(),
                ) {
                    Ok(server) => server,
                    Err(error) => {
                        eprintln!("magicdns: failed to start local dns server: {error}");
                        return None;
                    }
                }
            }
        };
        let local_addr = server.local_addr();

        let suffix = app
            .magic_dns_suffix
            .trim()
            .trim_matches('.')
            .to_ascii_lowercase();
        if suffix.is_empty() {
            println!(
                "magicdns: local dns running on {local_addr} (system split-dns disabled; empty suffix)"
            );
            return Some(Self {
                suffix,
                resolver_installed: false,
                server,
            });
        }

        let nameserver = match local_addr {
            SocketAddr::V4(v4) => *v4.ip(),
            SocketAddr::V6(_) => {
                eprintln!("magicdns: local dns unexpectedly bound to IPv6; split-dns disabled");
                return Some(Self {
                    suffix,
                    resolver_installed: false,
                    server,
                });
            }
        };

        let resolver_config = MagicDnsResolverConfig {
            suffix: suffix.clone(),
            nameserver,
            port: local_addr.port(),
            records,
        };

        match install_system_resolver(&resolver_config) {
            Ok(()) => {
                println!(
                    "magicdns: active for .{} via {}:{}",
                    suffix, resolver_config.nameserver, resolver_config.port
                );
                Some(Self {
                    suffix,
                    resolver_installed: true,
                    server,
                })
            }
            Err(error) => {
                eprintln!(
                    "magicdns: system resolver install failed ({error}); local dns remains on {local_addr}"
                );
                Some(Self {
                    suffix,
                    resolver_installed: false,
                    server,
                })
            }
        }
    }

    /// Rebuild MagicDNS records from the current `AppConfig` and push them
    /// to the in-process responder (and the `/etc/hosts` fallback block on
    /// Linux if it's active). Without this, the records map is frozen at
    /// daemon-start time and any peer added later (via `add-participant`,
    /// invite acceptance, FIPS roster event, or peer-alias rename) returns
    /// NXDOMAIN until the daemon is restarted.
    fn refresh_records(&self, app: &AppConfig) {
        let records = build_magic_dns_records(app);
        self.server.update_records(records.clone());
        #[cfg(target_os = "linux")]
        if !self.suffix.is_empty()
            && let Err(error) = nostr_vpn_core::magic_dns::refresh_linux_hosts_fallback_if_active(
                &self.suffix,
                &records,
            )
        {
            eprintln!("magicdns: failed to refresh hosts fallback: {error}");
        }
        let _ = records; // suppress unused-binding on non-Linux
    }
}

impl Drop for ConnectMagicDnsRuntime {
    fn drop(&mut self) {
        if self.resolver_installed
            && !self.suffix.is_empty()
            && let Err(error) = uninstall_system_resolver(&self.suffix)
        {
            eprintln!(
                "magicdns: failed to remove system resolver for .{}: {error}",
                self.suffix
            );
        }

        self.server.stop();
    }
}

struct CliTunnelRuntime {
    iface: String,
    active_listen_port: Option<u16>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Default)]
struct LinuxExitNodeRuntime {
    ipv4_outbound_iface: Option<String>,
    ipv6_outbound_iface: Option<String>,
    ipv4_tunnel_source_cidr: Option<String>,
    ipv4_mss_clamp: Option<u16>,
    ipv4_forward_was_enabled: Option<bool>,
    ipv6_forward_was_enabled: Option<bool>,
    wireguard_exit: Option<LinuxWireGuardExitRuntime>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Default)]
struct MacosExitNodeRuntime {
    outbound_iface: Option<String>,
    tunnel_source_cidr: Option<String>,
    pf_was_enabled: Option<bool>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxWireGuardExitRuntime {
    interface: String,
    source_cidr: String,
    table: u32,
    priority: u32,
    created_interface: bool,
    endpoint_bypass_routes: Vec<String>,
    previous_default_route: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct MacosRouteSpec {
    gateway: Option<String>,
    interface: String,
}

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
struct MacosManagedRoute {
    target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interface: Option<String>,
}
