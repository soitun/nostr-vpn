#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinuxRouteGetSpec {
    pub(crate) gateway: Option<String>,
    pub(crate) dev: String,
    pub(crate) src: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) struct LinuxEndpointBypassRoute {
    pub(crate) target: String,
    pub(crate) gateway: Option<String>,
    pub(crate) dev: String,
    pub(crate) src: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) struct LinuxDefaultRouteSpec {
    pub(crate) line: String,
    pub(crate) dev: String,
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_default_route_device_from_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        tokens
            .windows(2)
            .find(|window| window[0] == "dev")
            .map(|window| window[1].to_string())
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_route_get_spec_from_output(output: &str) -> Option<LinuxRouteGetSpec> {
    let line = output.lines().find(|line| !line.trim().is_empty())?.trim();
    let tokens = line.split_whitespace().collect::<Vec<_>>();

    let mut gateway = None;
    let mut dev = None;
    let mut src = None;
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index] {
            "via" => {
                gateway = tokens.get(index + 1).map(|value| (*value).to_string());
                index += 2;
            }
            "dev" => {
                dev = tokens.get(index + 1).map(|value| (*value).to_string());
                index += 2;
            }
            "src" => {
                src = tokens.get(index + 1).map(|value| (*value).to_string());
                index += 2;
            }
            _ => {
                index += 1;
            }
        }
    }

    Some(LinuxRouteGetSpec {
        gateway,
        dev: dev?,
        src,
    })
}

