use std::collections::{HashMap, VecDeque};

use nostr_pubsub::{InvWantAction, InvWantCodec, InvWantMesh, InvWantWireMessage, MeshPeer};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind};
use nostr_vpn_core::control_pubsub::{
    CONTROL_PUBSUB_MAX_WIRE_BYTES, CONTROL_PUBSUB_PROTOCOL, CONTROL_PUBSUB_VERSION,
    ControlPubsubOptions,
};

fn signed_event(kind: u16, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .sign_with_keys(&Keys::generate())
        .expect("signed event")
}

fn peer(id: &str) -> MeshPeer {
    MeshPeer::new(id)
}

#[test]
fn three_node_line_delivers_after_relay_bootstrap_is_gone() {
    let options = ControlPubsubOptions {
        fanout: 8,
        max_hops: 4,
        ..ControlPubsubOptions::default()
    };
    let mut nodes = HashMap::from([
        (
            "a".to_string(),
            InvWantMesh::new(options.clone().into_mesh_options()),
        ),
        (
            "b".to_string(),
            InvWantMesh::new(options.clone().into_mesh_options()),
        ),
        (
            "c".to_string(),
            InvWantMesh::new(options.into_mesh_options()),
        ),
    ]);
    let peers = HashMap::from([
        ("a".to_string(), vec![peer("b")]),
        ("b".to_string(), vec![peer("a"), peer("c")]),
        ("c".to_string(), vec![peer("b")]),
    ]);
    let event = signed_event(37_196, "paid exit offer");
    let event_id = event.id.to_hex();
    let mut queue = VecDeque::new();
    let mut delivered = Vec::new();

    for action in nodes
        .get_mut("c")
        .expect("publisher")
        .publish(
            event.clone(),
            peers.get("c").expect("publisher peers"),
            1_000,
        )
        .expect("publish")
    {
        queue.push_back(("c".to_string(), action));
    }

    while let Some((sender, action)) = queue.pop_front() {
        match action {
            InvWantAction::Send { peer_id, message } => {
                let next_actions = nodes
                    .get_mut(&peer_id)
                    .expect("recipient")
                    .receive(
                        &sender,
                        message,
                        peers.get(&peer_id).expect("recipient peers"),
                        1_001,
                    )
                    .expect("receive");
                for next in next_actions {
                    queue.push_back((peer_id.clone(), next));
                }
            }
            InvWantAction::Deliver { event, .. } => {
                delivered.push((sender, event.id.to_hex()));
            }
        }
    }

    assert_eq!(
        delivered
            .iter()
            .filter(|(node, id)| node == "a" && id == &event_id)
            .count(),
        1
    );

    let duplicate = nodes
        .get_mut("a")
        .expect("subscriber")
        .receive(
            "b",
            InvWantWireMessage::Frame {
                event_id,
                event: Box::new(event),
            },
            peers.get("a").expect("subscriber peers"),
            1_002,
        )
        .expect("duplicate frame");
    assert!(
        duplicate
            .iter()
            .all(|action| !matches!(action, InvWantAction::Deliver { .. }))
    );
}

#[test]
fn control_pubsub_rejects_non_control_event_kinds() {
    let mut mesh = InvWantMesh::new(ControlPubsubOptions::default().into_mesh_options());
    let error = mesh
        .publish(signed_event(1, "ordinary note"), &[peer("peer")], 1_000)
        .expect_err("ordinary notes must not enter the nvpn control stream");
    assert!(error.to_string().contains("unsupported Nostr event kind 1"));
}

#[test]
fn control_pubsub_accepts_additional_subscription_kinds() {
    let peers = [peer("peer")];
    for kind in [30_064, 30_078] {
        let mut options = ControlPubsubOptions::default();
        options.allowed_kinds.insert(kind);
        let mut mesh = InvWantMesh::new(options.into_mesh_options());
        assert_eq!(
            mesh.publish(signed_event(kind, "subscribed event"), &peers, 1_000)
                .expect("additional subscription belongs on the control mesh")
                .len(),
            1
        );
    }
}

#[test]
fn relay_echo_does_not_reannounce_an_event() {
    let mut mesh = InvWantMesh::new(ControlPubsubOptions::default().into_mesh_options());
    let event = signed_event(37_195, "peer advert");
    let peers = [peer("peer")];

    assert_eq!(
        mesh.publish(event.clone(), &peers, 1_000)
            .expect("first relay ingress")
            .len(),
        1
    );
    assert!(
        mesh.publish(event, &peers, 1_001)
            .expect("echoed relay ingress")
            .is_empty()
    );
}

#[test]
fn control_pubsub_codec_is_versioned_and_bounded() {
    let codec = InvWantCodec::new(
        CONTROL_PUBSUB_PROTOCOL,
        CONTROL_PUBSUB_VERSION,
        CONTROL_PUBSUB_MAX_WIRE_BYTES,
    );
    let message = InvWantWireMessage::Inventory {
        event_id: "11".repeat(32),
        event_kind: 37_195,
        payload_bytes: 512,
        hop_limit: 4,
    };
    let encoded = codec.encode(&message).expect("encoded inventory");
    assert_eq!(codec.decode(&encoded).expect("decoded inventory"), message);

    let mut value: serde_json::Value = serde_json::from_slice(&encoded).expect("wire JSON");
    assert_eq!(value["protocol"], CONTROL_PUBSUB_PROTOCOL);
    value["version"] = serde_json::json!(2);
    let unsupported = serde_json::to_vec(&value).expect("unsupported version JSON");
    assert!(
        codec
            .decode(&unsupported)
            .expect_err("unsupported version must fail")
            .to_string()
            .contains("unsupported inv/want version 2")
    );

    let tiny = InvWantCodec::new(
        CONTROL_PUBSUB_PROTOCOL,
        CONTROL_PUBSUB_VERSION,
        encoded.len() - 1,
    );
    assert!(tiny.encode(&message).is_err());
}
