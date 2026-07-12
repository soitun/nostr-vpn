use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::secure_dns::{
    SECURE_DNS_MAX_MESSAGE_BYTES, SecureDnsLookup, SecureDnsResolver, WireGuardDnsResolver,
    build_servfail_response,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

#[cfg(target_os = "macos")]
const SECURE_DNS_PORT: u16 = 1053;
#[cfg(not(target_os = "macos"))]
const SECURE_DNS_PORT: u16 = 53;
const SECURE_DNS_BIND: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, SECURE_DNS_PORT));
const SECURE_DNS_MAX_IN_FLIGHT: usize = 64;
const SECURE_DNS_CLIENT_IDLE: Duration = Duration::from_secs(10);
type SharedResolver = Arc<dyn SecureDnsLookup>;
type ResolverState = Arc<RwLock<SharedResolver>>;

pub(crate) struct SecureDnsRuntime {
    udp_task: JoinHandle<()>,
    tcp_task: JoinHandle<()>,
    records: Arc<RwLock<HashMap<String, Ipv4Addr>>>,
    resolver: ResolverState,
    wireguard_dns_servers: Vec<IpAddr>,
    _system_dns: SystemDnsGuard,
}

impl SecureDnsRuntime {
    pub(crate) async fn start(
        interface: &str,
        interface_index: Option<u32>,
        records: HashMap<String, Ipv4Addr>,
        wireguard_dns_servers: Vec<IpAddr>,
    ) -> Result<Self> {
        let udp = Arc::new(
            tokio::net::UdpSocket::bind(SECURE_DNS_BIND)
                .await
                .with_context(|| format!("failed to bind secure DNS UDP on {SECURE_DNS_BIND}"))?,
        );
        let tcp = tokio::net::TcpListener::bind(SECURE_DNS_BIND)
            .await
            .with_context(|| format!("failed to bind secure DNS TCP on {SECURE_DNS_BIND}"))?;
        let resolver = Arc::new(RwLock::new(dns_resolver(&wireguard_dns_servers)?));
        let records = Arc::new(RwLock::new(records));
        let udp_task = tokio::spawn(run_udp(udp, Arc::clone(&resolver), Arc::clone(&records)));
        let tcp_task = tokio::spawn(run_tcp(tcp, Arc::clone(&resolver), Arc::clone(&records)));
        let system_dns = match SystemDnsGuard::install(interface, interface_index) {
            Ok(guard) => guard,
            Err(error) => {
                udp_task.abort();
                tcp_task.abort();
                return Err(error);
            }
        };
        Ok(Self {
            udp_task,
            tcp_task,
            records,
            resolver,
            wireguard_dns_servers,
            _system_dns: system_dns,
        })
    }

    pub(crate) fn update_config(
        &mut self,
        records: HashMap<String, Ipv4Addr>,
        wireguard_dns_servers: Vec<IpAddr>,
    ) -> Result<()> {
        if let Ok(mut current) = self.records.write() {
            *current = records;
        }
        if self.wireguard_dns_servers != wireguard_dns_servers {
            let resolver = dns_resolver(&wireguard_dns_servers)?;
            *self
                .resolver
                .write()
                .map_err(|_| anyhow!("secure DNS resolver lock poisoned"))? = resolver;
            self.wireguard_dns_servers = wireguard_dns_servers;
        }
        Ok(())
    }

    pub(crate) fn update_records(&self, records: HashMap<String, Ipv4Addr>) {
        if let Ok(mut current) = self.records.write() {
            *current = records;
        }
    }

    pub(crate) async fn stop(self) {
        let mut runtime = self;
        runtime.udp_task.abort();
        runtime.tcp_task.abort();
        let _ = (&mut runtime.udp_task).await;
        let _ = (&mut runtime.tcp_task).await;
    }
}

