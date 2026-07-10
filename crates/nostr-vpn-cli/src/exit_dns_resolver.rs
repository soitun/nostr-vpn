use std::collections::HashSet;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
#[cfg(target_os = "macos")]
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType};
use nostr_vpn_core::exit_dns::EXIT_DNS_MAX_PACKET_BYTES;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const DNS_PORT: u16 = 53;
const HOST_DNS_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_HOST_DNS_RESOLVERS: usize = 2;
const RESOLV_CONF: &str = "/etc/resolv.conf";

#[derive(Clone)]
pub(crate) struct HostDnsResolver {
    servers: Arc<[SocketAddr]>,
    timeout: Duration,
}

impl HostDnsResolver {
    pub(crate) fn system() -> Self {
        let servers = system_dns_resolvers();
        if servers.is_empty() {
            tracing::warn!("exit DNS has no OS-selected resolver; requests will return SERVFAIL");
        }
        Self {
            servers: servers.into(),
            timeout: HOST_DNS_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_servers(servers: Vec<SocketAddr>, timeout: Duration) -> Self {
        Self {
            servers: servers.into(),
            timeout,
        }
    }

    pub(crate) async fn resolve(&self, query: &[u8], transaction_id: u16) -> Option<Vec<u8>> {
        for server in self.servers.iter().copied() {
            if let Some(response) =
                query_resolver(server, query, transaction_id, self.timeout).await
            {
                return Some(response);
            }
        }
        None
    }
}

async fn query_resolver(
    server: SocketAddr,
    query: &[u8],
    transaction_id: u16,
    timeout: Duration,
) -> Option<Vec<u8>> {
    tokio::time::timeout(
        timeout,
        query_resolver_with_tcp_retry(server, query, transaction_id),
    )
    .await
    .ok()?
}

async fn query_resolver_with_tcp_retry(
    server: SocketAddr,
    query: &[u8],
    transaction_id: u16,
) -> Option<Vec<u8>> {
    let bind_addr = match server {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = tokio::net::UdpSocket::bind(bind_addr).await.ok()?;
    socket.connect(server).await.ok()?;
    socket.send(query).await.ok()?;

    let mut response = vec![0u8; EXIT_DNS_MAX_PACKET_BYTES];
    let len = socket.recv(&mut response).await.ok()?;
    response.truncate(len);
    let message = validated_dns_response(&response, transaction_id)?;
    if !message.metadata.truncation {
        return Some(response);
    }

    query_resolver_over_tcp(server, query, transaction_id).await
}

async fn query_resolver_over_tcp(
    server: SocketAddr,
    query: &[u8],
    transaction_id: u16,
) -> Option<Vec<u8>> {
    let query_len = u16::try_from(query.len()).ok()?;
    let mut stream = tokio::net::TcpStream::connect(server).await.ok()?;
    stream.write_all(&query_len.to_be_bytes()).await.ok()?;
    stream.write_all(query).await.ok()?;

    let response_len = stream.read_u16().await.ok()? as usize;
    if !(12..=EXIT_DNS_MAX_PACKET_BYTES).contains(&response_len) {
        return None;
    }
    let mut response = vec![0u8; response_len];
    stream.read_exact(&mut response).await.ok()?;
    let message = validated_dns_response(&response, transaction_id)?;
    if message.metadata.truncation {
        return None;
    }
    Some(response)
}

fn validated_dns_response(packet: &[u8], transaction_id: u16) -> Option<Message> {
    let message = Message::from_vec(packet).ok()?;
    if message.metadata.message_type != MessageType::Response || message.id != transaction_id {
        return None;
    }
    Some(message)
}

fn system_dns_resolvers() -> Vec<SocketAddr> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = ProcessCommand::new("scutil").arg("--dns").output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut servers = parse_macos_scutil_dns(&stdout);
            servers.truncate(MAX_HOST_DNS_RESOLVERS);
            if !servers.is_empty() {
                return servers;
            }
        }
    }

    let mut servers = fs::read_to_string(RESOLV_CONF)
        .ok()
        .map(|contents| parse_resolv_conf(&contents))
        .unwrap_or_default();
    servers.truncate(MAX_HOST_DNS_RESOLVERS);
    servers
}

fn parse_resolv_conf(contents: &str) -> Vec<SocketAddr> {
    let mut servers = Vec::new();
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let mut fields = line.split_whitespace();
        if fields.next() != Some("nameserver") {
            continue;
        }
        let Some(address) = fields.next().and_then(|value| value.parse::<IpAddr>().ok()) else {
            continue;
        };
        servers.push(SocketAddr::new(address, DNS_PORT));
    }
    deduplicate(servers)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_scutil_dns(contents: &str) -> Vec<SocketAddr> {
    let unscoped = contents
        .split("DNS configuration (for scoped queries)")
        .next()
        .unwrap_or(contents);
    let mut servers = Vec::new();
    for block in unscoped.split("resolver #") {
        let is_supplemental = block.lines().any(|line| {
            let line = line.trim_start();
            line.strip_prefix("domain :")
                .is_some_and(|domain| !domain.trim().is_empty())
                || line
                    .strip_prefix("flags")
                    .is_some_and(|flags| flags.contains("Supplemental"))
        });
        if is_supplemental {
            continue;
        }
        let port = block
            .lines()
            .find_map(|line| {
                line.trim_start()
                    .strip_prefix("port :")
                    .and_then(|port| port.trim().parse::<u16>().ok())
            })
            .unwrap_or(DNS_PORT);
        for line in block.lines() {
            let line = line.trim_start();
            if !line.starts_with("nameserver[") {
                continue;
            }
            let Some((_, address)) = line.split_once(" : ") else {
                continue;
            };
            if let Ok(address) = address.trim().parse::<IpAddr>() {
                servers.push(SocketAddr::new(address, port));
            }
        }
    }
    deduplicate(servers)
}

