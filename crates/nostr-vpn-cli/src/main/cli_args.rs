#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonControlRequest {
    Stop,
    Reload,
    Pause,
    Resume,
}

impl DaemonControlRequest {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stop => DAEMON_CONTROL_STOP_REQUEST,
            Self::Reload => DAEMON_CONTROL_RELOAD_REQUEST,
            Self::Pause => DAEMON_CONTROL_PAUSE_REQUEST,
            Self::Resume => DAEMON_CONTROL_RESUME_REQUEST,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            DAEMON_CONTROL_STOP_REQUEST => Some(Self::Stop),
            DAEMON_CONTROL_RELOAD_REQUEST => Some(Self::Reload),
            DAEMON_CONTROL_PAUSE_REQUEST => Some(Self::Pause),
            DAEMON_CONTROL_RESUME_REQUEST => Some(Self::Resume),
            _ => None,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "nvpn")]
#[command(version)]
#[command(about = "FIPS private mesh VPN")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Initialize a local config file (keys are generated automatically).
    Init {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        /// Device Nostr pubkeys (npub or hex) that define the network.
        #[arg(long = "device", alias = "participant")]
        devices: Vec<String>,
    },
    /// Show the running CLI version.
    Version(VersionArgs),
    /// Update this `nvpn` binary from the latest published release.
    Update(UpdateArgs),
    /// Install `nvpn` into a platform-appropriate default PATH location.
    InstallCli(InstallCliArgs),
    /// Remove an `nvpn` binary previously installed into PATH.
    UninstallCli(UninstallCliArgs),
    /// Manage the persistent system daemon service.
    Service(ServiceArgs),
    /// Start a session (foreground by default, or daemonized with --daemon).
    Start(StartArgs),
    /// Stop a background daemon started by `nvpn start --daemon`.
    Stop(StopArgs),
    /// Repair local network state left behind by a stopped or crashed VPN session.
    RepairNetwork(RepairNetworkArgs),
    /// Ask the running daemon to reload config and peer set.
    Reload(ReloadArgs),
    /// Pause VPN networking while keeping daemon running.
    Pause(ControlArgs),
    /// Resume VPN networking on a running daemon.
    Resume(ControlArgs),
    /// Run a FIPS private mesh session from config.
    Connect(ConnectArgs),
    /// Show local and discovered peer status.
    Status(StatusArgs),
    /// Update persisted node/network settings.
    Set(SetArgs),
    /// Emit a full `nvpn://invite/...` code for the active network.
    CreateInvite(CreateInviteArgs),
    /// Import a full `nvpn://invite/...` code into the active network config.
    ImportInvite(ImportInviteArgs),
    /// Broadcast the active network's invite over LAN multicast so nearby
    /// devices running `nvpn discover` can pair without copy/pasting a code.
    /// Runs in the foreground; Ctrl-C stops broadcasting.
    InviteBroadcast(InviteBroadcastArgs),
    /// Listen for nearby LAN invite broadcasts and print what's found. With
    /// `--accept`, import the first valid invite seen (queues a join request
    /// to the broadcaster, same as `nvpn import-invite`).
    Discover(DiscoverArgs),
    /// Add one or more devices to the active network roster.
    #[command(alias = "add-participant")]
    AddDevice(UpdateRosterArgs),
    /// Remove one or more devices from the active network roster.
    #[command(alias = "remove-participant")]
    RemoveDevice(UpdateRosterArgs),
    /// Add one or more admins to the active network roster.
    AddAdmin(UpdateRosterArgs),
    /// Remove one or more admins from the active network roster.
    RemoveAdmin(UpdateRosterArgs),
    /// Ping a peer by node ID or tunnel IP.
    Ping(PingArgs),
    /// Diagnose runtime/network issues and optionally write a support bundle.
    Doctor(DoctorArgs),
    /// Show local or peer tunnel IPs.
    Ip(IpArgs),
    /// Resolve a node/tunnel IP to peer metadata.
    Whois(WhoisArgs),
    /// Probe a WireGuard upstream config (Mullvad/Proton-style) by running
    /// the userspace WG state machine against it and reporting whether
    /// the handshake completes. Without --replace-default or --scoped-host,
    /// this does not create a tun device and does not modify routes.
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    WgUpstreamTest(WgUpstreamTestArgs),
    /// Manage Cashu-paid public exit routing.
    #[cfg(feature = "paid-exit")]
    #[command(name = "paid-exit")]
    PaidExit(PaidExitArgs),
    /// Internal config import helper for elevated GUI writes.
    #[command(hide = true)]
    ApplyConfig(ApplyConfigArgs),
    /// Internal daemon-backed config import helper for GUI writes.
    #[command(hide = true)]
    ApplyConfigDaemon(ApplyConfigArgs),
    /// Internal daemon entrypoint. Use `nvpn start --daemon`.
    #[command(hide = true)]
    Daemon(DaemonArgs),
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Args)]
struct WgUpstreamTestArgs {
    /// Path to a WireGuard config file (the same `[Interface]` /
    /// `[Peer]` syntax wg-quick / Mullvad / Proton VPN export).
    #[arg(long, required_unless_present = "self_test")]
    config_file: Option<PathBuf>,
    /// Generate a local paired WireGuard responder and test against it.
    /// This is for release-gate host/VM checks: it uses the same native
    /// tun/Wintun path as a provider config, but needs no external VPN account.
    #[arg(long, default_value_t = false)]
    self_test: bool,
    /// Maximum time to wait for the WG handshake to complete.
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
    /// If set, in addition to the handshake probe, bring up a userspace
    /// tun device, install a single host route to this IP via the tun
    /// (default route is **not** modified — the rest of the host's
    /// internet stays alive), wait for the WG handshake, ping the host
    /// through the tunnel, then tear everything back down. Requires
    /// root / sudo because it touches the tun and the routing table.
    #[arg(long, conflicts_with = "replace_default")]
    scoped_host: Option<std::net::IpAddr>,
    /// **DANGEROUS:** route ALL outbound traffic through the WG tunnel
    /// (Mullvad/Proton-style). Brings up the tun, runs the WG handshake
    /// FIRST, and only swaps the default route once we've confirmed the
    /// tunnel is live (so a misconfigured config can never take the
    /// host offline). The original default route + the WG-endpoint
    /// bypass are restored on Drop, even on panic / Ctrl-C. Requires
    /// root / sudo.
    #[arg(long, default_value_t = false)]
    replace_default: bool,
    /// Optional IP to ping through the tunnel for confidence after the
    /// handshake completes. Used by both `--scoped-host` (where it
    /// defaults to the scoped IP) and `--replace-default` (where it's
    /// any externally-reachable host, e.g. 1.1.1.1). When empty in
    /// `--replace-default` mode the command just holds the tunnel up
    /// for `--hold-secs` and then tears it down.
    #[arg(long)]
    probe_target: Option<std::net::IpAddr>,
    /// Number of pings to send to the probe target after the handshake.
    /// Ignored when neither `--scoped-host` nor a `--probe-target` is
    /// set.
    #[arg(long, default_value_t = 5)]
    ping_count: u8,
    /// After the data plane test completes, hold the tunnel up for
    /// this many seconds before tearing it down (lets you inspect
    /// routes / tcpdump from another shell).
    #[arg(long, default_value_t = 0)]
    hold_secs: u64,
    /// Override the tun device name. macOS picks utunN automatically
    /// when this is empty; Linux picks `nvpn-wg-test`.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[arg(long)]
    tun_name: Option<String>,
}

