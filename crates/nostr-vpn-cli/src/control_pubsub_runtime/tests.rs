use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

use fips_endpoint::{Config, PeerConfig, RoutingMode, TransportInstances, UdpConfig};
use nostr_pubsub::MeshPeer;
use nostr_sdk::prelude::{EventBuilder, EventId, Keys, Kind, Tag, TagKind, Timestamp, ToBech32};
use nostr_vpn_core::config::{NostrPubsubConfig, NostrPubsubMode};
use nostr_vpn_core::updater::UpdateRef;

use super::*;

fn available_udp_ports() -> [u16; 3] {
    let sockets = (0..3)
        .map(|_| UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral UDP port"))
        .collect::<Vec<_>>();
    let ports = [
        sockets[0].local_addr().expect("Alice UDP address").port(),
        sockets[1].local_addr().expect("Bob UDP address").port(),
        sockets[2].local_addr().expect("Carol UDP address").port(),
    ];
    drop(sockets);
    ports
}

fn endpoint_config(local_port: u16, peers: &[(&str, u16)]) -> Config {
    let mut config = Config::new();
    config.node.routing.mode = RoutingMode::ReplyLearned;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some(format!("127.0.0.1:{local_port}")),
        accept_connections: Some(true),
        ..UdpConfig::default()
    });
    config.peers.extend(
        peers
            .iter()
            .map(|(npub, port)| PeerConfig::new(*npub, "udp", format!("127.0.0.1:{port}"))),
    );
    config
}

async fn endpoint(keys: &Keys, config: Config) -> Arc<FipsEndpoint> {
    Arc::new(
        FipsEndpoint::builder()
            .config(config)
            .identity_nsec(keys.secret_key().to_bech32().expect("nsec"))
            .without_system_tun()
            .bind()
            .await
            .expect("bind FIPS endpoint"),
    )
}

async fn wait_connected(endpoint: &FipsEndpoint, peer_npub: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if endpoint
                .peers()
                .await
                .unwrap_or_default()
                .iter()
                .any(|peer| peer.connected && peer.npub == peer_npub)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("FIPS peer connected");
}

async fn assert_udp_link(endpoint: &FipsEndpoint, peer_npub: &str) {
    let peers = endpoint.peers().await.expect("FIPS peer snapshot");
    let peer = peers
        .iter()
        .find(|peer| peer.npub == peer_npub && peer.connected)
        .expect("connected FIPS peer");
    assert_eq!(peer.transport_type.as_deref(), Some("udp"));
}

fn update_events(publisher: &Keys, tree_name: &str) -> UpdateEventCache {
    let reference = UpdateRef {
        npub: publisher.public_key().to_bech32().expect("publisher npub"),
        tree_name: tree_name.to_string(),
        path: Some("latest".to_string()),
    };
    UpdateEventCache::new(&reference).expect("update event cache")
}

async fn start_pubsub(
    endpoint: Arc<FipsEndpoint>,
    update_events: UpdateEventCache,
) -> ControlPubsubFipsRuntime {
    ControlPubsubFipsRuntime::start_inner(
        endpoint,
        NostrPubsubConfig {
            mode: NostrPubsubMode::Client,
            fanout: 8,
            max_hops: 4,
            max_event_bytes: CONTROL_PUBSUB_MAX_EVENT_BYTES,
        },
        Vec::new(),
        None,
        None,
        Some(update_events),
    )
    .await
    .expect("start FIPS pubsub")
    .expect("FIPS pubsub enabled")
}

async fn wait_for_event(runtime: &ControlPubsubFipsRuntime, event_id: EventId) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if runtime
                .events()
                .await
                .iter()
                .any(|event| event.id == event_id)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("signed update root arrived over FIPS");
}

