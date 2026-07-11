use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_pubsub::{FipsPubsubWireCodec, FipsPubsubWireMessage, VerifiedEvent};
use nostr_sdk::prelude::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE, NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE,
    NostrIdentityDeviceApprovalSidecarRequest, NostrIdentityId, NostrVpnJoinApprovalContextRequest,
    build_device_approval_sidecar, build_nostr_vpn_join_approval_context_event,
};
use nostr_vpn_core::join_pubsub::{
    NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT, NostrJoinFipsPubsubClient, NostrJoinFipsPubsubDatagram,
};
use nostr_vpn_core::join_requests::MAX_NOSTR_JOIN_APPROVAL_AGE_SECS;

const REQUESTED_AT: u64 = 1_778_998_000;
const APPROVED_AT: u64 = REQUESTED_AT + 30;

fn unique_temp_config_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "nostr-vpn-{name}-{}-{}.toml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ))
}

fn pending_joiner() -> AppConfig {
    let mut config = AppConfig::generated_without_networks();
    config.node_name = "Pixel 10 Pro Device".to_string();
    config
        .ensure_pending_nostr_join_request(REQUESTED_AT)
        .expect("create pending join request");
    config
}

fn approval_events(
    joiner: &AppConfig,
    admin: &Keys,
    request_secret: &str,
    profile_id: NostrIdentityId,
) -> Vec<Event> {
    let pending = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending join request");
    let joiner_pubkey = joiner.own_nostr_pubkey_hex().expect("joiner pubkey");
    let admin_pubkey = admin.public_key().to_hex();
    let sidecar = build_device_approval_sidecar(
        admin,
        NostrIdentityDeviceApprovalSidecarRequest {
            profile_id,
            network_name: Some("Home Mesh".to_string()),
            request_pubkey: pending.request.request_pubkey.clone(),
            device_app_key_pubkey: joiner_pubkey.clone(),
            request_secret: request_secret.to_string(),
            canonical_profile_is_fresh: true,
            approved_at: APPROVED_AT,
        },
    )
    .expect("build approval sidecar");
    let approved_device_op = sidecar
        .approved_device_roster_op()
        .expect("fresh profile member op");
    let signed_roster = SignedRoster::sign(
        "8d4f34f5425bc50e",
        NetworkRoster {
            network_name: "Home Mesh".to_string(),
            devices: vec![admin_pubkey.clone(), joiner_pubkey],
            admins: vec![admin_pubkey.clone()],
            aliases: HashMap::from([(admin_pubkey, "home-exit".to_string())]),
            signed_at: APPROVED_AT,
        },
        admin,
    )
    .expect("sign network roster");
    let context = build_nostr_vpn_join_approval_context_event(
        admin,
        NostrVpnJoinApprovalContextRequest {
            profile_id,
            request_pubkey: pending.request.request_pubkey.clone(),
            device_app_key_pubkey: pending.request.device_app_key_pubkey.clone(),
            request_secret: request_secret.to_string(),
            mesh_network_id: "8d4f34f5425bc50e".to_string(),
            network_name: Some("Home Mesh".to_string()),
            roster_op_id: Some(approved_device_op.id.to_hex()),
            canonical_roster_events: sidecar
                .canonical_roster_events
                .iter()
                .map(JsonUtil::as_json)
                .collect(),
            signed_network_roster_event: signed_roster.event.as_json(),
            exit_node_pubkey: Some(admin.public_key().to_hex()),
            approved_at: APPROVED_AT,
        },
    )
    .expect("build approval context");

    let mut events = sidecar.canonical_roster_events;
    events.push(signed_roster.event);
    events.push(sidecar.receipt_event);
    events.push(context);
    events
}

fn delivered_datagram(
    client: &NostrJoinFipsPubsubClient,
    event: Event,
) -> NostrJoinFipsPubsubDatagram {
    let payload = FipsPubsubWireCodec::default()
        .encode_frame(&FipsPubsubWireMessage::deliver(
            client.subscription_id().clone(),
            VerifiedEvent::try_from(event).expect("verified delivered event"),
        ))
        .expect("delivered event frame");
    NostrJoinFipsPubsubDatagram {
        source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        payload,
    }
}