#[derive(Debug, Args)]
struct InstallCliArgs {
    /// Destination path for the installed executable.
    #[arg(long)]
    path: Option<PathBuf>,
    /// Overwrite destination if it already exists.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct VersionArgs {
    #[arg(long)]
    json: bool,
    /// Print component build details in text output.
    #[arg(long)]
    verbose: bool,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Only check whether an update is available.
    #[arg(long)]
    check: bool,
    /// Select the native desktop app artifact instead of the nvpn CLI archive.
    #[arg(long)]
    app: bool,
    /// Download the selected artifact and print/save it, without installing it.
    #[arg(long)]
    download_only: bool,
    /// Directory for --download-only artifacts.
    #[arg(long)]
    download_dir: Option<PathBuf>,
    /// Emit machine-readable JSON for GUI update helpers.
    #[arg(long)]
    json: bool,
    /// Destination binary to update (defaults to the currently running executable).
    #[arg(long)]
    path: Option<PathBuf>,
    /// Install even when the latest release is not newer than this binary.
    #[arg(long)]
    force: bool,
    /// Release manifest source to query.
    #[arg(long, value_enum, default_value = "auto")]
    source: UpdateSource,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum UpdateSource {
    Auto,
    Github,
    #[value(alias = "htree")]
    Hashtree,
}

#[derive(Debug, Args)]
struct UninstallCliArgs {
    /// Path to remove (defaults to the platform-appropriate install path).
    #[arg(long)]
    path: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Install and start the system service.
    Install(ServiceInstallArgs),
    /// Enable and start an installed system service.
    Enable(ServiceControlArgs),
    /// Stop and disable an installed system service.
    Disable(ServiceControlArgs),
    /// Remove the system service.
    Uninstall(ServiceUninstallArgs),
    /// Show service install/runtime status.
    Status(ServiceStatusArgs),
}

#[derive(Debug, Args)]
struct ServiceInstallArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = default_tunnel_iface())]
    iface: String,
    #[arg(long, alias = "announce-interval-secs", default_value_t = 20)]
    mesh_refresh_interval_secs: u64,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct ServiceUninstallArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ServiceStatusArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long, hide = true)]
    skip_binary_version: bool,
}