fn dns_resolver(wireguard_dns_servers: &[IpAddr]) -> Result<SharedResolver> {
    if wireguard_dns_servers.is_empty() {
        return Ok(Arc::new(
            SecureDnsResolver::new().context("failed to initialize secure DNS")?,
        ));
    }
    Ok(Arc::new(
        WireGuardDnsResolver::new(wireguard_dns_servers)
            .context("failed to initialize WireGuard exit DNS")?,
    ))
}

fn current_resolver(resolver: &ResolverState) -> Option<SharedResolver> {
    resolver.read().ok().map(|resolver| Arc::clone(&*resolver))
}

impl Drop for SecureDnsRuntime {
    fn drop(&mut self) {
        self.udp_task.abort();
        self.tcp_task.abort();
    }
}

async fn run_udp(
    socket: Arc<tokio::net::UdpSocket>,
    resolver: ResolverState,
    records: Arc<RwLock<HashMap<String, Ipv4Addr>>>,
) {
    let permits = Arc::new(Semaphore::new(SECURE_DNS_MAX_IN_FLIGHT));
    let mut requests = JoinSet::new();
    let mut packet = vec![0_u8; SECURE_DNS_MAX_MESSAGE_BYTES];
    loop {
        tokio::select! {
            completed = requests.join_next(), if !requests.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::debug!(%error, "secure DNS UDP task failed");
                }
            }
            received = socket.recv_from(&mut packet) => {
                let Ok((length, peer)) = received else { break; };
                let query = packet[..length].to_vec();
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    if let Some(response) = build_servfail_response(&query) {
                        let _ = socket.send_to(&response, peer).await;
                    }
                    continue;
                };
                let socket = Arc::clone(&socket);
                let resolver = current_resolver(&resolver);
                let records = Arc::clone(&records);
                requests.spawn(async move {
                    let _permit = permit;
                    if let Some(response) = match resolver {
                        Some(resolver) =>
                            resolve_or_servfail(resolver.as_ref(), &records, &query).await,
                        None => build_servfail_response(&query),
                    }
                    {
                        let _ = socket.send_to(&response, peer).await;
                    }
                });
            }
        }
    }
    requests.abort_all();
}

async fn run_tcp(
    listener: tokio::net::TcpListener,
    resolver: ResolverState,
    records: Arc<RwLock<HashMap<String, Ipv4Addr>>>,
) {
    let permits = Arc::new(Semaphore::new(SECURE_DNS_MAX_IN_FLIGHT));
    let mut requests = JoinSet::new();
    loop {
        tokio::select! {
            completed = requests.join_next(), if !requests.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::debug!(%error, "secure DNS TCP task failed");
                }
            }
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { break; };
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let resolver = Arc::clone(&resolver);
                let records = Arc::clone(&records);
                requests.spawn(async move {
                    let _permit = permit;
                    handle_tcp(stream, resolver, records).await;
                });
            }
        }
    }
    requests.abort_all();
}