fn bogus_targeted_event(request_pubkey: &str, event_type: &str, signer: &Keys) -> Event {
    let profile_id = NostrIdentityId::new_v4().to_string();
    EventBuilder::new(Kind::Custom(7_368), "not encrypted for the request key")
        .tag(Tag::parse(["type", event_type]).expect("type tag"))
        .tag(Tag::parse(["p", request_pubkey]).expect("request tag"))
        .tag(Tag::parse(["i", profile_id.as_str(), "subject"]).expect("subject tag"))
        .custom_created_at(Timestamp::from(APPROVED_AT))
        .sign_with_keys(signer)
        .expect("signed bogus targeted event")
}

#[test]
fn pending_join_request_is_secure_and_stable_across_reload() {
    let path = unique_temp_config_path("pending-join-request");
    let config = pending_joiner();
    let first_link = config
        .pending_nostr_join_request_link("nvpn://join-request/")
        .expect("encode first link");
    let pending = config
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .clone();
    assert_eq!(pending.request.label.as_deref(), Some("Pixel 10 Pro Dev"));

    config.save(&path).expect("save config");
    let raw = fs::read_to_string(&path).expect("read persisted config");
    assert!(!raw.contains(&pending.request.request_secret));
    assert!(!raw.contains(&pending.request_private_key));

    let mut reloaded = AppConfig::load(&path).expect("reload config");
    let second_link = reloaded
        .pending_nostr_join_request_link("nvpn://join-request/")
        .expect("encode reloaded link");
    assert_eq!(second_link, first_link);
    assert_eq!(
        reloaded
            .pending_nostr_join_request
            .as_ref()
            .expect("reloaded pending request"),
        &pending
    );
    assert!(
        !reloaded
            .ensure_pending_nostr_join_request(REQUESTED_AT + 100)
            .expect("reuse pending request")
    );
    assert_eq!(
        reloaded
            .pending_nostr_join_request_link("nvpn://join-request/")
            .expect("encode stable link"),
        first_link
    );

    AppConfig::delete_persisted_secrets_for_path(&path).expect("delete test secrets");
    let _ = fs::remove_file(path);
}

#[test]
fn approval_batch_is_auto_detected_applied_and_clears_pending_request() {
    let mut joiner = pending_joiner();
    let request_secret = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .request
        .request_secret
        .clone();
    let profile_id = NostrIdentityId::new_v4();
    let admin = Keys::generate();
    let unrelated = nostr_sdk::EventBuilder::text_note("unrelated")
        .sign_with_keys(&admin)
        .expect("unrelated event");
    let mut events = vec![unrelated];
    events.extend(approval_events(
        &joiner,
        &admin,
        &request_secret,
        profile_id,
    ));

    let applied = joiner
        .apply_nostr_join_approval_events(&events, APPROVED_AT + 1)
        .expect("apply approval")
        .expect("approval detected");

    assert_eq!(applied.profile_id, profile_id);
    assert_eq!(applied.network_id, "8d4f34f5425bc50e");
    assert_eq!(applied.approved_by_pubkey, admin.public_key().to_hex());
    assert!(joiner.pending_nostr_join_request.is_none());
    assert_eq!(joiner.nostr.identity_profile_id, Some(profile_id));
    let network = joiner.active_network();
    assert_eq!(network.name, "Home Mesh");
    assert_eq!(network.network_id, "8d4f34f5425bc50e");
    assert_eq!(network.admins, vec![admin.public_key().to_hex()]);
    assert_eq!(joiner.exit_node, admin.public_key().to_hex());
}