#[test]
fn late_connected_fips_peer_receives_cached_update_root_without_relays() {
    std::thread::Builder::new()
        .name("late-fips-update-peer".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("local FIPS update test runtime")
                .block_on(
                    late_connected_fips_peer_receives_cached_update_root_without_relays_run(),
                );
        })
        .expect("spawn local FIPS update test")
        .join()
        .expect("local FIPS update test thread");
}

async fn late_connected_fips_peer_receives_cached_update_root_without_relays_run() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let carol = Keys::generate();
    let publisher = Keys::generate();
    let alice_npub = alice.public_key().to_bech32().expect("Alice npub");
    let bob_npub = bob.public_key().to_bech32().expect("Bob npub");
    let carol_npub = carol.public_key().to_bech32().expect("Carol npub");
    let [alice_port, bob_port, carol_port] = available_udp_ports();

    let alice_endpoint = endpoint(
        &alice,
        endpoint_config(alice_port, &[(&bob_npub, bob_port)]),
    )
    .await;
    let bob_endpoint = endpoint(
        &bob,
        endpoint_config(bob_port, &[(&alice_npub, alice_port)]),
    )
    .await;
    wait_connected(&alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;

    let tree_name = "releases/test-app";
    let update_events = update_events(&publisher, tree_name);
    let alice_pubsub = start_pubsub(Arc::clone(&alice_endpoint), update_events.clone()).await;
    let bob_pubsub = start_pubsub(Arc::clone(&bob_endpoint), update_events.clone()).await;
    let root_event = EventBuilder::new(Kind::Custom(30_064), "")
        .tags([
            Tag::identifier(tree_name),
            Tag::custom(TagKind::Custom("l".into()), ["hashtree"]),
            Tag::custom(TagKind::Custom("hash".into()), ["aa".repeat(32)]),
        ])
        .sign_with_keys(&publisher)
        .expect("signed update root");
    let root_event_id = root_event.id;
    assert!(
        alice_pubsub
            .publish(root_event)
            .await
            .expect("publish update root")
    );
    wait_for_event(&bob_pubsub, root_event_id).await;

    let carol_endpoint = endpoint(
        &carol,
        endpoint_config(carol_port, &[(&bob_npub, bob_port)]),
    )
    .await;
    let carol_pubsub = start_pubsub(Arc::clone(&carol_endpoint), update_events).await;
    bob_endpoint
        .update_peers(vec![
            PeerConfig::new(&alice_npub, "udp", format!("127.0.0.1:{alice_port}")),
            PeerConfig::new(&carol_npub, "udp", format!("127.0.0.1:{carol_port}")),
        ])
        .await
        .expect("connect Bob to Carol");
    wait_connected(&bob_endpoint, &carol_npub).await;
    wait_connected(&carol_endpoint, &bob_npub).await;

    wait_for_event(&carol_pubsub, root_event_id).await;

    alice_pubsub.stop().await;
    bob_pubsub.stop().await;
    carol_pubsub.stop().await;
    alice_endpoint.shutdown().await.expect("shutdown Alice");
    bob_endpoint.shutdown().await.expect("shutdown Bob");
    carol_endpoint.shutdown().await.expect("shutdown Carol");
}

#[test]
fn standalone_publish_replays_after_udp_roster_peer_appears() {
    std::thread::Builder::new()
        .name("standalone-fips-pubsub".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("local standalone pubsub test runtime")
                .block_on(standalone_publish_replays_after_udp_roster_peer_appears_run());
        })
        .expect("spawn standalone pubsub test")
        .join()
        .expect("standalone pubsub test thread");
}

