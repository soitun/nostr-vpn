use std::fs;

const WEBVM_FIPS_HOST_IFACE: &str = "nvpnfips0";
const WEBVM_FIPS_HOST_MTU: u16 = 1280;
const WEBVM_FIPS_DNS_BIND: &str = "127.0.0.1:53";
const WEBVM_FIPS_ROUTE: &str = "fd00::/8";
const WEBVM_RESOLV_CONF: &str = "/etc/resolv.conf";

pub(crate) struct WebvmFipsHostNetworkRuntime {
    stop: Arc<AtomicBool>,
    vpn_dns_enabled: Arc<AtomicBool>,
    tun_read_thread: Option<std::thread::JoinHandle<()>>,
    tun_write_thread: Option<std::thread::JoinHandle<()>>,
    outbound_task: Option<tokio::task::JoinHandle<()>>,
    inbound_task: Option<tokio::task::JoinHandle<()>>,
    dns_task: Option<tokio::task::JoinHandle<()>>,
    dns_tcp_task: Option<tokio::task::JoinHandle<()>>,
    resolver: WebvmResolverGuard,
    _tun: Arc<SystemTun>,
}

impl WebvmFipsHostNetworkRuntime {
    pub(crate) async fn start(endpoint: Arc<FipsEndpoint>) -> Result<Self> {
        ensure_linux_tun_permissions(WEBVM_FIPS_HOST_IFACE)?;
        let tun = Arc::new(
            SystemTun::new(WEBVM_FIPS_HOST_IFACE)
                .with_context(|| fips_tun_create_context(WEBVM_FIPS_HOST_IFACE))?
                .set_non_blocking()
                .context("failed to set WebVM .fips TUN nonblocking")?,
        );
        let iface = tun.name().context("failed to read WebVM .fips TUN name")?;
        let address = format!("{}/128", endpoint.address().to_ipv6());
        crate::apply_local_interface_network_with_mtu_and_addresses(
            &iface,
            &[address],
            &[WEBVM_FIPS_ROUTE.to_string()],
            WEBVM_FIPS_HOST_MTU,
        )
        .context("failed to configure WebVM .fips route")?;

        let dns_socket = match tokio::net::UdpSocket::bind(WEBVM_FIPS_DNS_BIND).await {
            Ok(socket) => socket,
            Err(error) => {
                remove_webvm_fips_route_rule();
                return Err(error).with_context(|| {
                    format!("failed to bind WebVM .fips DNS on {WEBVM_FIPS_DNS_BIND}")
                });
            }
        };
        let dns_tcp = match tokio::net::TcpListener::bind(WEBVM_FIPS_DNS_BIND).await {
            Ok(listener) => listener,
            Err(error) => {
                remove_webvm_fips_route_rule();
                return Err(error).with_context(|| {
                    format!("failed to bind WebVM .fips DNS TCP on {WEBVM_FIPS_DNS_BIND}")
                });
            }
        };
        let mut resolver = match WebvmResolverGuard::install() {
            Ok(resolver) => resolver,
            Err(error) => {
                remove_webvm_fips_route_rule();
                return Err(error);
            }
        };
        let secure_dns = match nostr_vpn_core::secure_dns::SecureDnsResolver::new() {
            Ok(resolver) => resolver,
            Err(error) => {
                remove_webvm_fips_route_rule();
                let _ = resolver.restore();
                return Err(error).context("failed to initialize WebVM secure DNS");
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let vpn_dns_enabled = Arc::new(AtomicBool::new(false));

        let (tun_outbound_tx, mut tun_outbound_rx) = mpsc::channel::<Vec<u8>>(256);
        let (tun_inbound_tx, tun_inbound_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(256);
        let tun_fd = BorrowedTunFd::new(tun.as_raw_fd());
        let tun_read_thread = Some(spawn_webvm_fips_tun_reader(
            Arc::clone(&tun),
            Arc::clone(&stop),
            tun_outbound_tx,
        ));
        let tun_write_thread = Some(spawn_webvm_fips_tun_writer(
            tun_fd,
            Arc::clone(&stop),
            tun_inbound_rx,
        ));

        let outbound_endpoint = Arc::clone(&endpoint);
        let outbound_task = tokio::spawn(async move {
            while let Some(packet) = tun_outbound_rx.recv().await {
                if let Err(error) = outbound_endpoint.send_ip_packet(packet).await {
                    eprintln!("webvm: failed to send .fips IPv6 packet: {error}");
                    break;
                }
            }
        });
        let inbound_endpoint = Arc::clone(&endpoint);
        let inbound_task = tokio::spawn(async move {
            while let Some(delivered) = inbound_endpoint.recv_ip_packet().await {
                if tun_inbound_tx.send(delivered.packet).is_err() {
                    break;
                }
            }
        });
        let dns_enabled = Arc::clone(&vpn_dns_enabled);
        let dns_task = tokio::spawn(run_webvm_fips_dns_udp(
            dns_socket,
            Arc::clone(&endpoint),
            secure_dns.clone(),
            Arc::clone(&dns_enabled),
        ));
        let dns_tcp_task = tokio::spawn(run_webvm_fips_dns_tcp(
            dns_tcp,
            endpoint,
            secure_dns,
            dns_enabled,
        ));

        println!("webvm: .fips IPv6 and DNS active on {iface} before approval");
        Ok(Self {
            stop,
            vpn_dns_enabled,
            tun_read_thread,
            tun_write_thread,
            outbound_task: Some(outbound_task),
            inbound_task: Some(inbound_task),
            dns_task: Some(dns_task),
            dns_tcp_task: Some(dns_tcp_task),
            resolver,
            _tun: tun,
        })
    }

    pub(crate) fn enable_vpn_dns(&self, exit_node: &str) -> Result<()> {
        let npub = normalize_fips_endpoint_npub(exit_node);
        PeerIdentity::from_npub(&npub)
            .with_context(|| format!("invalid approved WebVM exit node {exit_node}"))?;
        self.vpn_dns_enabled.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) async fn stop(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.outbound_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.inbound_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.dns_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.dns_tcp_task.take() {
            task.abort();
            let _ = task.await;
        }
        let read_thread = self.tun_read_thread.take();
        let write_thread = self.tun_write_thread.take();
        tokio::task::spawn_blocking(move || {
            if let Some(thread) = read_thread {
                let _ = thread.join();
            }
            if let Some(thread) = write_thread {
                let _ = thread.join();
            }
        })
        .await
        .context("WebVM .fips worker join failed")?;
        remove_webvm_fips_route_rule();
        self.resolver.restore()
    }
}

impl Drop for WebvmFipsHostNetworkRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = &self.outbound_task {
            task.abort();
        }
        if let Some(task) = &self.inbound_task {
            task.abort();
        }
        if let Some(task) = &self.dns_task {
            task.abort();
        }
        if let Some(task) = &self.dns_tcp_task {
            task.abort();
        }
        remove_webvm_fips_route_rule();
    }
}

fn remove_webvm_fips_route_rule() {
    let _ = ProcessCommand::new("ip")
        .args([
            "-6",
            "rule",
            "del",
            "to",
            WEBVM_FIPS_ROUTE,
            "table",
            "main",
            "priority",
            "5265",
        ])
        .status();
}

fn spawn_webvm_fips_tun_reader(
    tun: Arc<SystemTun>,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("nvpn-webvm-fips-read".to_string())
        .spawn(move || {
            let tun_fd = BorrowedTunFd::new(tun.as_raw_fd());
            let mut scratch = vec![0u8; tun.read_buffer_len()];
            let mut batch = TunPipelineBatch::with_capacity(32);
            while !stop.load(Ordering::Acquire) {
                if !wait_fd_readable_blocking(tun_fd.as_raw_fd(), &stop) {
                    continue;
                }
                batch.clear();
                match tun.read_packets_into(&mut scratch, &mut batch) {
                    Ok(_) => {
                        for packet in batch.drain(..) {
                            if tx.blocking_send(packet.bytes).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        eprintln!("webvm: .fips TUN read failed: {error}");
                        return;
                    }
                }
            }
        })
        .expect("failed to spawn WebVM .fips TUN reader")
}

fn spawn_webvm_fips_tun_writer(
    tun_fd: BorrowedTunFd,
    stop: Arc<AtomicBool>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("nvpn-webvm-fips-write".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(packet) => {
                        write_linux_vnet_raw_packet_to_tun_blocking(tun_fd, &packet, &stop);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("failed to spawn WebVM .fips TUN writer")
}

async fn run_webvm_fips_dns_udp(
    socket: tokio::net::UdpSocket,
    endpoint: Arc<FipsEndpoint>,
    secure_dns: nostr_vpn_core::secure_dns::SecureDnsResolver,
    vpn_dns_enabled: Arc<AtomicBool>,
) {
    let hosts = fips_core::upper::hosts::HostMap::new();
    let mut buf = [0u8; 4096];
    loop {
        let Ok((len, source)) = socket.recv_from(&mut buf).await else {
            return;
        };
        let response = answer_webvm_fips_dns(
            &buf[..len],
            &endpoint,
            &secure_dns,
            &vpn_dns_enabled,
            &hosts,
        )
        .await;
        if let Some(response) = response {
            let _ = socket.send_to(&response, source).await;
        }
    }
}

async fn run_webvm_fips_dns_tcp(
    listener: tokio::net::TcpListener,
    endpoint: Arc<FipsEndpoint>,
    secure_dns: nostr_vpn_core::secure_dns::SecureDnsResolver,
    vpn_dns_enabled: Arc<AtomicBool>,
) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let endpoint = Arc::clone(&endpoint);
        let secure_dns = secure_dns.clone();
        let vpn_dns_enabled = Arc::clone(&vpn_dns_enabled);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

            let hosts = fips_core::upper::hosts::HostMap::new();
            loop {
                let Ok(length) = stream.read_u16().await else {
                    return;
                };
                let length = usize::from(length);
                if !(12..=nostr_vpn_core::secure_dns::SECURE_DNS_MAX_MESSAGE_BYTES)
                    .contains(&length)
                {
                    return;
                }
                let mut query = vec![0_u8; length];
                if stream.read_exact(&mut query).await.is_err() {
                    return;
                }
                let Some(response) =
                    answer_webvm_fips_dns(&query, &endpoint, &secure_dns, &vpn_dns_enabled, &hosts)
                        .await
                else {
                    return;
                };
                let Ok(length) = u16::try_from(response.len()) else {
                    return;
                };
                if stream.write_all(&length.to_be_bytes()).await.is_err()
                    || stream.write_all(&response).await.is_err()
                {
                    return;
                }
            }
        });
    }
}