#[test]
fn fips_pubsub_sends_only_receipt_subscription_and_auto_applies_approval() {
    let mut joiner = pending_joiner();
    let pending = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .clone();
    let mut client = NostrJoinFipsPubsubClient::new(&joiner).expect("pubsub client");
    let subscribe = client
        .subscribe_datagram(&joiner)
        .expect("subscription datagram");
    assert_eq!(subscribe.source_port, NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT);
    assert_eq!(
        subscribe.destination_port,
        NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
    );
    assert!(subscribe.payload.len() <= client.max_frame_bytes());
    let subscribe_json = String::from_utf8(subscribe.payload.clone()).expect("REQ utf8");
    assert!(subscribe_json.contains("\"REQ\""));
    assert!(subscribe_json.contains(&pending.request.request_pubkey));
    assert!(!subscribe_json.contains(&pending.request.request_secret));
    assert!(!subscribe_json.contains(&pending.request_private_key));
    let subscribe_value: serde_json::Value =
        serde_json::from_str(&subscribe_json).expect("REQ json");
    assert_eq!(subscribe_value[0], "REQ");
    let filter = &subscribe_value[2];
    assert_eq!(filter["#p"][0], pending.request.request_pubkey);
    assert!(filter.get("authors").is_none());
    let close = client.close_datagram().expect("close datagram");
    assert!(close.payload.len() <= client.max_frame_bytes());
    assert!(
        String::from_utf8(close.payload)
            .expect("CLOSE utf8")
            .contains("\"CLOSE\"")
    );

    let admin = Keys::generate();
    let profile_id = NostrIdentityId::new_v4();
    let events = approval_events(&joiner, &admin, &pending.request.request_secret, profile_id);
    let codec = FipsPubsubWireCodec::default();
    let targeted = &events[events.len() - 2..];
    for event in targeted {
        assert!(!event.content.contains(&pending.request.request_secret));
        assert!(!event.tags.iter().any(|tag| {
            tag.as_slice()
                .iter()
                .any(|value| value == &pending.request.request_secret)
        }));
    }
    let first_frame = codec
        .encode_frame(&FipsPubsubWireMessage::deliver(
            client.subscription_id().clone(),
            VerifiedEvent::try_from(targeted[0].clone()).expect("verified receipt"),
        ))
        .expect("receipt frame");
    assert!(
        client
            .ingest_datagram(
                &mut joiner,
                &NostrJoinFipsPubsubDatagram {
                    source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                    payload: first_frame,
                },
                APPROVED_AT + 1,
            )
            .expect("ingest receipt")
            .is_none()
    );
    let second_frame = codec
        .encode_frame(&FipsPubsubWireMessage::deliver(
            client.subscription_id().clone(),
            VerifiedEvent::try_from(targeted[1].clone()).expect("verified context"),
        ))
        .expect("context frame");
    let applied = client
        .ingest_datagram(
            &mut joiner,
            &NostrJoinFipsPubsubDatagram {
                source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
                payload: second_frame,
            },
            APPROVED_AT + 1,
        )
        .expect("ingest context")
        .expect("approval applied");
    assert_eq!(applied.profile_id, profile_id);
    assert!(joiner.pending_nostr_join_request.is_none());
}

#[test]
fn fips_pubsub_ignores_targeted_bogus_events_without_poisoning_valid_pair() {
    let mut joiner = pending_joiner();
    let pending = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .clone();
    let mut client = NostrJoinFipsPubsubClient::new(&joiner).expect("pubsub client");
    let attacker = Keys::generate();

    for index in 0..8 {
        let event_type = if index % 2 == 0 {
            NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE
        } else {
            NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE
        };
        let datagram = delivered_datagram(
            &client,
            bogus_targeted_event(&pending.request.request_pubkey, event_type, &attacker),
        );
        assert!(
            client
                .ingest_datagram(&mut joiner, &datagram, APPROVED_AT + 1)
                .expect("bogus targeted event is ignored")
                .is_none()
        );
        assert!(joiner.pending_nostr_join_request.is_some());
    }

    let admin = Keys::generate();
    let profile_id = NostrIdentityId::new_v4();
    let events = approval_events(&joiner, &admin, &pending.request.request_secret, profile_id);
    let targeted = &events[events.len() - 2..];
    let receipt_datagram = delivered_datagram(&client, targeted[0].clone());
    assert!(
        client
            .ingest_datagram(&mut joiner, &receipt_datagram, APPROVED_AT + 1)
            .expect("valid receipt")
            .is_none()
    );
    let context_datagram = delivered_datagram(&client, targeted[1].clone());
    let applied = client
        .ingest_datagram(&mut joiner, &context_datagram, APPROVED_AT + 1)
        .expect("valid context")
        .expect("valid approval pair applies");

    assert_eq!(applied.profile_id, profile_id);
    assert!(joiner.pending_nostr_join_request.is_none());
}