async fn handle_tcp(
    mut stream: tokio::net::TcpStream,
    resolver: ResolverState,
    records: Arc<RwLock<HashMap<String, Ipv4Addr>>>,
) {
    loop {
        let Ok(Ok(length)) = tokio::time::timeout(SECURE_DNS_CLIENT_IDLE, stream.read_u16()).await
        else {
            return;
        };
        let length = usize::from(length);
        if !(12..=SECURE_DNS_MAX_MESSAGE_BYTES).contains(&length) {
            return;
        }
        let mut query = vec![0_u8; length];
        let Ok(Ok(_)) =
            tokio::time::timeout(SECURE_DNS_CLIENT_IDLE, stream.read_exact(&mut query)).await
        else {
            return;
        };
        let response = match current_resolver(&resolver) {
            Some(resolver) => resolve_or_servfail(resolver.as_ref(), &records, &query).await,
            None => build_servfail_response(&query),
        };
        let Some(response) = response else {
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
}

async fn resolve_or_servfail(
    resolver: &dyn SecureDnsLookup,
    records: &Arc<RwLock<HashMap<String, Ipv4Addr>>>,
    query: &[u8],
) -> Option<Vec<u8>> {
    if let Ok(records) = records.read()
        && let Some(response) =
            nostr_vpn_core::magic_dns::build_magic_dns_response_if_handled(query, &records)
    {
        return Some(response);
    }
    match resolver.resolve(query).await {
        Ok(response) => Some(response),
        Err(error) => {
            tracing::debug!(%error, "secure DNS resolution failed closed");
            build_servfail_response(query)
        }
    }
}

struct SystemDnsGuard {
    #[cfg(target_os = "linux")]
    linux: LinuxDnsRestore,
    #[cfg(target_os = "macos")]
    resolver_path: PathBuf,
    #[cfg(target_os = "windows")]
    interface_index: u32,
}

#[cfg(target_os = "linux")]
enum LinuxDnsRestore {
    Resolved { interface: String },
    ContainerResolvConf { previous: Vec<u8> },
}

impl SystemDnsGuard {
    fn install(interface: &str, interface_index: Option<u32>) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let _ = interface_index;
            let resolved = (|| -> Result<()> {
                run_checked(Command::new("resolvectl").args(["dns", interface, "127.0.0.1"]))?;
                run_checked(Command::new("resolvectl").args(["domain", interface, "~."]))?;
                Ok(())
            })();
            if resolved.is_ok() {
                let _ = Command::new("resolvectl").arg("flush-caches").status();
                return Ok(Self {
                    linux: LinuxDnsRestore::Resolved {
                        interface: interface.to_string(),
                    },
                });
            }
            let _ = Command::new("resolvectl")
                .args(["revert", interface])
                .status();
            if !std::path::Path::new("/.dockerenv").exists() {
                return Err(resolved.expect_err("failed resolved setup has an error"));
            }
            let path = std::path::Path::new("/etc/resolv.conf");
            let previous = std::fs::read(path).context("failed to read container resolv.conf")?;
            std::fs::write(
                path,
                b"# Managed by nvpn secure DNS\nnameserver 127.0.0.1\noptions timeout:1 attempts:1\n",
            )
            .context("failed to install container secure DNS resolver")?;
            return Ok(Self {
                linux: LinuxDnsRestore::ContainerResolvConf { previous },
            });
        }

        #[cfg(target_os = "macos")]
        {
            let _ = (interface, interface_index);
            let resolver_path = PathBuf::from("/etc/resolver/nvpn-secure-dns");
            if let Some(parent) = resolver_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::write(&resolver_path, macos_secure_dns_resolver_config())
                .with_context(|| format!("failed to install {}", resolver_path.display()))?;
            return Ok(Self { resolver_path });
        }

        #[cfg(target_os = "windows")]
        {
            let _ = interface;
            let interface_index = interface_index
                .ok_or_else(|| anyhow!("Windows secure DNS requires a tunnel interface index"))?;
            run_windows_powershell(&windows_secure_dns_install_script(interface_index))?;
            return Ok(Self { interface_index });
        }

        #[allow(unreachable_code)]
        Err(anyhow!("secure system DNS is unsupported on this platform"))
    }
}

#[cfg(target_os = "macos")]
fn macos_secure_dns_resolver_config() -> String {
    format!(
        "# Managed by nvpn\ndomain .\nnameserver 127.0.0.1\nport {SECURE_DNS_PORT}\noptions timeout:1 attempts:1\n"
    )
}

impl Drop for SystemDnsGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        match &self.linux {
            LinuxDnsRestore::Resolved { interface } => {
                let _ = Command::new("resolvectl")
                    .args(["revert", interface])
                    .status();
                let _ = Command::new("resolvectl").arg("flush-caches").status();
            }
            LinuxDnsRestore::ContainerResolvConf { previous } => {
                let _ = std::fs::write("/etc/resolv.conf", previous);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::fs::remove_file(&self.resolver_path);
        }
        #[cfg(target_os = "windows")]
        {
            let _ =
                run_windows_powershell(&windows_secure_dns_uninstall_script(self.interface_index));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run_checked(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .context("failed to execute DNS configuration command")?;
    if output.status.success() {
        return Ok(());
    }
    let details = if output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout)
    } else {
        String::from_utf8_lossy(&output.stderr)
    };
    Err(anyhow!(
        "DNS configuration command failed: {}",
        details.trim()
    ))
}