async fn answer_webvm_fips_dns(
    query: &[u8],
    endpoint: &FipsEndpoint,
    secure_dns: &nostr_vpn_core::secure_dns::SecureDnsResolver,
    vpn_dns_enabled: &AtomicBool,
    hosts: &fips_core::upper::hosts::HostMap,
) -> Option<Vec<u8>> {
    if let Some(response) = webvm_iris_localhost_dns_response(query) {
        return Some(response);
    }
    if webvm_dns_query_is_fips(query) {
        let (response, identity) = fips_core::upper::dns::handle_dns_packet(query, 60, hosts)?;
        if let Some(identity) = identity {
            let remote = PeerIdentity::from_pubkey_full(identity.pubkey);
            // TODO(fips): replace this route-seeding payload with a public
            // FipsEndpoint::register_identity API once FIPS exposes one.
            if let Err(error) = endpoint.send_batch_to_peer(remote, vec![Vec::new()]).await {
                eprintln!("webvm: .fips identity discovery failed: {error}");
            }
        }
        return Some(response);
    }
    answer_webvm_public_dns(
        query,
        secure_dns,
        vpn_dns_enabled.load(Ordering::Acquire),
    )
    .await
}

async fn answer_webvm_public_dns(
    query: &[u8],
    secure_dns: &nostr_vpn_core::secure_dns::SecureDnsResolver,
    vpn_dns_enabled: bool,
) -> Option<Vec<u8>> {
    if !vpn_dns_enabled {
        return webvm_public_dns_refused_response(query);
    }
    match nostr_vpn_core::secure_dns::SecureDnsLookup::resolve(secure_dns, query).await {
        Ok(response) => Some(response),
        Err(error) => {
            eprintln!("webvm: secure DNS request failed: {error:#}");
            nostr_vpn_core::secure_dns::build_servfail_response(query)
        }
    }
}

