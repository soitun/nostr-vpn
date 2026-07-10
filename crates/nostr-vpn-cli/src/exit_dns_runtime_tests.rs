use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fips_core::{FipsEndpoint, PeerIdentity};
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RecordType};
use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

use super::*;

fn dns_query(id: u16) -> Vec<u8> {
    let mut message = Message::new(id, MessageType::Query, OpCode::Query);
    message.add_query(Query::query(
        Name::from_ascii("fixture.example.").expect("DNS name"),
        RecordType::A,
    ));
    let mut bytes = Vec::new();
    message
        .emit(&mut BinEncoder::new(&mut bytes))
        .expect("DNS query");
    bytes
}

fn response_code(packet: &[u8]) -> ResponseCode {
    Message::from_vec(packet)
        .expect("DNS response")
        .metadata
        .response_code
}

#[derive(Clone, Copy)]
enum FixtureMode {
    Respond,
    DropAll,
    DropTransaction(u16),
}

async fn dns_fixture(mode: FixtureMode) -> (SocketAddr, Arc<AtomicUsize>, JoinHandle<()>) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind DNS fixture");
    let address = socket.local_addr().expect("DNS fixture address");
    let requests = Arc::new(AtomicUsize::new(0));
    let task_requests = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            let Ok((len, source)) = socket.recv_from(&mut buf).await else {
                return;
            };
            task_requests.fetch_add(1, Ordering::Relaxed);
            let Ok(request) = Message::from_vec(&buf[..len]) else {
                continue;
            };
            if matches!(mode, FixtureMode::DropAll)
                || matches!(mode, FixtureMode::DropTransaction(id) if id == request.id)
            {
                continue;
            }
            let mut response = Message::new(request.id, MessageType::Response, OpCode::Query);
            response.metadata.recursion_desired = request.recursion_desired;
            response.metadata.recursion_available = true;
            response.metadata.response_code = ResponseCode::NoError;
            for query in request.queries {
                response.add_query(query);
            }
            let mut bytes = Vec::new();
            if response.emit(&mut BinEncoder::new(&mut bytes)).is_ok() {
                let _ = socket.send_to(&bytes, source).await;
            }
        }
    });
    (address, requests, task)
}

#[tokio::test]
async fn exit_dns_forwards_only_for_enabled_active_roster_peer() {
    let (resolver_address, requests, fixture) = dns_fixture(FixtureMode::Respond).await;
    let resolver =
        HostDnsResolver::with_servers(vec![resolver_address], Duration::from_millis(100));
    let policy = ExitDnsServicePolicy::default();
    let allowed = "npub1-active-roster-peer".to_string();
    policy.reconfigure(true, std::slice::from_ref(&allowed));

    let response = answer_exit_dns_request(
        &allowed,
        ExitDnsRequest::new(1, dns_query(10)).expect("request"),
        &policy,
        Some(&resolver),
    )
    .await;
    assert_eq!(response.nonce, 1);
    assert_eq!(response.transaction_id, 10);
    assert_eq!(response_code(&response.response), ResponseCode::NoError);
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    let rejected = answer_exit_dns_request(
        "npub1-wrong-peer",
        ExitDnsRequest::new(2, dns_query(11)).expect("request"),
        &policy,
        Some(&resolver),
    )
    .await;
    assert_eq!(response_code(&rejected.response), ResponseCode::ServFail);
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    policy.reconfigure(false, std::slice::from_ref(&allowed));
    let disabled = answer_exit_dns_request(
        &allowed,
        ExitDnsRequest::new(3, dns_query(12)).expect("request"),
        &policy,
        Some(&resolver),
    )
    .await;
    assert_eq!(response_code(&disabled.response), ResponseCode::ServFail);
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    fixture.abort();
}