async fn standalone_publish_replays_after_udp_roster_peer_appears_run() {
    const APPLICATION_SERVICE_PORT: u16 = 47_371;

    let alice = Keys::generate();
    let bob = Keys::generate();
    let publisher = Keys::generate();
    let alice_npub = alice.public_key().to_bech32().expect("Alice npub");
    let bob_npub = bob.public_key().to_bech32().expect("Bob npub");
    let [alice_port, bob_port, _] = available_udp_ports();
    let alice_endpoint = endpoint(&alice, endpoint_config(alice_port, &[])).await;
    let tree_name = "releases/standalone-test";
    let update_events = update_events(&publisher, tree_name);
    let alice_pubsub = start_pubsub(Arc::clone(&alice_endpoint), update_events.clone()).await;
    let root = signed_update_root(&publisher, tree_name, 1, "33");

    assert!(
        !alice_pubsub
            .publish(root.clone())
            .await
            .expect("cache standalone update root")
    );
    assert!(
        alice_pubsub
            .events()
            .await
            .iter()
            .any(|event| event.id == root.id),
        "standalone publication remains available for reconnect replay"
    );

    let bob_endpoint = endpoint(
        &bob,
        endpoint_config(bob_port, &[(&alice_npub, alice_port)]),
    )
    .await;
    let application_receiver = bob_endpoint
        .register_service_receiver(APPLICATION_SERVICE_PORT)
        .await
        .expect("register application-owned FIPS service");
    alice_endpoint
        .update_peers(vec![PeerConfig::new(
            &bob_npub,
            "udp",
            format!("127.0.0.1:{bob_port}"),
        )])
        .await
        .expect("add UDP roster peer");
    wait_connected(&alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;
    let bob_pubsub = start_pubsub(Arc::clone(&bob_endpoint), update_events).await;

    wait_for_event(&bob_pubsub, root.id).await;
    assert_udp_link(&alice_endpoint, &bob_npub).await;
    assert_udp_link(&bob_endpoint, &alice_npub).await;

    let application_payload = b"application-owned-route".to_vec();
    alice_endpoint
        .send_datagram(
            PeerIdentity::from_npub(&bob_npub).expect("Bob identity"),
            APPLICATION_SERVICE_PORT,
            APPLICATION_SERVICE_PORT,
            application_payload.clone(),
        )
        .await
        .expect("send application-owned FSP datagram");
    let mut datagrams = Vec::new();
    let count = tokio::time::timeout(
        Duration::from_secs(5),
        application_receiver.recv_batch_into(&mut datagrams, 1),
    )
    .await
    .expect("application-owned datagram arrived")
    .expect("application service remained registered");
    assert_eq!(count, 1);
    assert_eq!(datagrams[0].data.as_ref(), application_payload.as_slice());

    alice_pubsub.stop().await;
    bob_pubsub.stop().await;
    alice_endpoint.shutdown().await.expect("shutdown Alice");
    bob_endpoint.shutdown().await.expect("shutdown Bob");
}

#[test]
fn control_pubsub_preserves_56k_event_limit_over_real_udp_fips() {
    std::thread::Builder::new()
        .name("bounded-fips-pubsub".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("local bounded pubsub test runtime")
                .block_on(control_pubsub_preserves_56k_event_limit_over_real_udp_fips_run());
        })
        .expect("spawn bounded pubsub test")
        .join()
        .expect("bounded pubsub test thread");
}