#[derive(Debug, Args)]
struct ServiceControlArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ConnectArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, default_value_t = default_tunnel_iface())]
    iface: String,
    #[arg(long, alias = "announce-interval-secs", default_value_t = 20)]
    mesh_refresh_interval_secs: u64,
}

#[derive(Debug, Args, Clone)]
struct DaemonArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, default_value_t = default_tunnel_iface())]
    iface: String,
    #[arg(long, alias = "announce-interval-secs", default_value_t = 20)]
    mesh_refresh_interval_secs: u64,
    #[arg(long, hide = true, default_value_t = false)]
    paused: bool,
    #[arg(long, hide = true, default_value_t = false)]
    service: bool,
}

#[derive(Debug, Args)]
struct StartArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, default_value_t = default_tunnel_iface())]
    iface: String,
    #[arg(long, alias = "announce-interval-secs", default_value_t = 20)]
    mesh_refresh_interval_secs: u64,
    #[arg(long)]
    daemon: bool,
    #[arg(long, conflicts_with = "no_connect")]
    connect: bool,
    #[arg(long, conflicts_with = "connect")]
    no_connect: bool,
}

#[derive(Debug, Args)]
struct StopArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    timeout_secs: u64,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct RepairNetworkArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ReloadArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ControlArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, hide = true, default_value_t = 2)]
    discover_secs: u64,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct SetArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long)]
    node_name: Option<String>,
    #[arg(long)]
    node_id: Option<String>,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    tunnel_ip: Option<String>,
    #[arg(long)]
    listen_port: Option<u16>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long)]
    exit_node: Option<String>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    exit_node_leak_protection: Option<bool>,
    #[arg(long)]
    advertise_routes: Option<String>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    advertise_exit_node: Option<bool>,
    #[cfg(feature = "paid-exit")]
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    paid_exit_enabled: Option<bool>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_meter: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_upstream: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_price_msat: Option<u64>,
    #[cfg(feature = "paid-exit")]
    #[arg(long, value_name = "UNITS")]
    paid_exit_per_units: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_accepted_mints: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_country_code: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_region: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_asn: Option<u32>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_network_class: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_ipv4: Option<bool>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_ipv6: Option<bool>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_max_channel_capacity_sat: Option<u64>,
    #[cfg(feature = "paid-exit")]
    #[arg(long)]
    paid_exit_channel_expiry_secs: Option<u64>,
    #[cfg(feature = "paid-exit")]
    #[arg(long, value_name = "UNITS")]
    paid_exit_free_probe_units: Option<String>,
    #[cfg(feature = "paid-exit")]
    #[arg(long, value_name = "UNITS")]
    paid_exit_grace_units: Option<String>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    wireguard_exit_enabled: Option<bool>,
    #[arg(long)]
    wireguard_exit_interface: Option<String>,
    #[arg(long)]
    wireguard_exit_address: Option<String>,
    #[arg(long)]
    wireguard_exit_private_key: Option<String>,
    #[arg(long)]
    wireguard_exit_peer_public_key: Option<String>,
    #[arg(long)]
    wireguard_exit_peer_preshared_key: Option<String>,
    #[arg(long)]
    wireguard_exit_endpoint: Option<String>,
    #[arg(long)]
    wireguard_exit_allowed_ips: Option<String>,
    #[arg(long)]
    wireguard_exit_dns: Option<String>,
    #[arg(long)]
    wireguard_exit_mtu: Option<u16>,
    #[arg(long)]
    wireguard_exit_keepalive: Option<u16>,
    #[arg(long)]
    wireguard_exit_config: Option<String>,
    #[arg(long)]
    wireguard_exit_config_file: Option<PathBuf>,
    #[arg(long)]
    autoconnect: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    join_requests_enabled: Option<bool>,
    #[arg(
        long = "fips-advertise-public-endpoint",
        alias = "fips-advertise-endpoint",
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    fips_advertise_public_endpoint: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    fips_host_tunnel_enabled: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    connect_to_non_roster_fips_peers: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    fips_nostr_discovery_enabled: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    fips_bootstrap_enabled: Option<bool>,
    #[arg(long)]
    fips_host_inbound_tcp_ports: Option<String>,
    #[arg(long = "fips-peer-endpoint")]
    fips_peer_endpoints: Vec<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct CreateInviteArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ImportInviteArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    invite: String,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct InviteBroadcastArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// How long to keep broadcasting before exiting. Defaults to 15 minutes,
    /// matching the GUI's broadcast window.
    #[arg(long, value_name = "SECS")]
    duration_secs: Option<u64>,
}

