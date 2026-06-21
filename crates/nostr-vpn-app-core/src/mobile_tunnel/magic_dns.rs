struct MobileDnsQuery<'a> {
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &'a [u8],
}

async fn mobile_magic_dns_response_packet(
    packet: &[u8],
    app_config: &Arc<RwLock<AppConfig>>,
    forwarders: &[SocketAddr],
) -> Option<Vec<u8>> {
    let query = parse_mobile_magic_dns_query(packet)?;
    let response = {
        let app = app_config.read().ok()?;
        let records = build_magic_dns_records(&app);
        build_magic_dns_response_if_handled(query.payload, &records)
    };
    let response = match response {
        Some(response) => response,
        None => forward_mobile_dns_query(query.payload, forwarders).await?,
    };
    build_mobile_dns_response_packet(&query, &response)
}

fn parse_mobile_magic_dns_query(packet: &[u8]) -> Option<MobileDnsQuery<'_>> {
    if packet.len() < 28 || packet[0] >> 4 != 4 {
        return None;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    if header_len < 20 || packet.len() < header_len + 8 || packet[9] != 17 {
        return None;
    }
    let total_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if total_len < header_len + 8 || packet.len() < total_len {
        return None;
    }
    let fragment = u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff;
    if fragment != 0 {
        return None;
    }
    let source = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let destination = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
    if destination != parse_ipv4(nostr_vpn_core::MESH_MAGIC_DNS_SERVER)? {
        return None;
    }
    let udp = header_len;
    let source_port = u16::from_be_bytes([packet[udp], packet[udp + 1]]);
    let destination_port = u16::from_be_bytes([packet[udp + 2], packet[udp + 3]]);
    if destination_port != 53 {
        return None;
    }
    let udp_len = usize::from(u16::from_be_bytes([packet[udp + 4], packet[udp + 5]]));
    if udp_len < 8 || udp + udp_len > total_len {
        return None;
    }
    Some(MobileDnsQuery {
        source,
        destination,
        source_port,
        destination_port,
        payload: &packet[udp + 8..udp + udp_len],
    })
}

fn build_mobile_dns_response_packet(
    query: &MobileDnsQuery<'_>,
    dns_response: &[u8],
) -> Option<Vec<u8>> {
    let udp_len = 8_usize.checked_add(dns_response.len())?;
    let total_len = 20_usize.checked_add(udp_len)?;
    let udp_len_u16 = u16::try_from(udp_len).ok()?;
    let total_len_u16 = u16::try_from(total_len).ok()?;
    let mut packet = vec![0_u8; total_len];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&total_len_u16.to_be_bytes());
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&query.destination.octets());
    packet[16..20].copy_from_slice(&query.source.octets());
    let checksum = ipv4_header_checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());

    packet[20..22].copy_from_slice(&query.destination_port.to_be_bytes());
    packet[22..24].copy_from_slice(&query.source_port.to_be_bytes());
    packet[24..26].copy_from_slice(&udp_len_u16.to_be_bytes());
    packet[28..].copy_from_slice(dns_response);
    Some(packet)
}

fn ipv4_header_checksum(header: &[u8]) -> u16 {
    let mut sum = 0_u32;
    for chunk in header.chunks(2) {
        let value = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from(chunk[0]) << 8
        };
        sum = sum.wrapping_add(u32::from(value));
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !u16::try_from(sum).expect("folded IPv4 checksum fits in u16")
}

async fn forward_mobile_dns_query(query: &[u8], forwarders: &[SocketAddr]) -> Option<Vec<u8>> {
    for forwarder in forwarders {
        let bind_addr = if forwarder.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        let Ok(socket) = tokio::net::UdpSocket::bind(bind_addr).await else {
            continue;
        };
        if socket.send_to(query, forwarder).await.is_err() {
            continue;
        }
        let mut buffer = vec![0_u8; 4096];
        let Ok(Ok((len, _))) = tokio::time::timeout(
            MOBILE_MAGIC_DNS_FORWARD_TIMEOUT,
            socket.recv_from(&mut buffer),
        )
        .await
        else {
            continue;
        };
        buffer.truncate(len);
        return Some(buffer);
    }
    None
}

fn mobile_magic_dns_forwarders(
    configured: &[String],
    tunnel_dns_servers: &[String],
    magic_dns_server: &str,
) -> Vec<SocketAddr> {
    let mut seen = HashSet::new();
    tunnel_dns_servers
        .iter()
        .filter(|server| server.trim() != magic_dns_server.trim())
        .filter_map(|server| parse_dns_forwarder(server))
        .chain(
            configured
                .iter()
                .filter_map(|server| parse_dns_forwarder(server)),
        )
        .chain(
            MOBILE_MAGIC_DNS_FORWARDERS
                .iter()
                .filter_map(|server| parse_dns_forwarder(server)),
        )
        .filter(|server| seen.insert(*server))
        .collect()
}

fn parse_dns_forwarder(value: &str) -> Option<SocketAddr> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value.parse::<SocketAddr>().ok().or_else(|| {
        value
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, 53))
    })
}

fn ipv4_is_lan_endpoint_hint(ip: Ipv4Addr) -> bool {
    ip.is_private() && !ipv4_is_mesh_tunnel_ip(ip)
}

fn ipv4_is_mesh_tunnel_ip(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 10 && octets[1] == 44
}

#[cfg(debug_assertions)]
pub(crate) fn mobile_debug_log(message: impl AsRef<str>) {
    let dir = std::env::temp_dir();
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("nvpn-mobile-debug.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{:?} {}", SystemTime::now(), message.as_ref());
}

#[cfg(not(debug_assertions))]
pub(crate) fn mobile_debug_log(_message: impl AsRef<str>) {}

fn parse_ipv4(value: &str) -> Option<Ipv4Addr> {
    strip_cidr(value.trim()).parse().ok()
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Append-once-per-line packet diagnostic for local debug builds.
fn log_pump_packet(message: &str) {
    #[cfg(all(debug_assertions, any(target_os = "ios", target_os = "android")))]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join("nvpn-wg.log");
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{secs:.3} mobile-pump: {message}");
        }
    }
    #[cfg(not(all(debug_assertions, any(target_os = "ios", target_os = "android"))))]
    let _ = message;
}

fn empty_config() -> MobileTunnelConfig {
    MobileTunnelConfig {
        config_path: String::new(),
        app_config_toml: String::new(),
        identity_nsec: String::new(),
        node_name: String::new(),
        network_id: String::new(),
        invite_secret: String::new(),
        local_address: String::new(),
        advertised_endpoint: String::new(),
        listen_port: 0,
        mtu: DEFAULT_MOBILE_MTU,
        peers: Vec::new(),
        peer_hints: HashMap::new(),
        bootstrap_peers: HashMap::new(),
        route_targets: Vec::new(),
        nostr_relays: Vec::new(),
        stun_servers: Vec::new(),
        share_local_candidates: false,
        connect_to_non_roster_fips_peers: false,
        nostr_discovery_enabled: true,
        excluded_routes: Vec::new(),
        dns_servers: Vec::new(),
        dns_forwarders: Vec::new(),
        magic_dns_server: String::new(),
        wireguard_exit: None,
        join_requests_enabled: false,
        pending_join_request_recipient: String::new(),
        pending_join_invite_secret: String::new(),
        pending_join_requested_at: 0,
        error: String::new(),
    }
}
