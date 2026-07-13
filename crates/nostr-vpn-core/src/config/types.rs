pub const DEFAULT_RELAYS: &[&str] = &[];

/// No identity is privileged as a built-in FIPS gateway. New configs discover
/// peers over Nostr; operators can still add explicit bootstrap/transit peers.
pub const DEFAULT_FIPS_BOOTSTRAP_PEERS: &[(&str, &[&str])] = &[];

/// The default bootstrap peer list as an owned map, used to seed configs and to
/// power "reset to defaults".
pub fn default_fips_bootstrap_peers() -> HashMap<String, Vec<String>> {
    DEFAULT_FIPS_BOOTSTRAP_PEERS
        .iter()
        .map(|(npub, addrs)| {
            (
                (*npub).to_string(),
                addrs.iter().map(|addr| (*addr).to_string()).collect(),
            )
        })
        .collect()
}

/// Split a transport-tagged peer address into `(transport, address)`. A bare
/// `host:port` defaults to UDP. Used to lower bootstrap/transit address strings
/// and direct WebRTC peer IDs into fips `PeerAddress` values.
pub fn split_peer_transport_addr(value: &str) -> (String, String) {
    let value = value.trim();
    for transport in ["udp", "tcp", "tor", "webrtc"] {
        if let Some(rest) = value.strip_prefix(&format!("{transport}:")) {
            return (transport.to_string(), rest.trim().to_string());
        }
    }
    ("udp".to_string(), value.to_string())
}

pub fn normalize_fips_peer_endpoint_hint(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    if let Some(addr) = peer_endpoint_hint_addr(&PeerEndpointHint::udp(endpoint)) {
        return Some(addr);
    }

    let default_port = default_listen_port();
    let endpoint = if let Some(host) = endpoint
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let host = host.trim();
        if host.is_empty() || !host.contains(':') {
            return None;
        }
        format!("[{host}]:{default_port}")
    } else if endpoint.contains(':') {
        return None;
    } else {
        format!("{endpoint}:{default_port}")
    };
    peer_endpoint_hint_addr(&PeerEndpointHint::udp(endpoint))
}

