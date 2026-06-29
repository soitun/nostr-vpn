use std::collections::{BTreeMap, HashMap};

use nostr_sdk::prelude::Keys;
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND, CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE,
    NostrIdentityCapabilities, NostrIdentityId, NostrIdentityKeyPurpose, RosterAppKeyRole,
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