fn deduplicate(servers: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut seen = HashSet::new();
    servers
        .into_iter()
        .filter(|server| seen.insert(*server))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::{OpCode, Query};
    use hickory_proto::rr::{Name, RecordType};
    use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

    fn dns_query(id: u16) -> Vec<u8> {
        let mut message = Message::new(id, MessageType::Query, OpCode::Query);
        message.add_query(Query::query(
            Name::from_ascii("tcp-retry.example.").expect("DNS name"),
            RecordType::A,
        ));
        let mut bytes = Vec::new();
        message
            .emit(&mut BinEncoder::new(&mut bytes))
            .expect("DNS query");
        bytes
    }

    fn dns_response(query: &[u8], truncated: bool) -> Vec<u8> {
        let request = Message::from_vec(query).expect("DNS query");
        let mut response = Message::new(request.id, MessageType::Response, OpCode::Query);
        response.metadata.recursion_desired = request.recursion_desired;
        response.metadata.recursion_available = true;
        response.metadata.truncation = truncated;
        for query in request.queries {
            response.add_query(query);
        }
        let mut bytes = Vec::new();
        response
            .emit(&mut BinEncoder::new(&mut bytes))
            .expect("DNS response");
        bytes
    }

    #[test]
    fn resolv_conf_parser_keeps_only_nameservers_in_order() {
        let servers = parse_resolv_conf(
            "search example.test\n\
             nameserver 127.0.0.53\n\
             nameserver 2001:db8::53\n\
             nameserver 127.0.0.53 # duplicate\n\
             options timeout:1\n",
        );
        assert_eq!(
            servers,
            vec![
                "127.0.0.53:53".parse().expect("IPv4 resolver"),
                "[2001:db8::53]:53".parse().expect("IPv6 resolver"),
            ]
        );
    }

    #[test]
    fn macos_parser_uses_default_unscoped_resolvers_not_split_fips() {
        let servers = parse_macos_scutil_dns(
            "DNS configuration\n\n\
             resolver #1\n\
               search domain[0] : lan\n\
               nameserver[0] : 192.0.2.53\n\
               nameserver[1] : 2001:db8::53\n\
             resolver #2\n\
               domain : fips\n\
               nameserver[0] : ::1\n\
               port : 5354\n\n\
             DNS configuration (for scoped queries)\n\n\
             resolver #1\n\
               nameserver[0] : 198.51.100.53\n",
        );
        assert_eq!(
            servers,
            vec![
                "192.0.2.53:53".parse().expect("IPv4 resolver"),
                "[2001:db8::53]:53".parse().expect("IPv6 resolver"),
            ]
        );
    }

    #[test]
    fn macos_parser_skips_supplemental_search_domain_resolver() {
        let servers = parse_macos_scutil_dns(
            "DNS configuration\n\n\
             resolver #1\n\
               search domain[0] : mesh.example\n\
               nameserver[0] : 100.100.100.100\n\
               nameserver[1] : fd00::53\n\
               flags : Supplemental, Request A records, Request AAAA records\n\
               order : 100200\n\
             resolver #2\n\
               nameserver[0] : 192.0.2.53\n\
               nameserver[1] : 2001:db8::53\n\
               flags : Request A records, Request AAAA records\n\
               order : 200000\n\n\
             DNS configuration (for scoped queries)\n",
        );
        assert_eq!(
            servers,
            vec![
                "192.0.2.53:53".parse().expect("IPv4 resolver"),
                "[2001:db8::53]:53".parse().expect("IPv6 resolver"),
            ]
        );
    }

    #[tokio::test]
    async fn truncated_udp_response_retries_over_tcp_on_same_resolver() {
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind UDP fixture");
        let address = udp.local_addr().expect("fixture address");
        let tcp = tokio::net::TcpListener::bind(address)
            .await
            .expect("bind TCP fixture on same resolver address");

        let udp_task = tokio::spawn(async move {
            let mut query = [0u8; 4096];
            let (len, source) = udp.recv_from(&mut query).await.expect("UDP query");
            udp.send_to(&dns_response(&query[..len], true), source)
                .await
                .expect("truncated UDP response");
        });
        let tcp_task = tokio::spawn(async move {
            let (mut stream, _) = tcp.accept().await.expect("TCP retry");
            let query_len = stream.read_u16().await.expect("TCP query length") as usize;
            let mut query = vec![0u8; query_len];
            stream.read_exact(&mut query).await.expect("TCP DNS query");
            let response = dns_response(&query, false);
            stream
                .write_all(&(response.len() as u16).to_be_bytes())
                .await
                .expect("TCP response length");
            stream.write_all(&response).await.expect("TCP DNS response");
        });

        let resolver = HostDnsResolver::with_servers(vec![address], Duration::from_millis(500));
        let response = resolver
            .resolve(&dns_query(404), 404)
            .await
            .expect("TCP retry response");
        let response = Message::from_vec(&response).expect("DNS response");
        assert_eq!(response.id, 404);
        assert!(!response.metadata.truncation);
        udp_task.await.expect("UDP fixture task");
        tcp_task.await.expect("TCP fixture task");
    }
}