pub fn normalize_relay_urls(values: Vec<String>) -> Vec<String> {
    let mut relays = values
        .into_iter()
        .flat_map(|value| {
            value
                .split([',', '\n', '\r', ' ', '\t'])
                .map(str::trim)
                .filter(|relay| !relay.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    relays.sort();
    relays.dedup();
    relays
}

include!("types/nostr.rs");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
    #[serde(default = "default_node_name")]
    pub node_name: String,
    #[serde(default = "default_lan_discovery_enabled", skip_serializing)]
    pub lan_discovery_enabled: bool,
    #[serde(default = "default_launch_on_startup")]
    pub launch_on_startup: bool,
    #[serde(default = "default_autoconnect")]
    pub autoconnect: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fips_peer_endpoints: HashMap<String, Vec<String>>,
    /// Publish this node's exact UDP endpoint through public FIPS/Nostr discovery.
    /// Roster peers still receive private endpoint hints over FIPS capabilities.
    #[serde(
        default = "default_fips_advertise_public_endpoint",
        alias = "fips_advertise_endpoint",
        skip_serializing_if = "is_false"
    )]
    pub fips_advertise_public_endpoint: bool,
    #[serde(
        default = "default_fips_host_tunnel_enabled",
        skip_serializing_if = "is_false"
    )]
    pub fips_host_tunnel_enabled: bool,
    #[serde(
        default = "default_connect_to_non_roster_fips_peers",
        skip_serializing_if = "is_false"
    )]
    pub connect_to_non_roster_fips_peers: bool,
    /// Find/advertise FIPS peers over Nostr relays. When false, the node still
    /// connects to LAN, static, and bootstrap peers but does not use relays for
    /// endpoint discovery or advertising.
    #[serde(
        default = "default_fips_nostr_discovery_enabled",
        skip_serializing_if = "is_true"
    )]
    pub fips_nostr_discovery_enabled: bool,
    /// Enable the browser-compatible FIPS WebRTC transport. Nostr discovery
    /// remains available to UDP/TCP transports when this is false.
    #[serde(
        default = "default_fips_webrtc_enabled",
        skip_serializing_if = "is_false"
    )]
    pub fips_webrtc_enabled: bool,
    /// Master switch for dialing the bootstrap/transit peer list below. When off,
    /// the list is kept but not dialed.
    #[serde(
        default = "default_fips_bootstrap_enabled",
        skip_serializing_if = "is_false"
    )]
    pub fips_bootstrap_enabled: bool,
    /// Editable operator-supplied transit/bootstrap peers (npub ->
    /// transport-tagged addresses). New configs leave this empty so Nostr
    /// discovery, rather than a privileged identity, chooses peers.
    #[serde(default = "default_fips_bootstrap_peers")]
    pub fips_bootstrap_peers: HashMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fips_host_inbound_tcp_ports: Vec<u16>,
    #[serde(default, skip_serializing)]
    pub mesh_mtu_profile: String,
    #[serde(default, skip_serializing)]
    pub mesh_underlay_udp_mtu: u16,
    #[serde(default, skip_serializing)]
    pub mesh_tunnel_mtu: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub exit_node: String,
    #[serde(default, skip_serializing_if = "InternetSource::is_direct")]
    pub internet_source: InternetSource,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exit_node_public_paid_exit: bool,
    #[serde(
        default = "default_exit_node_leak_protection",
        skip_serializing_if = "is_true"
    )]
    pub exit_node_leak_protection: bool,
    #[serde(default = "default_close_to_tray_on_close")]
    pub close_to_tray_on_close: bool,
    #[serde(default = "default_magic_dns_suffix", skip)]
    pub magic_dns_suffix: String,
    #[serde(default, skip_serializing_if = "WireGuardExitConfig::is_default")]
    pub wireguard_exit: WireGuardExitConfig,
    #[serde(default, skip_serializing_if = "PaidExitConfig::is_default")]
    pub paid_exit: PaidExitConfig,
    #[serde(
        default = "default_wallet_fiat_enabled",
        skip_serializing_if = "is_true"
    )]
    pub wallet_fiat_enabled: bool,
    #[serde(default, skip_serializing_if = "FiatCurrency::is_usd")]
    pub wallet_fiat_currency: FiatCurrency,
    #[serde(default = "default_peer_aliases")]
    pub peer_aliases: HashMap<String, String>,
    #[serde(default)]
    pub nat: NatConfig,
    #[serde(default)]
    pub nostr: NostrConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_nostr_join_request: Option<PendingNostrJoinRequest>,
    #[serde(default)]
    pub node: NodeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InternetSource {
    #[default]
    Direct,
    PrivateVpn,
    PaidAutomatic,
    PaidManual,
    WireGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum FiatCurrency {
    #[default]
    Usd,
    Eur,
    Gbp,
    Cad,
    Aud,
    Jpy,
    Chf,
}

impl FiatCurrency {
    pub fn is_usd(&self) -> bool {
        *self == Self::Usd
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Cad => "CAD",
            Self::Aud => "AUD",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
        }
    }
}

impl std::str::FromStr for FiatCurrency {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "USD" => Ok(Self::Usd),
            "EUR" => Ok(Self::Eur),
            "GBP" => Ok(Self::Gbp),
            "CAD" => Ok(Self::Cad),
            "AUD" => Ok(Self::Aud),
            "JPY" => Ok(Self::Jpy),
            "CHF" => Ok(Self::Chf),
            _ => Err("expected one of: USD, EUR, GBP, CAD, AUD, JPY, CHF"),
        }
    }
}

impl InternetSource {
    pub fn is_direct(&self) -> bool {
        *self == Self::Direct
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::PrivateVpn => "private_vpn",
            Self::PaidAutomatic => "paid_automatic",
            Self::PaidManual => "paid_manual",
            Self::WireGuard => "wireguard",
        }
    }
}

impl std::str::FromStr for InternetSource {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "direct" | "this_device" | "local" | "off" => Ok(Self::Direct),
            "private_vpn" | "private" | "peer" => Ok(Self::PrivateVpn),
            "paid_automatic" | "paid_auto" | "automatic" | "auto" => Ok(Self::PaidAutomatic),
            "paid_manual" | "manual" | "paid" => Ok(Self::PaidManual),
            "wireguard" | "wg" => Ok(Self::WireGuard),
            _ => {
                Err("expected one of: direct, private_vpn, paid_automatic, paid_manual, wireguard")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WireGuardExitConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(
        default = "default_wireguard_exit_interface",
        skip_serializing_if = "wireguard_exit_interface_is_default"
    )]
    pub interface: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub address: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub peer_public_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub peer_preshared_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    #[serde(
        default = "default_wireguard_exit_allowed_ips",
        skip_serializing_if = "wireguard_exit_allowed_ips_is_default"
    )]
    pub allowed_ips: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    #[serde(
        default = "default_wireguard_exit_mtu",
        skip_serializing_if = "wireguard_exit_mtu_is_default"
    )]
    pub mtu: u16,
    #[serde(
        default = "default_wireguard_exit_persistent_keepalive_secs",
        skip_serializing_if = "wireguard_exit_persistent_keepalive_secs_is_default"
    )]
    pub persistent_keepalive_secs: u16,
}