#[test]
fn fips_pubsub_endpoint_rejects_oversized_and_wrong_port_frames() {
    let mut joiner = pending_joiner();
    let mut client = NostrJoinFipsPubsubClient::new(&joiner).expect("pubsub client");
    let oversized = NostrJoinFipsPubsubDatagram {
        source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        payload: vec![b' '; client.max_frame_bytes() + 1],
    };
    assert!(
        client
            .ingest_datagram(&mut joiner, &oversized, REQUESTED_AT)
            .expect_err("oversized frame must fail")
            .to_string()
            .contains("limit")
    );

    let mut wrong_port = client
        .subscribe_datagram(&joiner)
        .expect("subscription datagram");
    wrong_port.destination_port += 1;
    assert!(
        client
            .ingest_datagram(&mut joiner, &wrong_port, REQUESTED_AT)
            .expect_err("wrong port must fail")
            .to_string()
            .contains("service port")
    );
}

#[test]
fn approval_batch_ignores_wrong_secret_spoofed_signer_and_tampering() {
    let joiner = pending_joiner();
    let request_secret = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .request
        .request_secret
        .clone();
    let profile_id = NostrIdentityId::new_v4();
    let admin = Keys::generate();

    let wrong_request_secret = URL_SAFE_NO_PAD.encode([99_u8; 32]);
    let wrong_secret = approval_events(&joiner, &admin, &wrong_request_secret, profile_id);
    let mut candidate = joiner.clone();
    assert!(
        candidate
            .apply_nostr_join_approval_events(&wrong_secret, APPROVED_AT + 1)
            .expect("wrong secret is ignored")
            .is_none()
    );
    assert!(candidate.pending_nostr_join_request.is_some());

    let mut spoofed = approval_events(&joiner, &admin, &request_secret, profile_id);
    let attacker = Keys::generate();
    let receipt_index = spoofed.len() - 2;
    let attacker_receipt = approval_events(&joiner, &attacker, &request_secret, profile_id);
    spoofed[receipt_index] = attacker_receipt[attacker_receipt.len() - 2].clone();
    let mut candidate = joiner.clone();
    assert!(
        candidate
            .apply_nostr_join_approval_events(&spoofed, APPROVED_AT + 1)
            .expect("spoofed signer is ignored")
            .is_none()
    );
    assert!(candidate.pending_nostr_join_request.is_some());

    let mut tampered = approval_events(&joiner, &admin, &request_secret, profile_id);
    let context_index = tampered.len() - 1;
    let mut value: serde_json::Value =
        serde_json::from_str(&tampered[context_index].as_json()).expect("event json");
    value["content"] = serde_json::Value::String("tampered".to_string());
    tampered[context_index] =
        Event::from_json(serde_json::to_string(&value).expect("tampered json"))
            .expect("parse tampered event");
    let mut candidate = joiner.clone();
    assert!(
        candidate
            .apply_nostr_join_approval_events(&tampered, APPROVED_AT + 1)
            .expect("tampered event is ignored")
            .is_none()
    );
    assert!(candidate.pending_nostr_join_request.is_some());
}

#[test]
fn approval_batch_ignores_profile_mismatch_and_stale_receipt() {
    let joiner = pending_joiner();
    let request_secret = joiner
        .pending_nostr_join_request
        .as_ref()
        .expect("pending request")
        .request
        .request_secret
        .clone();
    let admin = Keys::generate();
    let profile_id = NostrIdentityId::new_v4();
    let other_profile_id = NostrIdentityId::new_v4();

    let mut mismatched = approval_events(&joiner, &admin, &request_secret, profile_id);
    let other = approval_events(&joiner, &admin, &request_secret, other_profile_id);
    let context_index = mismatched.len() - 1;
    mismatched[context_index] = other[other.len() - 1].clone();
    let mut candidate = joiner.clone();
    assert!(
        candidate
            .apply_nostr_join_approval_events(&mismatched, APPROVED_AT + 1)
            .expect("profile mismatch is ignored")
            .is_none()
    );
    assert!(candidate.pending_nostr_join_request.is_some());

    let stale = approval_events(&joiner, &admin, &request_secret, profile_id);
    let mut candidate = joiner.clone();
    assert!(
        candidate
            .apply_nostr_join_approval_events(
                &stale,
                APPROVED_AT + MAX_NOSTR_JOIN_APPROVAL_AGE_SECS + 1,
            )
            .expect("stale approval is ignored")
            .is_none()
    );
    assert!(candidate.pending_nostr_join_request.is_some());
}