#[cfg(target_os = "linux")]
fn linux_default_route_from_output(output: &str) -> Option<LinuxDefaultRouteSpec> {
    let line = output.lines().find(|line| !line.trim().is_empty())?.trim();
    Some(LinuxDefaultRouteSpec {
        line: line.to_string(),
        dev: linux_default_route_device_from_output(line)?,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn command_stdout_checked(command: &mut ProcessCommand) -> Result<String> {
    let display = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("failed to execute {display}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "command failed: {display}\nstdout: {}\nstderr: {}",
            stdout.trim(),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_default_route() -> Result<LinuxDefaultRouteSpec> {
    linux_default_route_for_family("-4", "IPv4")
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_default_ipv6_route() -> Result<LinuxDefaultRouteSpec> {
    linux_default_route_for_family("-6", "IPv6")
}

#[cfg(target_os = "linux")]
fn linux_default_route_for_family(
    family_flag: &str,
    family_label: &str,
) -> Result<LinuxDefaultRouteSpec> {
    let output = command_stdout_checked(
        ProcessCommand::new("ip")
            .arg(family_flag)
            .arg("route")
            .arg("show")
            .arg("default"),
    )?;
    linux_default_route_from_output(&output)
        .ok_or_else(|| anyhow!("failed to resolve default {family_label} route"))
}

#[cfg(target_os = "linux")]
pub(crate) fn restore_linux_default_route(route: &str) -> Result<()> {
    restore_linux_default_route_for_family("-4", route)
}

#[cfg(target_os = "linux")]
pub(crate) fn restore_linux_default_ipv6_route(route: &str) -> Result<()> {
    restore_linux_default_route_for_family("-6", route)
}

#[cfg(target_os = "linux")]
fn restore_linux_default_route_for_family(family_flag: &str, route: &str) -> Result<()> {
    let mut command = ProcessCommand::new("ip");
    command.arg(family_flag).arg("route").arg("replace");
    for token in route.split_whitespace() {
        command.arg(token);
    }
    run_checked(&mut command)
}

#[cfg(target_os = "linux")]
pub(crate) fn delete_linux_default_route() -> Result<()> {
    run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("del")
            .arg("default"),
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn delete_linux_default_ipv6_route() -> Result<()> {
    run_checked(
        ProcessCommand::new("ip")
            .arg("-6")
            .arg("route")
            .arg("del")
            .arg("default"),
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn flush_linux_route_cache() -> Result<()> {
    run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("flush")
            .arg("cache"),
    )
}

#[cfg(target_os = "linux")]
fn relay_bypass_ipv4_hosts(app: &AppConfig) -> Vec<Ipv4Addr> {
    let mut hosts = app
        .nostr
        .relays
        .iter()
        .flat_map(|relay| relay_ipv4_hosts(relay))
        .collect::<Vec<_>>();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

#[cfg(target_os = "linux")]
fn relay_ipv4_hosts(relay: &str) -> Vec<Ipv4Addr> {
    let Some((host, port)) = relay_host_port(relay) else {
        return Vec::new();
    };

    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return vec![ip];
    }

    if host.parse::<IpAddr>().is_ok() {
        return Vec::new();
    }

    (host.as_str(), port)
        .to_socket_addrs()
        .map(|addrs| {
            addrs
                .filter_map(|addr| match addr.ip() {
                    IpAddr::V4(ip) => Some(ip),
                    IpAddr::V6(_) => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn relay_host_port(relay: &str) -> Option<(String, u16)> {
    let relay = relay.trim();
    if relay.is_empty() {
        return None;
    }

    let (scheme, remainder) = relay
        .split_once("://")
        .map_or(("", relay), |(scheme, rest)| (scheme, rest));
    let authority = remainder.split('/').next().unwrap_or(remainder);
    let default_port = match scheme {
        "wss" | "https" => 443,
        _ => 80,
    };

    split_host_port(authority, default_port)
}

#[cfg(target_os = "linux")]
pub(crate) fn stun_host_port(server: &str) -> Option<(String, u16)> {
    let server = server.trim();
    if server.is_empty() {
        return None;
    }

    let authority = server
        .strip_prefix("stun://")
        .or_else(|| server.strip_prefix("stun:"))
        .unwrap_or(server);

    split_host_port(authority, 3478)
}

#[cfg(target_os = "linux")]
fn stun_ipv4_hosts(app: &AppConfig) -> Vec<Ipv4Addr> {
    let mut hosts = app
        .nat
        .stun_servers
        .iter()
        .filter_map(|server| stun_host_port(server))
        .flat_map(|(host, port)| {
            if let Ok(ip) = host.parse::<Ipv4Addr>() {
                return vec![ip];
            }

            if host.parse::<IpAddr>().is_ok() {
                return Vec::new();
            }

            (host.as_str(), port)
                .to_socket_addrs()
                .map(|addrs| {
                    addrs
                        .filter_map(|addr| match addr.ip() {
                            IpAddr::V4(ip) => Some(ip),
                            IpAddr::V6(_) => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

#[cfg(target_os = "linux")]
fn management_ipv4_hosts_from_interfaces(interfaces: &[NetworkInterface]) -> Vec<Ipv4Addr> {
    let mut hosts = interfaces
        .iter()
        .filter(|interface| interface.is_up() && !interface.is_loopback() && !interface.is_tun())
        .flat_map(|interface| {
            let gateways = interface
                .gateway
                .iter()
                .flat_map(|gateway| gateway.ipv4.iter().copied());
            let dns_servers = interface
                .dns_servers
                .iter()
                .filter_map(|server| match server {
                    IpAddr::V4(ip) => Some(*ip),
                    IpAddr::V6(_) => None,
                });
            gateways.chain(dns_servers).collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

#[cfg(target_os = "linux")]
pub(crate) fn control_plane_bypass_ipv4_hosts_from_interfaces(
    app: &AppConfig,
    interfaces: &[NetworkInterface],
) -> Vec<Ipv4Addr> {
    let mut hosts = relay_bypass_ipv4_hosts(app);
    hosts.extend(stun_ipv4_hosts(app));
    hosts.extend(management_ipv4_hosts_from_interfaces(interfaces));
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

#[cfg(target_os = "linux")]
pub(crate) fn control_plane_bypass_ipv4_hosts(app: &AppConfig) -> Vec<Ipv4Addr> {
    control_plane_bypass_ipv4_hosts_from_interfaces(app, &get_interfaces())
}

#[cfg(target_os = "linux")]
pub(crate) fn split_host_port(authority: &str, default_port: u16) -> Option<(String, u16)> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after_host) = rest.split_once(']')?;
        let port = after_host
            .strip_prefix(':')
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(default_port);
        return Some((host.to_string(), port));
    }

    match authority.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':') && !host.is_empty() && port.parse::<u16>().is_ok() =>
        {
            Some((host.to_string(), port.parse::<u16>().ok()?))
        }
        _ => Some((authority.to_string(), default_port)),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_bypass_route_specs_for_hosts(
    mut hosts: Vec<Ipv4Addr>,
    tunnel_iface: &str,
    original_default_route: Option<&str>,
) -> Result<Vec<LinuxEndpointBypassRoute>> {
    hosts.sort_unstable();
    hosts.dedup();

    let mut routes = Vec::with_capacity(hosts.len());
    for host in hosts {
        let output = command_stdout_checked(
            ProcessCommand::new("ip")
                .arg("-4")
                .arg("route")
                .arg("get")
                .arg(host.to_string()),
        )?;
        let spec = linux_route_get_spec_from_output(&output)
            .and_then(|spec| {
                if spec.dev == tunnel_iface {
                    None
                } else {
                    Some(spec)
                }
            })
            .or_else(|| {
                original_default_route
                    .and_then(linux_route_get_spec_from_output)
                    .filter(|spec| spec.dev != tunnel_iface)
            })
            .ok_or_else(|| anyhow!("failed to resolve bypass route for {host}"))?;
        routes.push(LinuxEndpointBypassRoute {
            target: format!("{host}/32"),
            gateway: spec.gateway,
            dev: spec.dev,
            src: spec.src,
        });
    }

    Ok(routes)
}

#[cfg(target_os = "linux")]
pub(crate) fn apply_linux_endpoint_bypass_route(route: &LinuxEndpointBypassRoute) -> Result<()> {
    let mut command = ProcessCommand::new("ip");
    command
        .arg("-4")
        .arg("route")
        .arg("replace")
        .arg(&route.target);
    if let Some(gateway) = route.gateway.as_deref() {
        command.arg("via").arg(gateway);
    }
    command.arg("dev").arg(&route.dev);
    if let Some(src) = route.src.as_deref() {
        command.arg("src").arg(src);
    }
    run_checked(&mut command)
}

#[cfg(target_os = "linux")]
pub(crate) fn delete_linux_endpoint_bypass_route(target: &str) -> Result<()> {
    run_checked(
        ProcessCommand::new("ip")
            .arg("-4")
            .arg("route")
            .arg("del")
            .arg(target),
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_default_routes() -> Result<Vec<MacosRouteSpec>> {
    crate::macos_network::macos_default_routes()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_underlay_default_route_from_routes(
    routes: &[MacosRouteSpec],
) -> Option<MacosRouteSpec> {
    crate::macos_network::macos_underlay_default_route_from_routes(routes)
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_underlay_default_route_from_system() -> Result<Option<MacosRouteSpec>> {
    crate::macos_network::macos_underlay_default_route_from_system()
}

#[cfg(target_os = "macos")]
pub(crate) fn delete_macos_managed_route(
    target: &str,
    gateway: Option<&str>,
    interface: Option<&str>,
) -> Result<()> {
    crate::macos_network::delete_macos_managed_route(target, gateway, interface)
}

#[cfg(target_os = "macos")]
pub(crate) fn restore_macos_default_route(route: &MacosRouteSpec) -> Result<()> {
    crate::macos_network::restore_macos_default_route(route)
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_macos_default_route(
    gateway: Option<&str>,
    ifscope: Option<&str>,
) -> Result<()> {
    crate::macos_network::apply_macos_default_route(gateway, ifscope)
}

#[cfg(target_os = "macos")]
pub(crate) fn delete_macos_default_route_for_interface(iface: &str) -> Result<()> {
    crate::macos_network::delete_macos_default_route_for_interface(iface)
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_iface_has_ipv4_address(iface: &str, needle: Ipv4Addr) -> Result<bool> {
    crate::macos_network::macos_iface_has_ipv4_address(iface, needle)
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_macos_route_spec(
    target: &str,
    gateway: Option<&str>,
    ifscope: Option<&str>,
) -> Result<()> {
    crate::macos_network::apply_macos_route_spec(target, gateway, ifscope)
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_pf_enabled() -> Result<bool> {
    crate::macos_network::macos_pf_enabled()
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_macos_exit_node_pf_rules(
    tunnel_iface: &str,
    outbound_iface: &str,
    tunnel_source_cidr: &str,
) -> Result<()> {
    crate::macos_network::apply_macos_exit_node_pf_rules(
        tunnel_iface,
        outbound_iface,
        tunnel_source_cidr,
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn enable_macos_pf() -> Result<()> {
    crate::macos_network::enable_macos_pf()
}

#[cfg(target_os = "macos")]
pub(crate) fn cleanup_macos_pf_nat() -> Result<()> {
    crate::macos_network::cleanup_macos_pf_nat()
}

#[cfg(target_os = "linux")]
pub(crate) fn read_linux_ip_forward(family: LinuxExitNodeIpFamily) -> Result<bool> {
    let path = linux_ip_forward_path(family);
    Ok(fs::read_to_string(path)
        .with_context(|| format!("failed to read {path}"))?
        .trim()
        == "1")
}

#[cfg(target_os = "linux")]
pub(crate) fn write_linux_ip_forward(family: LinuxExitNodeIpFamily, enabled: bool) -> Result<()> {
    let path = linux_ip_forward_path(family);
    fs::write(path, if enabled { "1" } else { "0" })
        .with_context(|| format!("failed to write {path}"))
}