async fn control_pubsub_preserves_56k_event_limit_over_real_udp_fips_run() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let publisher = Keys::generate();
    let alice_npub = alice.public_key().to_bech32().expect("Alice npub");
    let bob_npub = bob.public_key().to_bech32().expect("Bob npub");
    let [alice_port, bob_port, _] = available_udp_ports();
    let alice_endpoint = endpoint(
        &alice,
        endpoint_config(alice_port, &[(&bob_npub, bob_port)]),
    )
    .await;
    let bob_endpoint = endpoint(
        &bob,
        endpoint_config(bob_port, &[(&alice_npub, alice_port)]),
    )
    .await;
    wait_connected(&alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;

    let tree_name = "releases/bounded-test";
    let update_events = update_events(&publisher, tree_name);
    let alice_pubsub = start_pubsub(Arc::clone(&alice_endpoint), update_events.clone()).await;
    let bob_pubsub = start_pubsub(Arc::clone(&bob_endpoint), update_events).await;
    let accepted = signed_update_root_with_content(
        &publisher,
        tree_name,
        1,
        "44",
        CONTROL_PUBSUB_MAX_EVENT_BYTES - 1_024,
    );
    assert!(
        serde_json::to_vec(&accepted).expect("event JSON").len() <= CONTROL_PUBSUB_MAX_EVENT_BYTES
    );
    assert!(
        alice_pubsub
            .publish(accepted.clone())
            .await
            .expect("publish near-limit update root")
    );
    wait_for_event(&bob_pubsub, accepted.id).await;

    let oversized = signed_update_root_with_content(
        &publisher,
        tree_name,
        2,
        "55",
        CONTROL_PUBSUB_MAX_EVENT_BYTES,
    );
    assert!(
        serde_json::to_vec(&oversized).expect("event JSON").len() > CONTROL_PUBSUB_MAX_EVENT_BYTES
    );
    let error = alice_pubsub
        .publish(oversized.clone())
        .await
        .expect_err("reject oversized update root");
    assert!(error.to_string().contains("maximum"));
    assert!(
        !alice_pubsub
            .events()
            .await
            .iter()
            .any(|event| event.id == oversized.id)
    );

    alice_pubsub.stop().await;
    bob_pubsub.stop().await;
    alice_endpoint.shutdown().await.expect("shutdown Alice");
    bob_endpoint.shutdown().await.expect("shutdown Bob");
}

struct RejectAllPeers;

impl MeshPeerPolicy for RejectAllPeers {
    fn select_mesh_peer(&self, _peer_id: &str) -> nostr_pubsub::Result<Option<MeshPeer>> {
        Ok(None)
    }
}

#[test]
fn peer_policy_rejects_events_from_authenticated_udp_fips_peer() {
    std::thread::Builder::new()
        .name("policy-fips-pubsub".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("local policy pubsub test runtime")
                .block_on(peer_policy_rejects_events_from_authenticated_udp_fips_peer_run());
        })
        .expect("spawn policy pubsub test")
        .join()
        .expect("policy pubsub test thread");
}

async fn peer_policy_rejects_events_from_authenticated_udp_fips_peer_run() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let publisher = Keys::generate();
    let alice_npub = alice.public_key().to_bech32().expect("Alice npub");
    let bob_npub = bob.public_key().to_bech32().expect("Bob npub");
    let [alice_port, bob_port, _] = available_udp_ports();
    let alice_endpoint = endpoint(
        &alice,
        endpoint_config(alice_port, &[(&bob_npub, bob_port)]),
    )
    .await;
    let bob_endpoint = endpoint(
        &bob,
        endpoint_config(bob_port, &[(&alice_npub, alice_port)]),
    )
    .await;
    wait_connected(&alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;

    let tree_name = "releases/policy-test";
    let update_events = update_events(&publisher, tree_name);
    let alice_pubsub = start_pubsub(Arc::clone(&alice_endpoint), update_events.clone()).await;
    let bob_pubsub = ControlPubsubFipsRuntime::start_inner(
        Arc::clone(&bob_endpoint),
        NostrPubsubConfig {
            mode: NostrPubsubMode::Client,
            fanout: 8,
            max_hops: 4,
            max_event_bytes: CONTROL_PUBSUB_MAX_EVENT_BYTES,
        },
        Vec::new(),
        None,
        Some(Arc::new(RejectAllPeers)),
        Some(update_events),
    )
    .await
    .expect("start policy-bound FIPS pubsub")
    .expect("FIPS pubsub enabled");
    let root = signed_update_root(&publisher, tree_name, 1, "66");
    assert!(
        alice_pubsub
            .publish(root.clone())
            .await
            .expect("publish policy test event")
    );
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert!(
        !bob_pubsub
            .events()
            .await
            .iter()
            .any(|event| event.id == root.id),
        "peer policy must reject stream records after FIPS authentication"
    );

    alice_pubsub.stop().await;
    bob_pubsub.stop().await;
    alice_endpoint.shutdown().await.expect("shutdown Alice");
    bob_endpoint.shutdown().await.expect("shutdown Bob");
}