#[cfg(any(target_os = "windows", test))]
fn windows_secure_dns_install_script(interface_index: u32) -> String {
    format!(
        concat!(
            "$ErrorActionPreference = 'Stop'\n",
            "$displayName = 'nostr-vpn secure DNS'\n",
            "$comment = 'nostr-vpn authenticated DNS-over-HTTPS stub'\n",
            "Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object {{ $_.DisplayName -eq $displayName -or $_.Comment -eq $comment }} | ForEach-Object {{ $_ | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue | Out-Null }}\n",
            "Set-DnsClientServerAddress -InterfaceIndex {} -ServerAddresses @('127.0.0.1') -ErrorAction Stop\n",
            "Add-DnsClientNrptRule -Namespace '.' -NameServers '127.0.0.1' -DisplayName $displayName -Comment $comment -ErrorAction Stop | Out-Null\n",
            "Clear-DnsClientCache -ErrorAction SilentlyContinue\n",
        ),
        interface_index
    )
}

#[cfg(any(target_os = "windows", test))]
fn windows_secure_dns_uninstall_script(interface_index: u32) -> String {
    format!(
        concat!(
            "$displayName = 'nostr-vpn secure DNS'\n",
            "$comment = 'nostr-vpn authenticated DNS-over-HTTPS stub'\n",
            "Get-DnsClientNrptRule -ErrorAction SilentlyContinue | Where-Object {{ $_.DisplayName -eq $displayName -or $_.Comment -eq $comment }} | ForEach-Object {{ $_ | Remove-DnsClientNrptRule -Force -ErrorAction SilentlyContinue | Out-Null }}\n",
            "Set-DnsClientServerAddress -InterfaceIndex {} -ResetServerAddresses -ErrorAction SilentlyContinue\n",
            "Clear-DnsClientCache -ErrorAction SilentlyContinue\n",
        ),
        interface_index
    )
}

