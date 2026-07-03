use std::collections::{BTreeMap, HashMap};

use nostr_sdk::ToBech32;
use nostr_sdk::prelude::Keys;
use nostr_sdk::prelude::{Event, JsonUtil};
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    CANONICAL_NETWORK_NAME_FACT, CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND,
    CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE, NostrIdentityCapabilities,
    NostrIdentityDeviceApprovalSidecarRequest, NostrIdentityId, NostrIdentityKeyPurpose,
    NostrIdentityRosterOp, RosterAppKeyRole, RosterIdentityBridgeSource,
    build_device_approval_for_link_request, build_device_approval_sidecar,
    build_identity_link_request_from_manual_npub, build_roster_app_key_sidecar_event,
    build_roster_app_key_sidecar_event_with_network_name,
    parse_identity_link_request_event_for_invite_pubkey, parse_identity_roster_bridge_event,
    parse_nostr_identity_device_approval_receipt_event,
    parse_nostr_identity_device_approval_receipt_roster_op, parse_roster_app_key_sidecar_event,
    roster_app_key_identities, signed_roster_app_key_identities,
};
use uuid::Uuid;

#[test]
fn bridge_represents_roster_members_as_canonical_app_keys() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let admin_hex = admin.public_key().to_hex();
    let member_hex = member.public_key().to_hex();
    let member_profile = NostrIdentityId::from_uuid(
        Uuid::parse_str("11111111-2222-4333-8444-555555555555").expect("uuid"),
    );
    let roster = NetworkRoster {
        network_name: "Home".to_string(),
        devices: vec![member_hex.clone()],
        admins: vec![admin_hex.clone()],
        aliases: HashMap::from([(member_hex.clone(), "garden-node".to_string())]),
        signed_at: 1_726_000_000,
    };

    let identities = roster_app_key_identities(
        &roster,
        &BTreeMap::from([(member_hex.clone(), member_profile)]),
    )
    .expect("bridge roster identities");

    assert_eq!(identities.len(), 2);
    let admin_identity = identities
        .iter()
        .find(|identity| identity.facet.pubkey == admin_hex)
        .expect("admin identity");
    assert_eq!(admin_identity.role, RosterAppKeyRole::Admin);
    assert_eq!(
        admin_identity.facet.capabilities,
        NostrIdentityCapabilities::app_admin()
    );
    assert_eq!(
        admin_identity.facet.purposes,
        [NostrIdentityKeyPurpose::AppKey].into_iter().collect()
    );
    assert_eq!(admin_identity.legacy_network_alias, None);
    assert_eq!(admin_identity.facet.label, None);

    let member_identity = identities
        .iter()
        .find(|identity| identity.facet.pubkey == member_hex)
        .expect("member identity");
    assert_eq!(member_identity.role, RosterAppKeyRole::Member);
    assert_eq!(
        member_identity.facet.capabilities,
        NostrIdentityCapabilities::app_writer()
    );
    assert_eq!(member_identity.facet.profile_id, Some(member_profile));
    assert_eq!(
        member_identity.legacy_network_alias.as_deref(),
        Some("garden-node")
    );
    assert_eq!(member_identity.facet.label, None);
}

#[test]
fn signed_roster_bridge_does_not_change_30388_wire_tags() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let admin_hex = admin.public_key().to_hex();
    let member_hex = member.public_key().to_hex();
    let roster = NetworkRoster {
        network_name: "Home".to_string(),
        devices: vec![member_hex.clone()],
        admins: vec![admin_hex],
        aliases: HashMap::from([(member_hex.clone(), "phone".to_string())]),
        signed_at: 1_726_000_000,
    };
    let signed = SignedRoster::sign("mesh-home", roster, &admin).expect("sign roster");
    let before_tags = signed
        .event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();

    let identities =
        signed_roster_app_key_identities(&signed, &BTreeMap::new()).expect("bridge signed roster");

    assert_eq!(identities.len(), 2);
    assert_eq!(u16::from(signed.event.kind), 30_388);
    assert_ne!(
        u16::from(signed.event.kind),
        CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND
    );
    assert_eq!(
        CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE,
        "nostr_identity_roster_op"
    );
    assert!(signed.event.content.is_empty());
    assert_eq!(
        signed
            .event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>(),
        before_tags
    );
    assert!(
        before_tags
            .iter()
            .any(|tag| tag.first() == Some(&"alias".to_string()))
    );
    assert!(!before_tags.iter().any(|tag| {
        tag.first().is_some_and(|name| {
            matches!(
                name.as_str(),
                "type" | "key_purpose" | "encrypted_device_labels"
            )
        })
    }));
    signed
        .verify()
        .expect("legacy signed roster still verifies");
}