#[derive(Debug, Args)]
struct DiscoverArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Stop listening after this many seconds (default: keep listening until
    /// Ctrl-C, or until the first invite when `--accept` is set).
    #[arg(long, value_name = "SECS")]
    duration_secs: Option<u64>,
    /// Import the first valid invite seen and exit. Equivalent to piping the
    /// discovered code through `nvpn import-invite`.
    #[arg(long)]
    accept: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct UpdateRosterArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant", required = true)]
    devices: Vec<String>,
    #[arg(long)]
    publish: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PingArgs {
    target: String,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, hide = true, default_value_t = 2)]
    discover_secs: u64,
    #[arg(long, default_value_t = 3)]
    count: u32,
    #[arg(long, default_value_t = 2)]
    timeout_secs: u64,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, default_value_t = 4)]
    timeout_secs: u64,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    write_bundle: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct IpArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, hide = true, default_value_t = 2)]
    discover_secs: u64,
    #[arg(long)]
    peer: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct WhoisArgs {
    query: String,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    network_id: Option<String>,
    #[arg(long = "device", alias = "participant")]
    devices: Vec<String>,
    #[arg(long, hide = true, default_value_t = 2)]
    discover_secs: u64,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ApplyConfigArgs {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    config: Option<PathBuf>,
}