#[cfg(target_os = "windows")]
fn run_windows_powershell(script: &str) -> Result<()> {
    run_checked(Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
    use hickory_proto::rr::{Name, RData, RecordType};
    use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

    struct FixtureResolver {
        fail: bool,
    }

    #[async_trait::async_trait]
    impl SecureDnsLookup for FixtureResolver {
        async fn resolve(
            &self,
            query: &[u8],
        ) -> std::result::Result<Vec<u8>, nostr_vpn_core::secure_dns::SecureDnsError> {
            if self.fail {
                return Err(nostr_vpn_core::secure_dns::SecureDnsError::InvalidResponse);
            }
            let request = Message::from_vec(query).expect("fixture query");
            let mut response =
                Message::new(request.id, MessageType::Response, request.metadata.op_code);
            response.metadata.recursion_available = true;
            for query in request.queries {
                response.add_query(query);
            }
            let mut packet = Vec::new();
            response
                .emit(&mut BinEncoder::new(&mut packet))
                .expect("fixture response");
            Ok(packet)
        }
    }

    fn query_packet(name: &str, id: u16) -> Vec<u8> {
        let mut query = Message::new(id, MessageType::Query, OpCode::Query);
        query.add_query(Query::query(
            Name::from_ascii(name).expect("query name"),
            RecordType::A,
        ));
        let mut packet = Vec::new();
        query
            .emit(&mut BinEncoder::new(&mut packet))
            .expect("query packet");
        packet
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_secure_dns_uses_explicit_unicast_resolver_port() {
        assert_eq!(
            SECURE_DNS_BIND,
            "127.0.0.1:1053".parse::<SocketAddr>().unwrap()
        );
        let resolver = macos_secure_dns_resolver_config();
        assert!(resolver.contains("nameserver 127.0.0.1\n"));
        assert!(resolver.contains("port 1053\n"));
        assert!(resolver.contains("domain .\n"));
    }

    #[test]
    fn windows_policy_forces_all_dns_to_local_authenticated_stub() {
        let script = windows_secure_dns_install_script(42);
        assert!(script.contains("-InterfaceIndex 42"));
        assert!(script.contains("-Namespace '.'"));
        assert!(script.contains("-NameServers '127.0.0.1'"));
        assert!(!script.contains("1.1.1.1"));
        assert!(!script.contains("9.9.9.9"));
        let cleanup = windows_secure_dns_uninstall_script(42);
        assert!(cleanup.contains("-InterfaceIndex 42"));
        assert!(cleanup.contains("-ResetServerAddresses"));
    }

    #[tokio::test]
    async fn magic_dns_is_answered_locally_before_doh() {
        let packet = query_packet("peer.nvpn.", 55);
        let records = Arc::new(RwLock::new(HashMap::from([(
            "peer.nvpn".to_string(),
            Ipv4Addr::new(10, 44, 1, 9),
        )])));
        let resolver = SecureDnsResolver::new().expect("secure resolver");

        let response = resolve_or_servfail(&resolver, &records, &packet)
            .await
            .expect("local response");
        let response = Message::from_vec(&response).expect("DNS response");
        assert_eq!(response.id, 55);
        assert!(response.answers.iter().any(|answer| {
            matches!(
                &answer.data,
                RData::A(hickory_proto::rr::rdata::A(address))
                    if *address == Ipv4Addr::new(10, 44, 1, 9)
            )
        }));
    }

    #[tokio::test]
    async fn local_stub_serves_udp_and_fails_closed() {
        let server = Arc::new(
            tokio::net::UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("UDP server"),
        );
        let address = server.local_addr().expect("UDP address");
        let resolver: ResolverState =
            Arc::new(RwLock::new(Arc::new(FixtureResolver { fail: true })));
        let records = Arc::new(RwLock::new(HashMap::new()));
        let task = tokio::spawn(run_udp(server, resolver, records));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("UDP client");
        client
            .send_to(&query_packet("example.com.", 81), address)
            .await
            .expect("UDP query");
        let mut response = [0_u8; 512];
        let (length, _) =
            tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut response))
                .await
                .expect("UDP timeout")
                .expect("UDP response");
        task.abort();

        let response = Message::from_vec(&response[..length]).expect("DNS response");
        assert_eq!(response.id, 81);
        assert_eq!(response.metadata.response_code, ResponseCode::ServFail);
    }

    #[tokio::test]
    async fn local_stub_serves_framed_tcp_dns() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP server");
        let address = listener.local_addr().expect("TCP address");
        let resolver: ResolverState =
            Arc::new(RwLock::new(Arc::new(FixtureResolver { fail: false })));
        let records = Arc::new(RwLock::new(HashMap::new()));
        let task = tokio::spawn(run_tcp(listener, resolver, records));
        let mut client = tokio::net::TcpStream::connect(address)
            .await
            .expect("TCP client");
        let query = query_packet("example.com.", 82);
        client
            .write_all(&(query.len() as u16).to_be_bytes())
            .await
            .expect("TCP query length");
        client.write_all(&query).await.expect("TCP query");
        let response_length = client.read_u16().await.expect("TCP response length") as usize;
        let mut response = vec![0_u8; response_length];
        client
            .read_exact(&mut response)
            .await
            .expect("TCP response");
        task.abort();

        let response = Message::from_vec(&response).expect("DNS response");
        assert_eq!(response.id, 82);
        assert_eq!(response.metadata.message_type, MessageType::Response);
    }
}