#[tokio::test]
async fn exit_dns_timeout_returns_correlated_servfail() {
    let (resolver_address, requests, fixture) = dns_fixture(FixtureMode::DropAll).await;
    let resolver = HostDnsResolver::with_servers(vec![resolver_address], Duration::from_millis(40));
    let policy = ExitDnsServicePolicy::default();
    let allowed = "npub1-active-roster-peer".to_string();
    policy.reconfigure(true, std::slice::from_ref(&allowed));

    let response = answer_exit_dns_request(
        &allowed,
        ExitDnsRequest::new(0xfeed, dns_query(0xbeef)).expect("request"),
        &policy,
        Some(&resolver),
    )
    .await;
    assert_eq!(response.nonce, 0xfeed);
    assert_eq!(response.transaction_id, 0xbeef);
    assert_eq!(response_code(&response.response), ResponseCode::ServFail);
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    fixture.abort();
}

#[tokio::test]
async fn fips_service_loopback_correlates_exit_dns_response() {
    let (resolver_address, requests, fixture) = dns_fixture(FixtureMode::Respond).await;
    let endpoint = Arc::new(
        FipsEndpoint::builder()
            .without_system_tun()
            .bind()
            .await
            .expect("bind FIPS endpoint"),
    );
    let endpoint_npub = endpoint.npub().to_string();
    let runtime = ExitDnsFipsRuntime::start_for_test(
        Arc::clone(&endpoint),
        Some(HostDnsResolver::with_servers(
            vec![resolver_address],
            Duration::from_millis(100),
        )),
        true,
        std::slice::from_ref(&endpoint_npub),
        Duration::from_millis(500),
    )
    .await
    .expect("start exit DNS runtime");
    let exit_peer = PeerIdentity::from_npub(&endpoint_npub).expect("local FIPS identity");

    let response = runtime
        .client()
        .resolve(exit_peer, &dns_query(77))
        .await
        .expect("FIPS exit DNS response");
    assert_eq!(Message::from_vec(&response).expect("DNS response").id, 77);
    assert_eq!(response_code(&response), ResponseCode::NoError);
    assert_eq!(requests.load(Ordering::Relaxed), 1);

    runtime.stop().await;
    endpoint.shutdown().await.expect("shutdown FIPS endpoint");
    fixture.abort();
}

#[tokio::test]
async fn timed_out_upstream_does_not_block_another_fips_request() {
    let (resolver_address, requests, fixture) =
        dns_fixture(FixtureMode::DropTransaction(100)).await;
    let endpoint = Arc::new(
        FipsEndpoint::builder()
            .without_system_tun()
            .bind()
            .await
            .expect("bind FIPS endpoint"),
    );
    let endpoint_npub = endpoint.npub().to_string();
    let runtime = ExitDnsFipsRuntime::start_for_test(
        Arc::clone(&endpoint),
        Some(HostDnsResolver::with_servers(
            vec![resolver_address],
            Duration::from_millis(150),
        )),
        true,
        std::slice::from_ref(&endpoint_npub),
        Duration::from_millis(500),
    )
    .await
    .expect("start exit DNS runtime");
    let peer = PeerIdentity::from_npub(&endpoint_npub).expect("local FIPS identity");
    let slow_client = runtime.client();
    let fast_client = runtime.client();

    let slow_query = dns_query(100);
    let slow = slow_client.resolve(peer, &slow_query);
    let fast = async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::time::timeout(
            Duration::from_millis(100),
            fast_client.resolve(peer, &dns_query(101)),
        )
        .await
        .expect("second request should not wait for first timeout")
        .expect("second exit DNS response")
    };
    let (slow, fast) = tokio::join!(slow, fast);
    assert_eq!(
        response_code(&slow.expect("timed-out request SERVFAIL")),
        ResponseCode::ServFail
    );
    assert_eq!(response_code(&fast), ResponseCode::NoError);
    assert_eq!(requests.load(Ordering::Relaxed), 2);

    runtime.stop().await;
    endpoint.shutdown().await.expect("shutdown FIPS endpoint");
    fixture.abort();
}
