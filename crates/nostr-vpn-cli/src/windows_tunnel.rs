use anyhow::{Context, Result, anyhow};
#[cfg(any(target_os = "windows", test))]
use netdev::interface::interface::Interface as NetworkInterface;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowsInterfaceAddress {
    pub address: Ipv4Addr,
    pub mask: Ipv4Addr,
}

pub(crate) fn windows_interface_address(address: &str) -> Result<WindowsInterfaceAddress> {
    let (ip, prefix_len) = address
        .trim()
        .split_once('/')
        .ok_or_else(|| anyhow!("windows interface address must be IPv4 CIDR"))?;
    let address = ip
        .parse::<Ipv4Addr>()
        .with_context(|| format!("invalid IPv4 interface address {ip}"))?;
    let prefix_len = prefix_len
        .parse::<u8>()
        .with_context(|| format!("invalid IPv4 prefix length {prefix_len}"))?;
    if prefix_len > 32 {
        return Err(anyhow!("invalid IPv4 prefix length {prefix_len}"));
    }

    Ok(WindowsInterfaceAddress {
        address,
        mask: ipv4_netmask(prefix_len),
    })
}

pub(crate) fn windows_add_route_args(prefix: &str, interface_index: u32) -> Result<Vec<String>> {
    validate_windows_route_prefix(prefix)?;
    Ok(vec![
        "interface".to_string(),
        "ipv4".to_string(),
        "add".to_string(),
        "route".to_string(),
        prefix.trim().to_string(),
        format!("interface={interface_index}"),
        "metric=1".to_string(),
        "store=active".to_string(),
    ])
}

pub(crate) fn windows_delete_route_args(prefix: &str, interface_index: u32) -> Result<Vec<String>> {
    validate_windows_route_prefix(prefix)?;
    Ok(vec![
        "interface".to_string(),
        "ipv4".to_string(),
        "delete".to_string(),
        "route".to_string(),
        prefix.trim().to_string(),
        format!("interface={interface_index}"),
        "store=active".to_string(),
    ])
}

fn validate_windows_route_prefix(prefix: &str) -> Result<()> {
    let trimmed = prefix.trim();
    let (ip, prefix_len) = trimmed
        .split_once('/')
        .ok_or_else(|| anyhow!("windows route prefix must be IPv4 CIDR"))?;
    ip.parse::<Ipv4Addr>()
        .with_context(|| format!("invalid windows route IPv4 prefix {ip}"))?;
    let prefix_len = prefix_len
        .parse::<u8>()
        .with_context(|| format!("invalid windows route prefix length {prefix_len}"))?;
    if prefix_len > 32 {
        return Err(anyhow!("invalid windows route prefix length {prefix_len}"));
    }
    Ok(())
}

fn ipv4_netmask(prefix_len: u8) -> Ipv4Addr {
    if prefix_len == 0 {
        return Ipv4Addr::UNSPECIFIED;
    }

    Ipv4Addr::from(u32::MAX << (32 - prefix_len))
}

#[cfg(target_os = "windows")]
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::net::UdpSocket;
#[cfg(any(target_os = "windows", test))]
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(any(target_os = "windows", test))]
use std::process::Command as ProcessCommand;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::thread::{self, JoinHandle};
#[cfg(target_os = "windows")]
use std::time::Duration;

#[cfg(any(target_os = "windows", test))]
use crate::TunnelPeer;
#[cfg(target_os = "windows")]
use crate::WireGuardPeerStatus;
#[cfg(target_os = "windows")]
use crate::userspace_wg::{
    DatagramProcessingResult, OutgoingDatagram, UserspaceWireGuardPeerConfig,
    UserspaceWireGuardRuntime,
};
#[cfg(target_os = "windows")]
use base64::Engine;
#[cfg(target_os = "windows")]
use base64::engine::general_purpose::STANDARD;
#[cfg(target_os = "windows")]
use nostr_vpn_wintun::load_wintun;
#[cfg(target_os = "windows")]
use wintun::{Adapter, MAX_RING_CAPACITY, Session};