#[test]
fn restarted_pubsub_resubscribes_after_tcp_fips_connection_loss() {
    std::thread::Builder::new()
        .name("restarted-fips-pubsub".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("local FIPS reconnect test runtime")
                .block_on(restarted_pubsub_resubscribes_after_connection_loss_run());
        })
        .expect("spawn local FIPS reconnect test")
        .join()
        .expect("local FIPS reconnect test thread");
}

async fn restarted_pubsub_resubscribes_after_connection_loss_run() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let publisher = Keys::generate();
    let alice_npub = alice.public_key().to_bech32().expect("Alice npub");
    let bob_npub = bob.public_key().to_bech32().expect("Bob npub");
    let [alice_port, bob_port, _] = available_udp_ports();
    let alice_config = endpoint_config(alice_port, &[(&bob_npub, bob_port)]);
    let alice_endpoint = endpoint(&alice, alice_config.clone()).await;
    let bob_endpoint = endpoint(
        &bob,
        endpoint_config(bob_port, &[(&alice_npub, alice_port)]),
    )
    .await;
    wait_connected(&alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;

    let tree_name = "releases/reconnect-test";
    let update_events = update_events(&publisher, tree_name);
    let alice_pubsub = start_pubsub(Arc::clone(&alice_endpoint), update_events.clone()).await;
    let bob_pubsub = start_pubsub(Arc::clone(&bob_endpoint), update_events.clone()).await;
    let first = signed_update_root(&publisher, tree_name, 1, "11");
    assert!(
        bob_pubsub
            .publish(first.clone())
            .await
            .expect("publish first update root")
    );
    wait_for_event(&alice_pubsub, first.id).await;

    alice_pubsub.stop().await;
    alice_endpoint
        .shutdown()
        .await
        .expect("stop Alice endpoint");
    drop(alice_endpoint);
    let restarted_alice_endpoint = endpoint(&alice, alice_config).await;
    wait_connected(&restarted_alice_endpoint, &bob_npub).await;
    wait_connected(&bob_endpoint, &alice_npub).await;
    let restarted_alice = start_pubsub(Arc::clone(&restarted_alice_endpoint), update_events).await;
    let second = signed_update_root(&publisher, tree_name, 2, "22");
    assert!(
        bob_pubsub
            .publish(second.clone())
            .await
            .expect("publish across stale TCP/FIPS session")
    );
    wait_for_event(&restarted_alice, second.id).await;

    bob_pubsub.stop().await;
    restarted_alice.stop().await;
    restarted_alice_endpoint
        .shutdown()
        .await
        .expect("shutdown restarted Alice");
    bob_endpoint.shutdown().await.expect("shutdown Bob");
}

fn signed_update_root(
    publisher: &Keys,
    tree_name: &str,
    created_at: u64,
    hash_byte: &str,
) -> Event {
    EventBuilder::new(Kind::Custom(30_064), "")
        .tags([
            Tag::identifier(tree_name),
            Tag::custom(TagKind::Custom("l".into()), ["hashtree"]),
            Tag::custom(TagKind::Custom("hash".into()), [hash_byte.repeat(32)]),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(publisher)
        .expect("signed update root")
}

fn signed_update_root_with_content(
    publisher: &Keys,
    tree_name: &str,
    created_at: u64,
    hash_byte: &str,
    content_bytes: usize,
) -> Event {
    EventBuilder::new(Kind::Custom(30_064), "x".repeat(content_bytes))
        .tags([
            Tag::identifier(tree_name),
            Tag::custom(TagKind::Custom("l".into()), ["hashtree"]),
            Tag::custom(TagKind::Custom("hash".into()), [hash_byte.repeat(32)]),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(publisher)
        .expect("signed update root")
}