#[test]
fn unified_bridge_accepts_legacy_signed_rosters_and_identity_roster_ops() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let admin_hex = admin.public_key().to_hex();
    let member_hex = member.public_key().to_hex();
    let profile_id = NostrIdentityId::from_uuid(
        Uuid::parse_str("15151515-2626-4747-8888-999999999999").expect("uuid"),
    );
    let roster = NetworkRoster {
        network_name: "Home".to_string(),
        devices: vec![member_hex.clone()],
        admins: vec![admin_hex.clone()],
        aliases: HashMap::from([(member_hex.clone(), "phone".to_string())]),
        signed_at: 1_726_000_010,
    };
    let signed = SignedRoster::sign("mesh-home", roster, &admin).expect("sign roster");

    let legacy = parse_identity_roster_bridge_event(
        &signed.event,
        &BTreeMap::from([(member_hex.clone(), profile_id)]),
    )
    .expect("parse legacy bridge event")
    .expect("legacy bridge event");

    assert_eq!(
        legacy.source,
        RosterIdentityBridgeSource::LegacySignedNetworkRoster
    );
    assert_eq!(legacy.network_id.as_deref(), Some("mesh-home"));
    assert_eq!(legacy.network_name.as_deref(), Some("Home"));
    assert_eq!(legacy.signer_pubkey, admin_hex);
    assert_eq!(legacy.signed_at, 1_726_000_010);
    assert_eq!(legacy.identities.len(), 2);
    assert!(legacy.identities.iter().any(|identity| {
        identity.role == RosterAppKeyRole::Admin
            && identity.facet.pubkey == admin.public_key().to_hex()
    }));
    assert!(legacy.identities.iter().any(|identity| {
        identity.role == RosterAppKeyRole::Member
            && identity.facet.pubkey == member_hex
            && identity.facet.profile_id == Some(profile_id)
    }));

    let sidecar = build_roster_app_key_sidecar_event(
        &admin,
        profile_id,
        &member.public_key().to_hex(),
        RosterAppKeyRole::Member,
        Vec::new(),
        None,
        1_726_000_011,
    )
    .expect("build sidecar");
    let canonical = parse_identity_roster_bridge_event(&sidecar, &BTreeMap::new())
        .expect("parse canonical bridge event")
        .expect("canonical bridge event");

    assert_eq!(
        canonical.source,
        RosterIdentityBridgeSource::NostrIdentityRosterOp
    );
    assert_eq!(canonical.network_id, None);
    assert_eq!(canonical.network_name, None);
    assert_eq!(canonical.signer_pubkey, admin.public_key().to_hex());
    assert_eq!(canonical.signed_at, 1_726_000_011);
    assert_eq!(canonical.identities.len(), 1);
    assert_eq!(canonical.identities[0].facet.pubkey, member_hex);
}

#[test]
fn bridge_builds_and_parses_canonical_roster_sidecar_facts() {
    let admin = Keys::generate();
    let member = Keys::generate();
    let profile_id = NostrIdentityId::from_uuid(
        Uuid::parse_str("22222222-3333-4444-8555-666666666666").expect("uuid"),
    );
    let created_at = 1_726_000_100;

    let event = build_roster_app_key_sidecar_event_with_network_name(
        &admin,
        profile_id,
        &member.public_key().to_bech32().expect("member npub"),
        RosterAppKeyRole::Member,
        Vec::new(),
        None,
        created_at,
        Some(" Home Mesh ".to_string()),
    )
    .expect("build sidecar");

    assert_eq!(u16::from(event.kind), CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND);
    assert!(event.content.is_empty());
    assert!(event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first() == Some(&"type".to_string())
            && parts.get(1) == Some(&CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE.to_string())
    }));
    assert!(event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first() == Some(&"key_pubkey".to_string())
            && parts.get(1) == Some(&member.public_key().to_hex())
    }));
    assert!(event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first() == Some(&CANONICAL_NETWORK_NAME_FACT.to_string())
            && parts.get(1) == Some(&"Home Mesh".to_string())
    }));

    let parsed = parse_roster_app_key_sidecar_event(&event)
        .expect("parse sidecar")
        .expect("sidecar app key identity");

    assert_eq!(parsed.role, RosterAppKeyRole::Member);
    assert_eq!(parsed.facet.pubkey, member.public_key().to_hex());
    assert_eq!(parsed.facet.profile_id, Some(profile_id));
    assert_eq!(
        parsed.facet.purposes,
        [NostrIdentityKeyPurpose::AppKey].into_iter().collect()
    );
    assert_eq!(
        parsed.facet.capabilities,
        NostrIdentityCapabilities::app_writer()
    );

    let bridged = parse_identity_roster_bridge_event(&event, &BTreeMap::new())
        .expect("parse bridge event")
        .expect("canonical bridge event");
    assert_eq!(bridged.network_name.as_deref(), Some("Home Mesh"));
}

