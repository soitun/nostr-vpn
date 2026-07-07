fn ipv4_is_local_only(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_link_local()
        || ip.is_loopback()
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
}

fn endpoint_host_ip(endpoint: &str) -> Option<IpAddr> {
    let host = endpoint
        .rsplit_once(':')
        .map_or(endpoint, |(host, _)| host)
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.parse::<IpAddr>().ok()
}

fn endpoint_is_local_only(endpoint: &str) -> bool {
    match endpoint_host_ip(endpoint) {
        Some(IpAddr::V4(ip)) => ipv4_is_local_only(ip),
        Some(IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unicast_link_local() || ip.is_unique_local()
        }
        None => endpoint.eq_ignore_ascii_case("localhost"),
    }
}

#[cfg(test)]
const TEST_MACOS_EUID_SENTINEL: u32 = u32::MAX;
#[cfg(test)]
static TEST_MACOS_EUID_OVERRIDE: AtomicU32 = AtomicU32::new(TEST_MACOS_EUID_SENTINEL);
#[cfg(test)]
static TEST_MACOS_EUID_OVERRIDE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
#[cfg(test)]
static TEST_REPAIR_SAVED_NETWORK_STATE_CALLS: AtomicU32 = AtomicU32::new(0);
#[cfg(test)]
static TEST_REPAIR_SAVED_NETWORK_STATE_CALLS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