#[cfg(test)]
mod webvm_host_dns_tests {
    use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
    use hickory_proto::rr::{Name, RecordType};
    use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

    use super::*;

    fn query(name: &str) -> Vec<u8> {
        let mut query = Message::new(73, MessageType::Query, OpCode::Query);
        query.add_query(hickory_proto::op::Query::query(
            Name::from_ascii(name).expect("query name"),
            RecordType::A,
        ));
        let mut packet = Vec::new();
        query
            .emit(&mut BinEncoder::new(&mut packet))
            .expect("query packet");
        packet
    }

    #[tokio::test]
    async fn webvm_public_dns_refuses_until_exit_is_approved() {
        let resolver = nostr_vpn_core::secure_dns::SecureDnsResolver::new().expect("resolver");
        let response = answer_webvm_public_dns(&query("example.com."), &resolver, false)
            .await
            .expect("refused response");
        let response = Message::from_vec(&response).expect("DNS response");
        assert_eq!(response.metadata.response_code, ResponseCode::Refused);
    }

    #[tokio::test]
    #[ignore = "requires live authenticated DNS-over-HTTPS"]
    async fn webvm_public_dns_resolves_over_live_authenticated_doh() {
        let resolver = nostr_vpn_core::secure_dns::SecureDnsResolver::new().expect("resolver");
        let response = answer_webvm_public_dns(&query("example.com."), &resolver, true)
            .await
            .expect("DoH response");
        let response = Message::from_vec(&response).expect("DNS response");
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert!(!response.answers.is_empty());
    }
}

struct WebvmResolverGuard {
    previous: Option<Vec<u8>>,
}

impl WebvmResolverGuard {
    fn install() -> Result<Self> {
        let path = std::path::Path::new(WEBVM_RESOLV_CONF);
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect {WEBVM_RESOLV_CONF}"))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(anyhow!(
                "WebVM requires {WEBVM_RESOLV_CONF} to be a regular file"
            ));
        }
        let previous =
            fs::read(path).with_context(|| format!("failed to read {WEBVM_RESOLV_CONF}"))?;
        if let Err(error) = fs::write(
            path,
            b"# Managed by nvpn WebVM FIPS\nnameserver 127.0.0.1\noptions timeout:1 attempts:2\n",
        ) {
            let _ = fs::write(path, &previous);
            return Err(error).with_context(|| format!("failed to configure {WEBVM_RESOLV_CONF}"));
        }
        Ok(Self {
            previous: Some(previous),
        })
    }

    fn restore(&mut self) -> Result<()> {
        let Some(previous) = self.previous.take() else {
            return Ok(());
        };
        fs::write(WEBVM_RESOLV_CONF, previous)
            .with_context(|| format!("failed to restore {WEBVM_RESOLV_CONF}"))
    }
}

impl Drop for WebvmResolverGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