#[test]
fn scan_to_approve_link_request_accepts_manual_npub_inputs() {
    let joining_device = Keys::generate();
    let admin = Keys::generate();
    let invite = Keys::generate();
    let profile_id = NostrIdentityId::from_uuid(
        Uuid::parse_str("33333333-4444-4555-8666-777777777777").expect("uuid"),
    );

    let event = build_identity_link_request_from_manual_npub(
        &joining_device,
        profile_id,
        &admin.public_key().to_bech32().expect("admin npub"),
        &invite.public_key().to_bech32().expect("invite npub"),
        "join request from phone",
        Some(" phone ".to_string()),
        1_726_000_200,
    )
    .expect("build link request");

    assert_eq!(u16::from(event.kind), CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND);
    let parsed = parse_identity_link_request_event_for_invite_pubkey(
        &event,
        &invite,
        invite.public_key().to_hex(),
    )
    .expect("parse link request");

    assert_eq!(parsed.content.identity, profile_id.as_uuid());
    assert_eq!(parsed.content.admin_pubkey, admin.public_key().to_hex());
    assert_eq!(parsed.content.invite_pubkey, invite.public_key().to_hex());
    assert_eq!(
        parsed.content.joining_pubkey,
        joining_device.public_key().to_hex()
    );
    assert_eq!(parsed.content.client_nonce, "join request from phone");
    assert_eq!(parsed.content.label.as_deref(), Some("phone"));
}

#[test]
fn approval_can_be_built_directly_from_a_scanned_link_request() {
    let admin = Keys::generate();
    let joining_device = Keys::generate();
    let profile_id = NostrIdentityId::from_uuid(
        Uuid::parse_str("35353535-4646-4747-8888-999999999999").expect("uuid"),
    );
    let request = build_identity_link_request_from_manual_npub(
        &joining_device,
        profile_id,
        &admin.public_key().to_bech32().expect("admin npub"),
        &admin.public_key().to_bech32().expect("invite npub"),
        "scan-secret",
        Some("Laptop".to_string()),
        1_726_000_250,
    )
    .expect("build link request");
    let signed_request = parse_identity_link_request_event_for_invite_pubkey(
        &request,
        &admin,
        admin.public_key().to_hex(),
    )
    .expect("parse link request");

    let approval = build_device_approval_for_link_request(
        &admin,
        &signed_request,
        Vec::new(),
        None,
        1_726_000_260,
    )
    .expect("build approval from link request");

    let receipt = parse_nostr_identity_device_approval_receipt_event(
        &approval.receipt_event,
        &joining_device,
    )
    .expect("parse approval receipt");
    assert_eq!(receipt.profile_id, profile_id);
    assert_eq!(receipt.request_pubkey, joining_device.public_key().to_hex());
    assert_eq!(
        receipt.device_app_key_pubkey,
        joining_device.public_key().to_hex()
    );
    assert_eq!(receipt.approved_by_pubkey, admin.public_key().to_hex());
    assert_eq!(receipt.request_secret, "scan-secret");
}

#[test]
fn approval_sidecar_embeds_canonical_roster_op_and_parses_receipt() {
    let admin = Keys::generate();
    let request = Keys::generate();
    let device = Keys::generate();
    let profile_id = NostrIdentityId::from_uuid(
        Uuid::parse_str("44444444-5555-4666-8777-888888888888").expect("uuid"),
    );

    let approval = build_device_approval_sidecar(
        &admin,
        NostrIdentityDeviceApprovalSidecarRequest {
            profile_id,
            network_name: Some("Home Mesh".to_string()),
            request_pubkey: request.public_key().to_bech32().expect("request npub"),
            device_app_key_pubkey: device.public_key().to_bech32().expect("device npub"),
            request_secret: "scan-secret".to_string(),
            parents: Vec::new(),
            actor_seq: None,
            approved_at: 1_726_000_300,
        },
    )
    .expect("build approval sidecar");

    assert_ne!(u16::from(approval.roster_op_event.kind), 30_388);
    assert_eq!(
        u16::from(approval.roster_op_event.kind),
        CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND
    );
    assert_eq!(
        u16::from(approval.receipt_event.kind),
        CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND
    );

    let receipt =
        parse_nostr_identity_device_approval_receipt_event(&approval.receipt_event, &request)
            .expect("parse receipt");
    assert_eq!(receipt.profile_id, profile_id);
    assert_eq!(receipt.request_pubkey, request.public_key().to_hex());
    assert_eq!(receipt.device_app_key_pubkey, device.public_key().to_hex());
    assert_eq!(receipt.approved_by_pubkey, admin.public_key().to_hex());
    assert_eq!(receipt.request_secret, "scan-secret");
    assert_eq!(
        receipt.signed_roster_event.as_deref(),
        Some(
            Event::from_json(approval.roster_op_event.as_json())
                .unwrap()
                .as_json()
        )
        .as_deref()
    );

    let roster_op = parse_nostr_identity_device_approval_receipt_roster_op(&receipt)
        .expect("receipt roster op");
    let bridged = parse_identity_roster_bridge_event(&approval.roster_op_event, &BTreeMap::new())
        .expect("parse bridge event")
        .expect("approval roster op");
    assert_eq!(bridged.network_name.as_deref(), Some("Home Mesh"));
    match roster_op.content.op {
        NostrIdentityRosterOp::AddFacet { facet } => {
            assert_eq!(facet.pubkey, device.public_key().to_hex());
            assert_eq!(facet.profile_id, Some(profile_id));
            assert_eq!(facet.capabilities, NostrIdentityCapabilities::app_writer());
        }
        other => panic!("expected add facet, got {other:?}"),
    }
}
