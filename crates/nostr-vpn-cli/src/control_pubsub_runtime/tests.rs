use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

use fips_endpoint::{Config, PeerConfig, RoutingMode, TransportInstances, UdpConfig};
use nostr_sdk::prelude::{EventBuilder, EventId, Keys, Kind, Tag, TagKind, ToBech32};
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
