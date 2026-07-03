#[cfg(any(target_os = "linux", target_os = "macos"))]
fn temporary_tun_read_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_unix_packet_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("NVPN_FIPS_PACKET_DEBUG")
            .ok()
            .is_some_and(|value| fips_packet_debug_value_enabled(&value))
    })
}

fn fips_packet_debug_value_enabled(value: &str) -> bool {
    let value = value.trim();
    !(value.is_empty()
        || value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn describe_ip_packet(packet: &[u8]) -> String {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => {
            let src = std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
            let dst = std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
            format!("IPv4 proto={} {src}->{dst}", packet[9])
        }
        Some(6) if packet.len() >= 40 => {
            let src = std::net::Ipv6Addr::from([
                packet[8], packet[9], packet[10], packet[11], packet[12], packet[13], packet[14],
                packet[15], packet[16], packet[17], packet[18], packet[19], packet[20],
                packet[21], packet[22], packet[23],
            ]);
            let dst = std::net::Ipv6Addr::from([
                packet[24], packet[25], packet[26], packet[27], packet[28], packet[29],
                packet[30], packet[31], packet[32], packet[33], packet[34], packet[35],
                packet[36], packet[37], packet[38], packet[39],
            ]);
            format!("IPv6 next_header={} {src}->{dst}", packet[6])
        }
        Some(version) => format!("IP version {version} short packet"),
        None => "empty packet".to_string(),
    }
}