#[cfg(target_os = "windows")]
const WINDOWS_TUNNEL_MTU: usize = 1380;
#[cfg(target_os = "windows")]
const WINDOWS_RUNTIME_IO_POLL_MS: u64 = 250;

#[cfg(target_os = "windows")]
pub(crate) struct WindowsTunnelBackend {
    session: Arc<Session>,
    runtime: Arc<Mutex<UserspaceWireGuardRuntime>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    interface_index: u32,
    endpoint_bypass_routes: Vec<WindowsEndpointBypassRoute>,
    route_targets: Vec<String>,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WindowsEndpointBypassRoute {
    pub target: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub interface_index: u32,
}

#[cfg(target_os = "windows")]
impl WindowsTunnelBackend {
    pub(crate) fn start(
        iface: &str,
        private_key_base64: &str,
        listen_port: u16,
        local_address: &str,
        peers: &[TunnelPeer],
    ) -> Result<Self> {
        let wintun = load_wintun()?;
        let adapter = Adapter::open(&wintun, iface)
            .or_else(|_| Adapter::create(&wintun, iface, "NostrVPN", None))
            .with_context(|| format!("failed to open or create wintun adapter {iface}"))?;
        adapter
            .set_mtu(WINDOWS_TUNNEL_MTU)
            .with_context(|| format!("failed to set MTU on wintun adapter {iface}"))?;
        let parsed_address = windows_interface_address(local_address)?;
        adapter
            .set_network_addresses_tuple(
                parsed_address.address.into(),
                parsed_address.mask.into(),
                None,
            )
            .with_context(|| format!("failed to set address on wintun adapter {iface}"))?;

        let interface_index = adapter
            .get_adapter_index()
            .with_context(|| format!("failed to resolve interface index for {iface}"))?;
        let route_targets = peers
            .iter()
            .flat_map(|peer| peer.allowed_ips.iter().cloned())
            .collect::<Vec<_>>();
        let endpoint_bypass_routes =
            windows_endpoint_bypass_routes_from_interfaces(peers, &netdev::get_interfaces())?;
        let applied_endpoint_bypass_routes =
            apply_windows_endpoint_bypass_routes(&endpoint_bypass_routes)?;
        let applied_routes = apply_windows_routes(interface_index, &route_targets)?;

        let session = Arc::new(
            adapter
                .start_session(MAX_RING_CAPACITY)
                .with_context(|| format!("failed to start wintun session for {iface}"))?,
        );
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, listen_port))
            .with_context(|| format!("failed to bind Windows WireGuard socket on {listen_port}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(WINDOWS_RUNTIME_IO_POLL_MS)))
            .context("failed to configure Windows WireGuard socket timeout")?;

        let runtime = Arc::new(Mutex::new(UserspaceWireGuardRuntime::new(
            private_key_base64,
            peers
                .iter()
                .map(userspace_peer_config_from_tunnel_peer)
                .collect::<Result<Vec<_>>>()?,
        )?));
        let stop = Arc::new(AtomicBool::new(false));

        let send_socket = socket
            .try_clone()
            .context("failed to clone Windows WireGuard send socket")?;
        let initial_outgoing = {
            let mut runtime = runtime
                .lock()
                .map_err(|_| anyhow!("userspace wireguard runtime mutex poisoned"))?;
            runtime.initiate_handshakes()
        };
        send_datagrams(&send_socket, initial_outgoing)?;

        let threads = vec![
            spawn_windows_udp_reader(
                stop.clone(),
                runtime.clone(),
                session.clone(),
                socket,
                send_socket
                    .try_clone()
                    .context("failed to clone UDP socket for Windows reader")?,
            ),
            spawn_windows_tun_reader(
                stop.clone(),
                runtime.clone(),
                session.clone(),
                send_socket
                    .try_clone()
                    .context("failed to clone UDP socket for Windows tunnel reader")?,
            ),
            spawn_windows_timer(stop.clone(), runtime.clone(), session.clone(), send_socket),
        ];

        Ok(Self {
            session,
            runtime,
            stop,
            threads,
            interface_index,
            endpoint_bypass_routes: applied_endpoint_bypass_routes,
            route_targets: applied_routes,
        })
    }

    pub(crate) fn peer_status(&self) -> Result<HashMap<String, WireGuardPeerStatus>> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("userspace wireguard runtime mutex poisoned"))?;
        Ok(runtime
            .peer_statuses()
            .into_iter()
            .map(|status| {
                let mut peer = WireGuardPeerStatus {
                    endpoint: Some(status.endpoint.to_string()),
                    last_handshake_sec: None,
                    last_handshake_nsec: None,
                    tx_bytes: status.tx_bytes,
                    rx_bytes: status.rx_bytes,
                };
                if let Some(handshake_age) = status.last_handshake_age {
                    peer.last_handshake_sec = Some(handshake_age.as_secs());
                    peer.last_handshake_nsec = Some(u64::from(handshake_age.subsec_nanos()));
                }
                (status.participant_pubkey, peer)
            })
            .collect())
    }

    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.session.shutdown();
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
        if let Err(error) = remove_windows_endpoint_bypass_routes(&self.endpoint_bypass_routes) {
            eprintln!("tunnel: failed to remove Windows endpoint bypass routes: {error}");
        }
        if let Err(error) = remove_windows_routes(self.interface_index, &self.route_targets) {
            eprintln!("tunnel: failed to remove Windows routes: {error}");
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsTunnelBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "windows")]
fn userspace_peer_config_from_tunnel_peer(
    peer: &TunnelPeer,
) -> Result<UserspaceWireGuardPeerConfig> {
    let public_key_bytes = hex::decode(&peer.pubkey_hex)
        .with_context(|| format!("invalid peer public key hex {}", peer.pubkey_hex))?;
    if public_key_bytes.len() != 32 {
        return Err(anyhow!(
            "invalid peer public key length {}; expected 32 bytes",
            public_key_bytes.len()
        ));
    }

    Ok(UserspaceWireGuardPeerConfig {
        participant_pubkey: peer.pubkey_hex.clone(),
        public_key_base64: STANDARD.encode(public_key_bytes),
        endpoint: peer
            .endpoint
            .parse::<SocketAddr>()
            .with_context(|| format!("invalid peer endpoint {}", peer.endpoint))?,
        allowed_ips: peer.allowed_ips.clone(),
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_udp_reader(
    stop: Arc<AtomicBool>,
    runtime: Arc<Mutex<UserspaceWireGuardRuntime>>,
    session: Arc<Session>,
    recv_socket: UdpSocket,
    send_socket: UdpSocket,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = vec![0_u8; 65_535];
        while !stop.load(Ordering::Relaxed) {
            match recv_socket.recv_from(&mut buffer) {
                Ok((len, source)) => {
                    let processed = match runtime.lock() {
                        Ok(mut runtime) => runtime.receive_datagram(source, &buffer[..len]),
                        Err(_) => Err(anyhow!("userspace wireguard runtime mutex poisoned")),
                    };
                    if let Err(error) = processed.and_then(|processed| {
                        apply_processed_datagrams(&session, &send_socket, processed)
                    }) {
                        eprintln!("tunnel: Windows UDP reader failed: {error}");
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        eprintln!("tunnel: Windows UDP socket receive failed: {error}");
                    }
                    break;
                }
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_tun_reader(
    stop: Arc<AtomicBool>,
    runtime: Arc<Mutex<UserspaceWireGuardRuntime>>,
    session: Arc<Session>,
    send_socket: UdpSocket,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let packet = match session.receive_blocking() {
                Ok(packet) => packet,
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        eprintln!("tunnel: Windows Wintun receive failed: {error}");
                    }
                    break;
                }
            };
            let payload = packet.bytes().to_vec();
            drop(packet);

            let outgoing = match runtime.lock() {
                Ok(mut runtime) => runtime.queue_tunnel_packet(&payload),
                Err(_) => Err(anyhow!("userspace wireguard runtime mutex poisoned")),
            };
            if let Err(error) = outgoing.and_then(|outgoing| send_datagrams(&send_socket, outgoing))
            {
                eprintln!("tunnel: Windows tunnel encapsulation failed: {error}");
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_timer(
    stop: Arc<AtomicBool>,
    runtime: Arc<Mutex<UserspaceWireGuardRuntime>>,
    session: Arc<Session>,
    send_socket: UdpSocket,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(WINDOWS_RUNTIME_IO_POLL_MS));
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let processed = match runtime.lock() {
                Ok(mut runtime) => Ok(runtime.tick_timers()),
                Err(_) => Err(anyhow!("userspace wireguard runtime mutex poisoned")),
            };
            if let Err(error) = processed
                .and_then(|processed| apply_processed_datagrams(&session, &send_socket, processed))
            {
                eprintln!("tunnel: Windows timer update failed: {error}");
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn apply_processed_datagrams(
    session: &Arc<Session>,
    send_socket: &UdpSocket,
    processed: DatagramProcessingResult,
) -> Result<()> {
    write_tunnel_packets(session, &processed.tunnel_packets)?;
    send_datagrams(send_socket, processed.outgoing)
}

#[cfg(target_os = "windows")]
pub(crate) fn write_tunnel_packets(session: &Arc<Session>, packets: &[Vec<u8>]) -> Result<()> {
    for packet in packets {
        let size = u16::try_from(packet.len())
            .map_err(|_| anyhow!("tunnel packet too large for wintun: {}", packet.len()))?;
        let mut outbound = session
            .allocate_send_packet(size)
            .context("failed to allocate packet for wintun session")?;
        outbound.bytes_mut().copy_from_slice(packet);
        session.send_packet(outbound);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn send_datagrams(socket: &UdpSocket, outgoing: Vec<OutgoingDatagram>) -> Result<()> {
    for datagram in outgoing {
        socket
            .send_to(&datagram.payload, datagram.endpoint)
            .with_context(|| {
                format!("failed to send WireGuard datagram to {}", datagram.endpoint)
            })?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn apply_windows_routes(
    interface_index: u32,
    route_targets: &[String],
) -> Result<Vec<String>> {
    let mut applied = Vec::new();
    for route_target in route_targets {
        let args = windows_add_route_args(route_target, interface_index)?;
        if let Err(error) = run_windows_netsh(&args) {
            let _ = remove_windows_routes(interface_index, &applied);
            return Err(error);
        }
        applied.push(route_target.clone());
    }
    Ok(applied)
}

#[cfg(target_os = "windows")]
pub(crate) fn remove_windows_routes(interface_index: u32, route_targets: &[String]) -> Result<()> {
    let mut first_error = None;
    for route_target in route_targets {
        let args = windows_delete_route_args(route_target, interface_index)?;
        if let Err(error) = run_windows_netsh(&args)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn windows_endpoint_bypass_routes_from_interfaces(
    peers: &[TunnelPeer],
    interfaces: &[NetworkInterface],
) -> Result<Vec<WindowsEndpointBypassRoute>> {
    let (interface_index, gateway) =
        windows_default_underlay_interface_from_interfaces(interfaces)?;
    let mut targets = peers
        .iter()
        .filter_map(|peer| peer.endpoint.parse::<SocketAddr>().ok())
        .filter_map(|endpoint| match endpoint.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();

    Ok(targets
        .into_iter()
        .map(|target| WindowsEndpointBypassRoute {
            target,
            gateway,
            interface_index,
        })
        .collect())
}

#[cfg(any(target_os = "windows", test))]
fn windows_default_underlay_interface_from_interfaces(
    interfaces: &[NetworkInterface],
) -> Result<(u32, Ipv4Addr)> {
    let interface = interfaces
        .iter()
        .find(|interface| {
            interface.default
                && interface.is_up()
                && !interface.is_loopback()
                && !interface.is_tun()
        })
        .ok_or_else(|| anyhow!("failed to resolve Windows underlay interface"))?;
    let gateway = interface
        .gateway
        .as_ref()
        .and_then(|gateway| gateway.ipv4.first().copied())
        .ok_or_else(|| {
            anyhow!(
                "failed to resolve Windows underlay gateway for {}",
                interface.name
            )
        })?;

    Ok((interface.index, gateway))
}

#[cfg(any(target_os = "windows", test))]
fn windows_add_endpoint_bypass_route_args(route: &WindowsEndpointBypassRoute) -> Vec<String> {
    vec![
        "add".to_string(),
        route.target.to_string(),
        "mask".to_string(),
        "255.255.255.255".to_string(),
        route.gateway.to_string(),
        "if".to_string(),
        route.interface_index.to_string(),
    ]
}

#[cfg(any(target_os = "windows", test))]
fn windows_delete_endpoint_bypass_route_args(route: &WindowsEndpointBypassRoute) -> Vec<String> {
    vec!["delete".to_string(), route.target.to_string()]
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn apply_windows_endpoint_bypass_routes(
    routes: &[WindowsEndpointBypassRoute],
) -> Result<Vec<WindowsEndpointBypassRoute>> {
    let mut applied = Vec::new();
    for route in routes {
        let add_args = windows_add_endpoint_bypass_route_args(route);
        if let Err(_add_error) = run_windows_route(&add_args) {
            let mut change_args = add_args.clone();
            change_args[0] = "change".to_string();
            if let Err(change_error) = run_windows_route(&change_args) {
                let _ = remove_windows_endpoint_bypass_routes(&applied);
                return Err(change_error.context(format!(
                    "failed to install Windows endpoint bypass route {}",
                    route.target
                )));
            }
        }
        applied.push(route.clone());
    }
    Ok(applied)
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn remove_windows_endpoint_bypass_routes(routes: &[WindowsEndpointBypassRoute]) -> Result<()> {
    let mut first_error = None;
    for route in routes {
        if let Err(error) = run_windows_route(&windows_delete_endpoint_bypass_route_args(route))
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn run_windows_route(args: &[String]) -> Result<()> {
    let display = format!("route {}", args.join(" "));
    let output = ProcessCommand::new("route")
        .args(args)
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
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_windows_netsh(args: &[String]) -> Result<()> {
    let display = format!("netsh {}", args.join(" "));
    let output = ProcessCommand::new("netsh")
        .args(args)
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{
        WindowsEndpointBypassRoute, WindowsInterfaceAddress,
        windows_add_endpoint_bypass_route_args, windows_add_route_args,
        windows_default_underlay_interface_from_interfaces,
        windows_delete_endpoint_bypass_route_args, windows_delete_route_args,
        windows_endpoint_bypass_routes_from_interfaces, windows_interface_address,
    };
    use netdev::interface::flags::{IFF_BROADCAST, IFF_MULTICAST, IFF_UP};
    use netdev::interface::interface::Interface as NetworkInterface;
    use netdev::net::device::NetworkDevice;

    #[test]
    fn parses_windows_interface_address_from_cidr() {
        assert_eq!(
            windows_interface_address("10.44.0.7/24").expect("parsed address"),
            WindowsInterfaceAddress {
                address: Ipv4Addr::new(10, 44, 0, 7),
                mask: Ipv4Addr::new(255, 255, 255, 0),
            }
        );
        assert_eq!(
            windows_interface_address("10.44.0.7/32").expect("parsed address"),
            WindowsInterfaceAddress {
                address: Ipv4Addr::new(10, 44, 0, 7),
                mask: Ipv4Addr::new(255, 255, 255, 255),
            }
        );
    }

    #[test]
    fn rejects_non_ipv4_windows_interface_address() {
        assert!(windows_interface_address("fd00::7/64").is_err());
        assert!(windows_interface_address("10.44.0.7").is_err());
    }

    #[test]
    fn builds_windows_route_add_arguments() {
        assert_eq!(
            windows_add_route_args("10.44.0.0/16", 7).expect("add args"),
            vec![
                "interface".to_string(),
                "ipv4".to_string(),
                "add".to_string(),
                "route".to_string(),
                "10.44.0.0/16".to_string(),
                "interface=7".to_string(),
                "metric=1".to_string(),
                "store=active".to_string(),
            ]
        );
    }

    #[test]
    fn builds_windows_route_delete_arguments() {
        assert_eq!(
            windows_delete_route_args("10.44.0.0/16", 7).expect("delete args"),
            vec![
                "interface".to_string(),
                "ipv4".to_string(),
                "delete".to_string(),
                "route".to_string(),
                "10.44.0.0/16".to_string(),
                "interface=7".to_string(),
                "store=active".to_string(),
            ]
        );
    }

    #[test]
    fn builds_windows_endpoint_bypass_route_arguments() {
        let route = WindowsEndpointBypassRoute {
            target: Ipv4Addr::new(203, 0, 113, 20),
            gateway: Ipv4Addr::new(192, 168, 64, 1),
            interface_index: 12,
        };

        assert_eq!(
            windows_add_endpoint_bypass_route_args(&route),
            vec![
                "add".to_string(),
                "203.0.113.20".to_string(),
                "mask".to_string(),
                "255.255.255.255".to_string(),
                "192.168.64.1".to_string(),
                "if".to_string(),
                "12".to_string(),
            ]
        );
        assert_eq!(
            windows_delete_endpoint_bypass_route_args(&route),
            vec!["delete".to_string(), "203.0.113.20".to_string()]
        );
    }

    #[test]
    fn windows_default_underlay_interface_uses_default_up_non_tun_interface() {
        let mut physical = NetworkInterface::dummy();
        physical.index = 42;
        physical.name = "Ethernet".to_string();
        physical.flags = (IFF_UP | IFF_BROADCAST | IFF_MULTICAST) as u32;
        physical.default = true;
        let mut gateway = NetworkDevice::new();
        gateway.ipv4.push(Ipv4Addr::new(192, 168, 64, 1));
        physical.gateway = Some(gateway);

        let mut tunnel = NetworkInterface::dummy();
        tunnel.index = 7;
        tunnel.name = "utun100".to_string();
        tunnel.flags = IFF_UP as u32;
        tunnel.default = false;

        assert_eq!(
            windows_default_underlay_interface_from_interfaces(&[tunnel, physical])
                .expect("underlay interface"),
            (42, Ipv4Addr::new(192, 168, 64, 1))
        );
    }

    #[test]
    fn windows_endpoint_bypass_routes_dedup_peer_endpoints() {
        let mut physical = NetworkInterface::dummy();
        physical.index = 42;
        physical.name = "Ethernet".to_string();
        physical.flags = (IFF_UP | IFF_BROADCAST | IFF_MULTICAST) as u32;
        physical.default = true;
        let mut gateway = NetworkDevice::new();
        gateway.ipv4.push(Ipv4Addr::new(192, 168, 64, 1));
        physical.gateway = Some(gateway);

        let routes = windows_endpoint_bypass_routes_from_interfaces(
            &[
                crate::TunnelPeer {
                    pubkey_hex: "a".repeat(64),
                    endpoint: "203.0.113.20:51820".to_string(),
                    allowed_ips: vec!["10.44.0.2/32".to_string()],
                },
                crate::TunnelPeer {
                    pubkey_hex: "b".repeat(64),
                    endpoint: "203.0.113.20:51820".to_string(),
                    allowed_ips: vec!["10.44.0.3/32".to_string()],
                },
                crate::TunnelPeer {
                    pubkey_hex: "c".repeat(64),
                    endpoint: "[2001:db8::1]:51820".to_string(),
                    allowed_ips: vec!["10.44.0.4/32".to_string()],
                },
            ],
            &[physical],
        )
        .expect("endpoint bypass routes");

        assert_eq!(
            routes,
            vec![WindowsEndpointBypassRoute {
                target: Ipv4Addr::new(203, 0, 113, 20),
                gateway: Ipv4Addr::new(192, 168, 64, 1),
                interface_index: 42,
            }]
        );
    }
}
