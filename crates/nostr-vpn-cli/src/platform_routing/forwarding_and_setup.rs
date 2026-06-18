#[cfg(target_os = "linux")]
fn linux_ip_forward_path(family: LinuxExitNodeIpFamily) -> &'static str {
    match family {
        LinuxExitNodeIpFamily::V4 => "/proc/sys/net/ipv4/ip_forward",
        LinuxExitNodeIpFamily::V6 => "/proc/sys/net/ipv6/conf/all/forwarding",
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub(crate) fn linux_exit_node_source_cidr(tunnel_ip: &str) -> Option<String> {
    let octets = strip_cidr(tunnel_ip).parse::<Ipv4Addr>().ok()?.octets();
    if octets[0] == 10 && octets[1] == 44 {
        return Some("10.44.0.0/16".to_string());
    }

    Some(format!("{}.{}.{}.0/24", octets[0], octets[1], octets[2]))
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinuxExitNodeIpFamily {
    V4,
    V6,
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(all(test, target_os = "windows"), allow(dead_code))]
pub(crate) struct LinuxExitNodeDefaultRouteFamilies {
    pub(crate) ipv4: bool,
    pub(crate) ipv6: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
#[cfg_attr(all(test, target_os = "windows"), allow(dead_code))]
pub(crate) fn linux_exit_node_default_route_families(
    routes: &[String],
) -> LinuxExitNodeDefaultRouteFamilies {
    LinuxExitNodeDefaultRouteFamilies {
        ipv4: routes.iter().any(|route| route == "0.0.0.0/0"),
        ipv6: routes.iter().any(|route| route == "::/0"),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_exit_node_firewall_binary(family: LinuxExitNodeIpFamily) -> &'static str {
    match family {
        LinuxExitNodeIpFamily::V4 => "iptables",
        LinuxExitNodeIpFamily::V6 => "ip6tables",
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_exit_node_forward_in_rule(
    tunnel_iface: &str,
    outbound_iface: &str,
    tunnel_source_cidr: &str,
    family: LinuxExitNodeIpFamily,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-i".to_string(),
        tunnel_iface.to_string(),
        "-o".to_string(),
        outbound_iface.to_string(),
        "-s".to_string(),
        tunnel_source_cidr.to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        match family {
            LinuxExitNodeIpFamily::V4 => "nvpn-exit-forward-in",
            LinuxExitNodeIpFamily::V6 => "nvpn-exit6-forward-in",
        }
        .to_string(),
        "-j".to_string(),
        "ACCEPT".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_exit_node_forward_out_rule(
    tunnel_iface: &str,
    outbound_iface: &str,
    family: LinuxExitNodeIpFamily,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-i".to_string(),
        outbound_iface.to_string(),
        "-o".to_string(),
        tunnel_iface.to_string(),
        "-m".to_string(),
        "conntrack".to_string(),
        "--ctstate".to_string(),
        "RELATED,ESTABLISHED".to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        match family {
            LinuxExitNodeIpFamily::V4 => "nvpn-exit-forward-out",
            LinuxExitNodeIpFamily::V6 => "nvpn-exit6-forward-out",
        }
        .to_string(),
        "-j".to_string(),
        "ACCEPT".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_exit_node_legacy_forward_in_rule(
    iface: &str,
    family: LinuxExitNodeIpFamily,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-i".to_string(),
        iface.to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        match family {
            LinuxExitNodeIpFamily::V4 => "nvpn-exit-forward-in",
            LinuxExitNodeIpFamily::V6 => "nvpn-exit6-forward-in",
        }
        .to_string(),
        "-j".to_string(),
        "ACCEPT".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_exit_node_legacy_forward_out_rule(
    iface: &str,
    family: LinuxExitNodeIpFamily,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-o".to_string(),
        iface.to_string(),
        "-m".to_string(),
        "conntrack".to_string(),
        "--ctstate".to_string(),
        "RELATED,ESTABLISHED".to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        match family {
            LinuxExitNodeIpFamily::V4 => "nvpn-exit-forward-out",
            LinuxExitNodeIpFamily::V6 => "nvpn-exit6-forward-out",
        }
        .to_string(),
        "-j".to_string(),
        "ACCEPT".to_string(),
    ]
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_exit_node_ipv4_masquerade_rule(
    outbound_iface: &str,
    tunnel_source_cidr: &str,
) -> Vec<String> {
    vec![
        "POSTROUTING".to_string(),
        "-o".to_string(),
        outbound_iface.to_string(),
        "-s".to_string(),
        tunnel_source_cidr.to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        "nvpn-exit-masq".to_string(),
        "-j".to_string(),
        "MASQUERADE".to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_exit_node_ipv4_mss_clamp_rule(
    tunnel_iface: &str,
    outbound_iface: &str,
    tunnel_source_cidr: &str,
    mss: u16,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-i".to_string(),
        tunnel_iface.to_string(),
        "-o".to_string(),
        outbound_iface.to_string(),
        "-s".to_string(),
        tunnel_source_cidr.to_string(),
        "-p".to_string(),
        "tcp".to_string(),
        "--tcp-flags".to_string(),
        "SYN,RST".to_string(),
        "SYN".to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        "nvpn-exit-mss".to_string(),
        "-j".to_string(),
        "TCPMSS".to_string(),
        "--set-mss".to_string(),
        mss.to_string(),
    ]
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_wireguard_exit_inbound_drop_rule(
    wireguard_iface: &str,
    tunnel_iface: &str,
    tunnel_source_cidr: &str,
) -> Vec<String> {
    vec![
        "FORWARD".to_string(),
        "-i".to_string(),
        wireguard_iface.to_string(),
        "-o".to_string(),
        tunnel_iface.to_string(),
        "-d".to_string(),
        tunnel_source_cidr.to_string(),
        "-m".to_string(),
        "conntrack".to_string(),
        "--ctstate".to_string(),
        "NEW,INVALID".to_string(),
        "-m".to_string(),
        "comment".to_string(),
        "--comment".to_string(),
        "nvpn-wg-upstream-inbound-drop".to_string(),
        "-j".to_string(),
        "DROP".to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn linux_iptables_rule_exists(
    family: LinuxExitNodeIpFamily,
    table: Option<&str>,
    rule: &[String],
) -> Result<bool> {
    let mut command = ProcessCommand::new(linux_exit_node_firewall_binary(family));
    if let Some(table) = table {
        command.arg("-t").arg(table);
    }
    command.arg("-C");
    for arg in rule {
        command.arg(arg);
    }

    let display = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("failed to execute {display}"))?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(
        "command failed: {display}\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_iptables_ensure_rule(
    family: LinuxExitNodeIpFamily,
    table: Option<&str>,
    rule: &[String],
) -> Result<()> {
    if linux_iptables_rule_exists(family, table, rule)? {
        return Ok(());
    }

    let mut command = ProcessCommand::new(linux_exit_node_firewall_binary(family));
    if let Some(table) = table {
        command.arg("-t").arg(table);
    }
    command.arg("-A");
    for arg in rule {
        command.arg(arg);
    }
    run_checked(&mut command)
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_iptables_ensure_rule_at_front(
    family: LinuxExitNodeIpFamily,
    table: Option<&str>,
    rule: &[String],
) -> Result<()> {
    if linux_iptables_rule_exists(family, table, rule)? {
        return Ok(());
    }

    let Some((chain, args)) = rule.split_first() else {
        return Err(anyhow!("iptables rule is missing a chain"));
    };

    let mut command = ProcessCommand::new(linux_exit_node_firewall_binary(family));
    if let Some(table) = table {
        command.arg("-t").arg(table);
    }
    command.arg("-I").arg(chain).arg("1");
    for arg in args {
        command.arg(arg);
    }
    run_checked(&mut command)
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_iptables_delete_rule(
    family: LinuxExitNodeIpFamily,
    table: Option<&str>,
    rule: &[String],
) -> Result<()> {
    if !linux_iptables_rule_exists(family, table, rule)? {
        return Ok(());
    }

    let mut command = ProcessCommand::new(linux_exit_node_firewall_binary(family));
    if let Some(table) = table {
        command.arg("-t").arg(table);
    }
    command.arg("-D");
    for arg in rule {
        command.arg(arg);
    }
    run_checked(&mut command)
}

#[cfg(any(test, not(target_os = "windows")))]
#[cfg_attr(all(test, target_os = "windows"), allow(dead_code))]
#[allow(dead_code)]
pub(crate) fn apply_local_interface_network_with_mtu(
    iface: &str,
    address: &str,
    route_targets: &[String],
    mtu: u16,
) -> Result<()> {
    apply_local_interface_network_with_mtu_and_addresses(
        iface,
        &[address.to_string()],
        route_targets,
        mtu,
    )
}

#[cfg(any(test, not(target_os = "windows")))]
#[cfg_attr(all(test, target_os = "windows"), allow(dead_code))]
pub(crate) fn apply_local_interface_network_with_mtu_and_addresses(
    iface: &str,
    addresses: &[String],
    route_targets: &[String],
    mtu: u16,
) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let mtu = mtu.to_string();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let mtu = mtu.as_str();
    #[cfg(target_os = "linux")]
    {
        let ipv4_route_source = addresses
            .iter()
            .find_map(|address| linux_ipv4_route_source(address));
        let local_has_ipv4 = addresses
            .iter()
            .any(|address| linux_tunnel_address_is_ipv4(address));
        let local_has_ipv6 = addresses
            .iter()
            .any(|address| linux_tunnel_address_is_ipv6(address));
        for address in addresses {
            run_checked(
                ProcessCommand::new("ip")
                    .arg("address")
                    .arg("replace")
                    .arg(address)
                    .arg("dev")
                    .arg(iface),
            )?;
        }
        run_checked(
            ProcessCommand::new("ip")
                .arg("link")
                .arg("set")
                .arg("mtu")
                .arg(mtu)
                .arg("up")
                .arg("dev")
                .arg(iface),
        )?;
        for target in route_targets {
            if linux_route_target_is_ipv4(target) && !local_has_ipv4 {
                continue;
            }
            if linux_route_target_is_ipv6(target) && !local_has_ipv6 {
                continue;
            }
            if target == "0.0.0.0/0" {
                let _ = ProcessCommand::new("ip")
                    .arg("-4")
                    .arg("route")
                    .arg("del")
                    .arg("default")
                    .status();
            } else if target == "::/0" {
                let _ = ProcessCommand::new("ip")
                    .arg("-6")
                    .arg("route")
                    .arg("del")
                    .arg("default")
                    .status();
            }
            let mut command = ProcessCommand::new("ip");
            command.args(linux_route_replace_args(
                target,
                iface,
                ipv4_route_source.as_deref(),
            ));
            run_checked(&mut command)?;
            if target == "fd00::/8" {
                let _ = ProcessCommand::new("ip")
                    .arg("-6")
                    .arg("rule")
                    .arg("add")
                    .arg("to")
                    .arg(target)
                    .arg("table")
                    .arg("main")
                    .arg("priority")
                    .arg("5265")
                    .status();
            }
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let primary_address = addresses
            .iter()
            .find(|address| linux_tunnel_address_is_ipv4(address))
            .or_else(|| addresses.first())
            .ok_or_else(|| anyhow!("no tunnel interface address configured"))?;
        let ip = strip_cidr(primary_address).to_string();
        run_checked(
            ProcessCommand::new("ifconfig")
                .arg(iface)
                .arg("inet")
                .arg(&ip)
                .arg(&ip)
                .arg("netmask")
                .arg(macos_tunnel_ipv4_netmask())
                .arg("mtu")
                .arg(mtu)
                .arg("up"),
        )?;
        for address in addresses {
            if !linux_tunnel_address_is_ipv6(address) {
                continue;
            }
            let (ip, prefix) = split_cidr(address, "128");
            run_checked(
                ProcessCommand::new("ifconfig")
                    .arg(iface)
                    .arg("inet6")
                    .arg(ip)
                    .arg("prefixlen")
                    .arg(prefix)
                    .arg("alias"),
            )?;
        }
        eprintln!(
            "tunnel: applying macOS interface {} with routes [{}]",
            iface,
            route_targets.join(", ")
        );
        for target in route_targets {
            apply_macos_route(iface, target)?;
        }
        return Ok(());
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = (iface, addresses, route_targets, mtu);

    #[allow(unreachable_code)]
    Err(anyhow!(
        "interface setup is not implemented for this platform"
    ))
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_tunnel_ipv4_netmask() -> &'static str {
    "255.255.255.255"
}
