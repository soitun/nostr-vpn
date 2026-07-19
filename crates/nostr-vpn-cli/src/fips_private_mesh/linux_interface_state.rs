#[cfg(any(target_os = "linux", test))]
#[derive(serde::Deserialize)]
struct LinuxIpInterfaceState {
    #[serde(default)]
    flags: Vec<String>,
    mtu: u16,
    #[serde(default)]
    txqlen: Option<usize>,
    #[serde(default)]
    addr_info: Vec<LinuxIpAddressState>,
}

#[cfg(any(target_os = "linux", test))]
#[derive(serde::Deserialize)]
struct LinuxIpAddressState {
    local: String,
    prefixlen: u8,
}

#[cfg(any(target_os = "linux", test))]
fn linux_interface_state_matches_json(
    raw: &str,
    addresses: &[String],
    mtu: u16,
    tx_queue_len: Option<usize>,
) -> bool {
    let Ok(interfaces) = serde_json::from_str::<Vec<LinuxIpInterfaceState>>(raw) else {
        return false;
    };
    let [interface] = interfaces.as_slice() else {
        return false;
    };
    if interface.mtu != mtu
        || !interface.flags.iter().any(|flag| flag == "UP")
        || tx_queue_len.is_some_and(|expected| interface.txqlen != Some(expected))
    {
        return false;
    }

    let actual = interface
        .addr_info
        .iter()
        .filter_map(|address| {
            address
                .local
                .parse::<IpAddr>()
                .ok()
                .map(|ip| (ip, address.prefixlen))
        })
        .collect::<HashSet<_>>();
    addresses.iter().all(|address| {
        let (ip, prefix_len) = address.split_once('/').map_or(
            (address.as_str(), None),
            |(ip, prefix_len)| (ip, Some(prefix_len)),
        );
        let Ok(ip) = ip.parse::<IpAddr>() else {
            return false;
        };
        let prefix_len = match prefix_len {
            Some(prefix_len) => match prefix_len.parse::<u8>() {
                Ok(prefix_len)
                    if (ip.is_ipv4() && prefix_len <= 32)
                        || (ip.is_ipv6() && prefix_len <= 128) =>
                {
                    prefix_len
                }
                _ => return false,
            },
            None if ip.is_ipv4() => 32,
            None => 128,
        };
        actual.contains(&(ip, prefix_len))
    })
}

#[cfg(target_os = "linux")]
fn linux_interface_state_matches(
    iface: &str,
    addresses: &[String],
    mtu: u16,
    tx_queue_len: Option<usize>,
) -> bool {
    let output = ProcessCommand::new("ip")
        .arg("-j")
        .arg("address")
        .arg("show")
        .arg("dev")
        .arg(iface)
        .output();
    let Ok(output) = output else {
        return false;
    };
    output.status.success()
        && std::str::from_utf8(&output.stdout).is_ok_and(|raw| {
            linux_interface_state_matches_json(raw, addresses, mtu, tx_queue_len)
        })
}