impl Default for WireGuardExitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface: default_wireguard_exit_interface(),
            address: String::new(),
            private_key: String::new(),
            peer_public_key: String::new(),
            peer_preshared_key: String::new(),
            endpoint: String::new(),
            allowed_ips: default_wireguard_exit_allowed_ips(),
            dns: Vec::new(),
            mtu: default_wireguard_exit_mtu(),
            persistent_keepalive_secs: default_wireguard_exit_persistent_keepalive_secs(),
        }
    }
}

impl WireGuardExitConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn configured(&self) -> bool {
        !self.address.trim().is_empty()
            && !self.private_key.trim().is_empty()
            && !self.peer_public_key.trim().is_empty()
            && !self.endpoint.trim().is_empty()
    }

    pub fn dns_server_ips(&self) -> Vec<IpAddr> {
        let mut servers = self
            .dns
            .iter()
            .filter_map(|server| server.trim().parse::<IpAddr>().ok())
            .collect::<Vec<_>>();
        servers.sort_unstable();
        servers.dedup();
        servers
    }
}

pub fn parse_wireguard_exit_config(raw: &str) -> Result<WireGuardExitConfig> {
    let mut config = WireGuardExitConfig {
        allowed_ips: Vec::new(),
        ..WireGuardExitConfig::default()
    };
    let mut section = WireGuardConfigSection::None;
    let mut saw_interface = false;
    let mut saw_peer = false;
    let mut addresses = Vec::new();

    for (line_index, raw_line) in raw.lines().enumerate() {
        let line_no = line_index + 1;
        let line = strip_wireguard_config_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(section_name) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            let section_name = section_name.trim().to_ascii_lowercase();
            section = match section_name.as_str() {
                "interface" => {
                    saw_interface = true;
                    WireGuardConfigSection::Interface
                }
                "peer" => {
                    if saw_peer {
                        return Err(anyhow!(
                            "WireGuard upstream import supports exactly one peer; extra [Peer] at line {line_no}"
                        ));
                    }
                    saw_peer = true;
                    WireGuardConfigSection::Peer
                }
                _ => {
                    return Err(anyhow!(
                        "unsupported WireGuard section [{section_name}] at line {line_no}"
                    ));
                }
            };
            continue;
        }

        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow!("expected key = value at line {line_no}"))?;
        let key = normalize_wireguard_config_key(key);
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if wireguard_config_key_is_shell_hook(&key) {
            return Err(anyhow!(
                "WireGuard hook directive at line {line_no} is not supported"
            ));
        }

        match section {
            WireGuardConfigSection::None => {
                return Err(anyhow!(
                    "WireGuard setting before any section at line {line_no}"
                ));
            }
            WireGuardConfigSection::Interface => match key.as_str() {
                "privatekey" => config.private_key = value.to_string(),
                "address" => addresses = parse_wireguard_address_list(value, line_no)?,
                "dns" => config.dns = parse_wireguard_value_list(value),
                "mtu" => config.mtu = parse_wireguard_u16(value, "MTU", line_no)?,
                "listenport" | "fwmark" | "table" | "saveconfig" => {
                    return Err(anyhow!(
                        "WireGuard interface setting '{key}' at line {line_no} is not supported by the upstream importer"
                    ));
                }
                _ => {
                    return Err(anyhow!(
                        "unsupported WireGuard interface setting '{key}' at line {line_no}"
                    ));
                }
            },
            WireGuardConfigSection::Peer => match key.as_str() {
                "publickey" => config.peer_public_key = value.to_string(),
                "presharedkey" => config.peer_preshared_key = value.to_string(),
                "endpoint" => config.endpoint = value.to_string(),
                "allowedips" => {
                    config.allowed_ips = parse_wireguard_allowed_ips(value, line_no)?;
                }
                "persistentkeepalive" => {
                    config.persistent_keepalive_secs =
                        parse_wireguard_u16(value, "PersistentKeepalive", line_no)?;
                }
                _ => {
                    return Err(anyhow!(
                        "unsupported WireGuard peer setting '{key}' at line {line_no}"
                    ));
                }
            },
        }
    }

    if !saw_interface {
        return Err(anyhow!(
            "WireGuard config is missing an [Interface] section"
        ));
    }
    if !saw_peer {
        return Err(anyhow!("WireGuard config is missing a [Peer] section"));
    }
    if !addresses.is_empty() {
        config.address = select_wireguard_exit_address(&addresses);
    }
    if config.allowed_ips.is_empty() {
        return Err(anyhow!("WireGuard peer is missing AllowedIPs"));
    }
    if !config.allowed_ips.iter().any(|route| route == "0.0.0.0/0") {
        return Err(anyhow!(
            "WireGuard upstream AllowedIPs must include 0.0.0.0/0"
        ));
    }

    normalize_wireguard_exit_config(&mut config);
    if config.address.trim().is_empty() {
        return Err(anyhow!("WireGuard interface is missing Address"));
    }
    if config.private_key.trim().is_empty() {
        return Err(anyhow!("WireGuard interface is missing PrivateKey"));
    }
    if config.peer_public_key.trim().is_empty() {
        return Err(anyhow!("WireGuard peer is missing PublicKey"));
    }
    if config.endpoint.trim().is_empty() {
        return Err(anyhow!("WireGuard peer is missing Endpoint"));
    }
    validate_wireguard_key(&config.private_key, "PrivateKey")?;
    validate_wireguard_key(&config.peer_public_key, "PublicKey")?;
    if !config.peer_preshared_key.trim().is_empty() {
        validate_wireguard_key(&config.peer_preshared_key, "PresharedKey")?;
    }
    Ok(config)
}

pub fn wireguard_exit_config_text(config: &WireGuardExitConfig) -> String {
    if !config.configured() {
        return String::new();
    }

    let mut lines = vec![
        "[Interface]".to_string(),
        format!("PrivateKey = {}", config.private_key),
        format!("Address = {}", config.address),
    ];
    if !config.dns.is_empty() {
        lines.push(format!("DNS = {}", config.dns.join(", ")));
    }
    if config.mtu > 0 {
        lines.push(format!("MTU = {}", config.mtu));
    }

    lines.push(String::new());
    lines.push("[Peer]".to_string());
    lines.push(format!("PublicKey = {}", config.peer_public_key));
    if !config.peer_preshared_key.trim().is_empty() {
        lines.push(format!("PresharedKey = {}", config.peer_preshared_key));
    }
    lines.push(format!("Endpoint = {}", config.endpoint));
    lines.push(format!("AllowedIPs = {}", config.allowed_ips.join(", ")));
    if config.persistent_keepalive_secs > 0 {
        lines.push(format!(
            "PersistentKeepalive = {}",
            config.persistent_keepalive_secs
        ));
    }

    lines.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireGuardConfigSection {
    None,
    Interface,
    Peer,
}

fn strip_wireguard_config_comment(line: &str) -> &str {
    line.split(['#', ';']).next().unwrap_or(line)
}

fn normalize_wireguard_config_key(key: &str) -> String {
    key.trim()
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn wireguard_config_key_is_shell_hook(key: &str) -> bool {
    matches!(
        key,
        "preup" | "postup" | "predown" | "postdown" | "preupcmd" | "postupcmd"
    )
}

fn parse_wireguard_value_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_wireguard_address_list(value: &str, line_no: usize) -> Result<Vec<String>> {
    parse_wireguard_value_list(value)
        .into_iter()
        .map(|address| normalize_wireguard_address(&address, line_no))
        .collect()
}

fn normalize_wireguard_address(value: &str, line_no: usize) -> Result<String> {
    let (ip, prefix) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("invalid WireGuard Address '{value}' at line {line_no}"))?;
    let ip: std::net::IpAddr = ip
        .trim()
        .parse()
        .with_context(|| format!("invalid WireGuard Address IP '{value}' at line {line_no}"))?;
    let prefix: u8 = prefix
        .trim()
        .parse()
        .with_context(|| format!("invalid WireGuard Address prefix '{value}' at line {line_no}"))?;
    let max_prefix = if ip.is_ipv4() { 32 } else { 128 };
    if prefix > max_prefix {
        return Err(anyhow!(
            "invalid WireGuard Address prefix '{value}' at line {line_no}"
        ));
    }
    Ok(format!("{ip}/{prefix}"))
}

fn select_wireguard_exit_address(addresses: &[String]) -> String {
    addresses
        .iter()
        .find(|address| {
            address
                .split_once('/')
                .and_then(|(ip, _)| ip.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| ip.is_ipv4())
        })
        .or_else(|| addresses.first())
        .cloned()
        .unwrap_or_default()
}

fn parse_wireguard_allowed_ips(value: &str, line_no: usize) -> Result<Vec<String>> {
    let mut routes = Vec::new();
    for route in parse_wireguard_value_list(value) {
        let normalized = normalize_advertised_route(&route).ok_or_else(|| {
            anyhow!("invalid WireGuard AllowedIPs route '{route}' at line {line_no}")
        })?;
        routes.push(normalized);
    }
    routes.sort();
    routes.dedup();
    Ok(routes)
}

fn parse_wireguard_u16(value: &str, field: &str, line_no: usize) -> Result<u16> {
    value
        .trim()
        .parse::<u16>()
        .with_context(|| format!("invalid WireGuard {field} '{value}' at line {line_no}"))
}

fn validate_wireguard_key(value: &str, field: &str) -> Result<()> {
    let raw = STANDARD
        .decode(value.trim())
        .with_context(|| format!("WireGuard {field} is not valid base64"))?;
    if raw.len() != 32 {
        return Err(anyhow!("WireGuard {field} must decode to exactly 32 bytes"));
    }
    Ok(())
}

fn default_wireguard_exit_interface() -> String {
    "nvpn-wg-exit".to_string()
}

fn default_wireguard_exit_allowed_ips() -> Vec<String> {
    vec!["0.0.0.0/0".to_string()]
}

fn default_wireguard_exit_mtu() -> u16 {
    1420
}

fn default_wireguard_exit_persistent_keepalive_secs() -> u16 {
    25
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn wireguard_exit_interface_is_default(value: &str) -> bool {
    value == default_wireguard_exit_interface()
}

fn wireguard_exit_allowed_ips_is_default(value: &[String]) -> bool {
    value == default_wireguard_exit_allowed_ips().as_slice()
}

fn wireguard_exit_mtu_is_default(value: &u16) -> bool {
    *value == default_wireguard_exit_mtu()
}

fn wireguard_exit_persistent_keepalive_secs_is_default(value: &u16) -> bool {
    *value == default_wireguard_exit_persistent_keepalive_secs()
}

fn default_exit_node_leak_protection() -> bool {
    true
}

fn normalize_wireguard_exit_config(config: &mut WireGuardExitConfig) {
    config.interface = config.interface.trim().to_string();
    if config.interface.is_empty() {
        config.interface = default_wireguard_exit_interface();
    }
    config.address = config.address.trim().to_string();
    config.private_key = config.private_key.trim().to_string();
    config.peer_public_key = config.peer_public_key.trim().to_string();
    config.peer_preshared_key = config.peer_preshared_key.trim().to_string();
    config.endpoint = config.endpoint.trim().to_string();
    config.allowed_ips = config
        .allowed_ips
        .iter()
        .filter_map(|route| normalize_advertised_route(route))
        .collect();
    config.allowed_ips.sort();
    config.allowed_ips.dedup();
    if config.allowed_ips.is_empty() {
        config.allowed_ips = default_wireguard_exit_allowed_ips();
    }
    config.dns = config
        .dns
        .iter()
        .map(|server| server.trim().to_string())
        .filter(|server| !server.is_empty())
        .collect();
    config.dns.sort();
    config.dns.dedup();
    if config.mtu == 0 {
        config.mtu = default_wireguard_exit_mtu();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatConfig {
    #[serde(default = "default_nat_enabled")]
    pub enabled: bool,
    #[serde(default = "default_nat_stun_servers")]
    pub stun_servers: Vec<String>,
    #[serde(default = "default_nat_discovery_timeout_secs")]
    pub discovery_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_network_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub network_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_secret: String,
    #[serde(default, alias = "participants")]
    pub devices: Vec<String>,
    /// Locally removed members. This is deliberately not part of the shared
    /// roster wire format: an admin keeps these tombstones so a later stale
    /// whole-roster snapshot cannot resurrect a removed device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_devices: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admins: Vec<String>,
    #[serde(
        default = "default_listen_for_join_requests",
        skip_serializing_if = "is_true"
    )]
    pub listen_for_join_requests: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_inviter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_join_request: Option<PendingOutboundJoinRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_join_requests: Vec<PendingInboundJoinRequest>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shared_roster_updated_at: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shared_roster_signed_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PendingOutboundJoinRequest {
    #[serde(default)]
    pub recipient: String,
    #[serde(default)]
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PendingInboundJoinRequest {
    #[serde(default)]
    pub requester: String,
    #[serde(default)]
    pub requester_node_name: String,
    #[serde(default)]
    pub requested_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledNetworkMesh {
    pub id: String,
    pub name: String,
    pub network_id: String,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedNetworkRoster {
    pub id: String,
    pub network_id: String,
    pub name: String,
    pub devices: Vec<String>,
    pub admins: Vec<String>,
    pub aliases: HashMap<String, String>,
    pub updated_at: u64,
    pub signed_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSignedSharedRosterUpdate {
    pub network_id: String,
    pub network_name: String,
    pub devices: Vec<String>,
    pub admins: Vec<String>,
    pub aliases: HashMap<String, String>,
    pub signed_at: u64,
    pub signed_by: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut config = Self {
            networks: vec![NetworkConfig {
                id: default_network_entry_id(1),
                name: default_network_name(1),
                enabled: default_network_enabled(),
                network_id: default_network_id(),
                invite_secret: default_invite_secret(),
                devices: Vec::new(),
                removed_devices: Vec::new(),
                admins: Vec::new(),
                listen_for_join_requests: default_listen_for_join_requests(),
                invite_inviter: String::new(),
                outbound_join_request: None,
                inbound_join_requests: Vec::new(),
                shared_roster_updated_at: 0,
                shared_roster_signed_by: String::new(),
            }],
            node_name: default_node_name(),
            lan_discovery_enabled: default_lan_discovery_enabled(),
            launch_on_startup: default_launch_on_startup(),
            autoconnect: default_autoconnect(),
            fips_peer_endpoints: HashMap::new(),
            fips_advertise_public_endpoint: default_fips_advertise_public_endpoint(),
            fips_host_tunnel_enabled: default_fips_host_tunnel_enabled(),
            connect_to_non_roster_fips_peers: default_connect_to_non_roster_fips_peers(),
            fips_nostr_discovery_enabled: default_fips_nostr_discovery_enabled(),
            fips_webrtc_enabled: default_fips_webrtc_enabled(),
            fips_bootstrap_enabled: default_fips_bootstrap_enabled(),
            fips_bootstrap_peers: default_fips_bootstrap_peers(),
            fips_host_inbound_tcp_ports: Vec::new(),
            mesh_mtu_profile: String::new(),
            mesh_underlay_udp_mtu: 0,
            mesh_tunnel_mtu: 0,
            exit_node: String::new(),
            internet_source: InternetSource::Direct,
            exit_node_public_paid_exit: false,
            exit_node_leak_protection: default_exit_node_leak_protection(),
            close_to_tray_on_close: default_close_to_tray_on_close(),
            magic_dns_suffix: default_magic_dns_suffix(),
            wireguard_exit: WireGuardExitConfig::default(),
            paid_exit: PaidExitConfig::default(),
            wallet_fiat_enabled: default_wallet_fiat_enabled(),
            wallet_fiat_currency: FiatCurrency::default(),
            peer_aliases: default_peer_aliases(),
            nat: NatConfig::default(),
            nostr: NostrConfig::default(),
            pending_nostr_join_request: None,
            node: NodeConfig::default(),
        };
        config.ensure_defaults();
        config
    }
}

impl Default for NatConfig {
    fn default() -> Self {
        Self {
            enabled: default_nat_enabled(),
            stun_servers: default_nat_stun_servers(),
            discovery_timeout_secs: default_nat_discovery_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    #[serde(default = "default_node_id")]
    pub id: String,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_tunnel_ip")]
    pub tunnel_ip: String,
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    #[serde(default)]
    pub advertised_routes: Vec<String>,
    #[serde(default)]
    pub advertise_exit_node: bool,
    #[serde(default, skip_serializing_if = "ConnectedUdpConfig::is_default")]
    pub connected_udp: ConnectedUdpConfig,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: default_node_id(),
            endpoint: default_endpoint(),
            tunnel_ip: default_tunnel_ip(),
            listen_port: default_listen_port(),
            advertised_routes: Vec::new(),
            advertise_exit_node: false,
            connected_udp: ConnectedUdpConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectedUdpConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fd_reserve: Option<usize>,
}

impl ConnectedUdpConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}
