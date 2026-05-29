use std::{collections::HashMap, fs};

use crate::*;
use nostr_sdk::prelude::{Keys, Tag, ToBech32};
use nostr_vpn_core::config::{NetworkConfig, PendingOutboundJoinRequest};

fn activate_first_network(config: &mut AppConfig) {
    let network_id = config.networks[0].id.clone();
    config
        .set_network_enabled(&network_id, true)
        .expect("activate first network");
}

#[test]
fn participants_override_targets_the_active_network() {
    let alice = Keys::generate().public_key().to_hex();
    let bob = Keys::generate().public_key().to_hex();
    let carol = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.networks = vec![
        NetworkConfig {
            id: "home".to_string(),
            name: "Home".to_string(),
            enabled: false,
            network_id: "mesh-home".to_string(),
            invite_secret: "home-secret".to_string(),
            participants: vec![alice.clone()],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
        NetworkConfig {
            id: "work".to_string(),
            name: "Work".to_string(),
            enabled: true,
            network_id: "mesh-work".to_string(),
            invite_secret: "work-secret".to_string(),
            participants: vec![bob],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
    ];
    config.ensure_defaults();

    apply_participants_override(&mut config, vec![carol.clone()]).expect("apply override");

    assert_eq!(config.participant_pubkeys_hex(), vec![carol.clone()]);
    assert_eq!(
        config
            .network_by_id("home")
            .expect("home network")
            .participants,
        vec![alice]
    );
    assert_eq!(
        config
            .network_by_id("work")
            .expect("work network")
            .participants,
        vec![carol]
    );
}

#[test]
fn participants_override_preserves_selected_exit_node_when_it_remains_a_member() {
    let exit_peer = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    config.exit_node = exit_peer.clone();

    apply_participants_override(&mut config, vec![exit_peer.clone()]).expect("apply override");

    assert_eq!(config.participant_pubkeys_hex(), vec![exit_peer.clone()]);
    assert_eq!(config.exit_node, exit_peer);
}

#[test]
fn pending_join_request_recipients_use_selected_admin_and_skip_self() {
    let mut config = AppConfig::generated();
    let own_pubkey = Keys::parse(&config.nostr.secret_key)
        .expect("own keys")
        .public_key()
        .to_hex();
    let admin = Keys::generate().public_key().to_hex();
    let backup_admin = Keys::generate().public_key().to_hex();

    config.networks[0].enabled = true;
    config.networks[0].participants.clear();
    config.networks[0].admins = vec![own_pubkey, admin.clone(), backup_admin];
    config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
        recipient: admin.clone(),
        requested_at: 123,
    });

    assert_eq!(pending_fips_join_request_recipients(&config), vec![admin]);
}

#[test]
fn pending_join_request_recipients_fall_back_to_admins_without_self() {
    let mut config = AppConfig::generated();
    let own_pubkey = Keys::parse(&config.nostr.secret_key)
        .expect("own keys")
        .public_key()
        .to_hex();
    let admin = Keys::generate().public_key().to_hex();
    let stale_recipient = Keys::generate().public_key().to_hex();

    config.networks[0].enabled = true;
    config.networks[0].participants.clear();
    config.networks[0].admins = vec![own_pubkey, admin.clone()];
    config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
        recipient: stale_recipient,
        requested_at: 123,
    });

    assert_eq!(pending_fips_join_request_recipients(&config), vec![admin]);
}

#[test]
fn participants_override_marks_shared_roster_updated_for_admin_owned_network() {
    let member = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own nostr pubkey");
    config.networks[0].admins = vec![own_pubkey.clone()];
    config.networks[0].shared_roster_updated_at = 0;
    config.networks[0].shared_roster_signed_by.clear();

    apply_participants_override(&mut config, vec![member.clone()]).expect("apply override");

    let active_network = config.active_network();
    assert_eq!(active_network.participants, vec![member]);
    assert!(active_network.shared_roster_updated_at > 0);
    assert_eq!(active_network.shared_roster_signed_by, own_pubkey);
}

#[test]
fn shared_roster_publish_allowed_only_for_current_signer() {
    let other_admin = Keys::generate().public_key().to_hex();
    let outsider = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    let own_pubkey = config.own_nostr_pubkey_hex().expect("own nostr pubkey");
    activate_first_network(&mut config);
    let network_id = config.active_network().id.clone();
    config.networks[0].admins = vec![own_pubkey.clone(), other_admin.clone()];

    assert!(shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        ""
    ));
    assert!(shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        &own_pubkey
    ));
    assert!(!shared_roster_publish_allowed(
        &config,
        &network_id,
        &own_pubkey,
        &other_admin
    ));
    assert!(!shared_roster_publish_allowed(
        &config,
        &network_id,
        &outsider,
        &outsider
    ));
}

#[test]
fn forwarded_signed_roster_can_be_selected_for_peer_sync() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-forwarded-signed-roster-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");

    let mut alice = AppConfig::generated();
    activate_first_network(&mut alice);
    let alice_pubkey = alice.own_nostr_pubkey_hex().expect("alice pubkey");

    let mut bob = AppConfig::generated();
    activate_first_network(&mut bob);
    let bob_pubkey = bob.own_nostr_pubkey_hex().expect("bob pubkey");
    let carol_pubkey = Keys::generate().public_key().to_hex();

    alice.networks[0].name = "Home".to_string();
    alice.networks[0].network_id = "mesh".to_string();
    alice.networks[0].participants = vec![bob_pubkey.clone(), carol_pubkey.clone()];
    alice.networks[0].admins = vec![alice_pubkey.clone(), bob_pubkey.clone()];
    alice.networks[0].shared_roster_updated_at = 1_726_000_000;
    alice.networks[0].shared_roster_signed_by = alice_pubkey.clone();

    let alice_shared = alice
        .shared_network_roster(&alice.networks[0].id)
        .expect("alice shared roster");
    let signed = SignedRoster::sign(
        &alice_shared.network_id,
        network_roster_from_shared(&alice_shared),
        &alice.nostr_keys().expect("alice keys"),
    )
    .expect("sign roster");

    bob.networks[0].name = "Home".to_string();
    bob.networks[0].network_id = "mesh".to_string();
    bob.networks[0].participants = vec![alice_pubkey.clone(), carol_pubkey];
    bob.networks[0].admins = vec![alice_pubkey, bob_pubkey];
    bob.networks[0].shared_roster_updated_at = signed.signed_at();
    bob.networks[0].shared_roster_signed_by = signed.signer_pubkey_hex().expect("signer");

    upsert_signed_roster(&signed_rosters_file_path(&config_path), signed.clone())
        .expect("persist signed roster");

    let forwarded = active_signed_roster_for_sync(&bob, &config_path, true)
        .expect("load forwarded roster")
        .expect("forwarded roster should be selected");
    assert_eq!(forwarded.artifact_hash(), signed.artifact_hash());
    assert!(
        active_signed_roster_for_sync(&bob, &config_path, false)
            .expect("load own-signed roster")
            .is_none(),
        "explicit admin publish should not re-sign or forward another admin's stored artifact"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inbound_fips_roster_requires_signed_event() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-unsigned-roster-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut status = String::new();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    config.networks[0].network_id = "mesh".to_string();

    let error = persist_shared_network_roster(&mut config, &config_path, None, &mut status)
        .expect_err("unsigned roster frame must be rejected");

    assert!(
        error.to_string().contains("missing signed roster event"),
        "unexpected error: {error:#}"
    );
    assert_eq!(config.networks[0].shared_roster_updated_at, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inbound_fips_roster_accepts_admin_signed_event() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-admin-roster-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut status = String::new();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let admin = config.nostr_keys().expect("admin keys");
    let admin_hex = admin.public_key().to_hex();
    let member_hex = Keys::generate().public_key().to_hex();
    config.networks[0].name = "Original".to_string();
    config.networks[0].network_id = "mesh".to_string();
    config.networks[0].admins = vec![admin_hex.clone()];
    config.networks[0].shared_roster_updated_at = 0;
    config.networks[0].shared_roster_signed_by.clear();

    let signed = SignedRoster::sign(
        "mesh",
        NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![member_hex.clone()],
            admins: vec![admin_hex.clone()],
            aliases: HashMap::new(),
            signed_at: 1_726_000_000,
        },
        &admin,
    )
    .expect("sign roster");

    let result =
        persist_shared_network_roster(&mut config, &config_path, Some(&signed), &mut status)
            .expect("admin-signed roster should apply");

    assert_eq!(result.as_deref(), Some("Home"));
    assert_eq!(config.networks[0].name, "Home");
    assert_eq!(config.networks[0].participants, vec![member_hex]);
    assert_eq!(config.networks[0].admins, vec![admin_hex.clone()]);
    assert_eq!(config.networks[0].shared_roster_updated_at, 1_726_000_000);
    assert_eq!(config.networks[0].shared_roster_signed_by, admin_hex);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inbound_fips_roster_rejects_tampered_signed_event() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-tampered-roster-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut status = String::new();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let admin = config.nostr_keys().expect("admin keys");
    let admin_hex = admin.public_key().to_hex();
    let member_hex = Keys::generate().public_key().to_hex();
    config.networks[0].name = "Original".to_string();
    config.networks[0].network_id = "mesh".to_string();
    config.networks[0].admins = vec![admin_hex.clone()];
    config.networks[0].shared_roster_updated_at = 0;
    config.networks[0].shared_roster_signed_by.clear();

    let signed = SignedRoster::sign(
        "mesh",
        NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![member_hex],
            admins: vec![admin_hex],
            aliases: HashMap::new(),
            signed_at: 1_726_000_000,
        },
        &admin,
    )
    .expect("sign roster");
    let mut event = signed.event.clone();
    event
        .tags
        .push(Tag::parse(["name", "Office"]).expect("tag"));
    let tampered = SignedRoster { event };

    let error =
        persist_shared_network_roster(&mut config, &config_path, Some(&tampered), &mut status)
            .expect_err("tampered signed roster frame must be rejected");

    assert!(
        error.to_string().contains("invalid roster event signature"),
        "unexpected error: {error:#}"
    );
    assert_eq!(config.networks[0].name, "Original");
    assert_eq!(config.networks[0].shared_roster_updated_at, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inbound_fips_roster_ignores_signed_event_from_non_admin_author() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-non-admin-roster-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut status = String::new();

    let known_admin = Keys::generate();
    let outsider = Keys::generate();
    let known_admin_hex = known_admin.public_key().to_hex();
    let outsider_hex = outsider.public_key().to_hex();
    let member_hex = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    config.networks[0].name = "Original".to_string();
    config.networks[0].network_id = "mesh".to_string();
    config.networks[0].admins = vec![known_admin_hex.clone()];
    config.networks[0].shared_roster_updated_at = 0;
    config.networks[0].shared_roster_signed_by.clear();

    let signed = SignedRoster::sign(
        "mesh",
        NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![member_hex],
            admins: vec![known_admin_hex, outsider_hex],
            aliases: HashMap::new(),
            signed_at: 1_726_000_000,
        },
        &outsider,
    )
    .expect("sign roster");

    let result =
        persist_shared_network_roster(&mut config, &config_path, Some(&signed), &mut status)
            .expect("valid event from non-admin author should be ignored");

    assert!(result.is_none());
    assert_eq!(config.networks[0].name, "Original");
    assert_eq!(config.networks[0].shared_roster_updated_at, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn active_network_invite_code_roundtrips_current_roster() {
    let participant_hex = Keys::generate().public_key().to_hex();
    let admin_hex = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let inviter_hex = config
        .own_nostr_pubkey_hex()
        .expect("generated config has own key");
    let inviter_npub = nostr_vpn_core::invite::to_npub(&inviter_hex);
    config.networks[0].name = "Work".to_string();
    config.networks[0].network_id = "8d4f34f5425bc50e".to_string();
    config.networks[0].participants = vec![participant_hex];
    config.networks[0].admins = vec![inviter_hex.clone(), admin_hex];
    config.networks[0].invite_inviter = inviter_hex;
    config.node.endpoint = "192.168.50.10:51820".to_string();
    config.nostr.relays = vec!["wss://temp.iris.to".to_string()];

    let invite = active_network_invite_code(&config).expect("invite should encode");
    let parsed = parse_network_invite(&invite).expect("invite should decode");

    assert!(invite.starts_with(NETWORK_INVITE_PREFIX));
    assert!(parsed.network_name.is_empty());
    assert_eq!(parsed.network_id, "8d4f34f5425bc50e");
    assert_eq!(parsed.invite_secret, config.networks[0].invite_secret);
    assert_eq!(parsed.admins.len(), 2);
    assert_eq!(parsed.inviter_npub, inviter_npub);
    assert_eq!(parsed.inviter_endpoints, vec!["192.168.50.10:51820"]);
    assert!(parsed.participants.is_empty());
    assert!(parsed.relays.is_empty());
}

#[test]
fn active_network_invite_omits_non_transport_inviter_endpoint() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let inviter_hex = config
        .own_nostr_pubkey_hex()
        .expect("generated config has own key");
    config.networks[0].network_id = "8d4f34f5425bc50e".to_string();
    config.networks[0].admins = vec![inviter_hex.clone()];
    config.networks[0].invite_inviter = inviter_hex;
    config.node.endpoint = "fips".to_string();

    let invite = active_network_invite_code(&config).expect("invite should encode");
    let parsed = parse_network_invite(&invite).expect("invite should decode");

    assert!(parsed.inviter_endpoints.is_empty());
}

#[test]
fn active_network_invite_requires_local_admin_key() {
    let other_admin = Keys::generate().public_key().to_hex();

    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let own_pubkey = config
        .own_nostr_pubkey_hex()
        .expect("generated config has own key");
    config.networks[0].network_id = "8d4f34f5425bc50e".to_string();
    config.networks[0].participants = vec![own_pubkey];
    config.networks[0].admins = vec![other_admin];
    config.node.endpoint = "192.168.50.10:51820".to_string();

    let error =
        active_network_invite_code(&config).expect_err("non-admin device must not create invite");

    assert!(error.to_string().contains("network admin"));
}

#[test]
fn importing_current_invite_queues_join_request_to_admin() {
    let admin_npub = Keys::generate()
        .public_key()
        .to_bech32()
        .expect("admin npub");
    let admin_hex = normalize_nostr_pubkey(&admin_npub).expect("normalize admin");
    let invite = serde_json::json!({
        "v": 3,
        "networkId": "8d4f34f5425bc50e",
        "inviteSecret": "join-secret",
        "inviterEndpoints": [" 192.168.50.20:51820 ", "fips", "198.51.100.10:51820", admin_npub],
        "admins": [admin_npub],
        "relays": ["wss://temp.iris.to"]
    })
    .to_string();

    let mut config = AppConfig::generated();
    let parsed = parse_network_invite(&invite).expect("invite should parse");
    apply_network_invite_to_active_network(&mut config, &parsed).expect("invite should apply");
    let queued = queue_active_network_join_request(&mut config).expect("join request should queue");

    let network = config.active_network();
    assert!(queued);
    assert_eq!(config.networks.len(), 1);
    assert_eq!(network.id, "network-1");
    assert_eq!(
        network
            .outbound_join_request
            .as_ref()
            .expect("pending join request")
            .recipient,
        admin_hex
    );
    assert_eq!(network.invite_secret, "join-secret");
    assert_eq!(
        config.fips_peer_endpoints.get(&admin_npub),
        Some(&vec!["192.168.50.20:51820".to_string()])
    );
    assert!(network.participants.is_empty());
}

#[test]
fn manual_join_invite_with_admin_id_and_mesh_id_queues_join_request() {
    // Mirrors the iOS / Android manual-join UI: user has the admin's
    // Device ID (npub) and the mesh network id but no invite link, so
    // the shell builds a synthetic JSON invite shaped like
    //   {"v":3,"networkId":"...","inviterNpub":"npub1...","admins":["npub1..."]}
    // and hands it to import_network_invite. The end state must be the
    // same as importing the equivalent invite link: network present
    // locally with the admin in its admin set, join request queued for
    // the admin to accept (which then sends back the roster including
    // us once the admin Add-by-Device-IDs us).
    let admin_npub = Keys::generate()
        .public_key()
        .to_bech32()
        .expect("admin npub");
    let admin_hex = normalize_nostr_pubkey(&admin_npub).expect("normalize admin");
    let manual_invite = serde_json::json!({
        "v": 3,
        "networkId": "abcdef0123456789",
        "inviterNpub": admin_npub,
        "admins": [admin_npub],
        "participants": []
    })
    .to_string();

    let mut config = AppConfig::generated();
    let parsed = parse_network_invite(&manual_invite).expect("manual invite parses");
    apply_network_invite_to_active_network(&mut config, &parsed).expect("manual invite applies");
    let queued = queue_active_network_join_request(&mut config).expect("join request queues");

    let network = config.active_network();
    assert!(queued, "join request should be queued for the admin");
    assert_eq!(
        network.network_id, "abcdef0123456789",
        "mesh id from manual invite should land on the active network"
    );
    assert!(
        network.admins.iter().any(|admin| admin == &admin_hex),
        "admin Device ID must end up in the active network's admin set"
    );
    assert_eq!(
        network
            .outbound_join_request
            .as_ref()
            .expect("pending join request")
            .recipient,
        admin_hex,
        "join request must be addressed to the admin from the manual invite"
    );
    // Manual invite carries no participants — the requester is added
    // only after the admin accepts and broadcasts the updated roster.
    assert!(
        network.participants.is_empty(),
        "no participants until the admin accepts and propagates the roster"
    );
}

#[test]
fn config_overrides_set_the_active_network_mesh_id() {
    let nonce = unix_timestamp();
    let dir = std::env::temp_dir().join(format!("nvpn-load-config-override-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");

    let mut config = AppConfig::generated();
    config.networks = vec![
        NetworkConfig {
            id: "home".to_string(),
            name: "Home".to_string(),
            enabled: false,
            network_id: "mesh-home".to_string(),
            invite_secret: "home-secret".to_string(),
            participants: vec!["11".repeat(32)],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
        NetworkConfig {
            id: "work".to_string(),
            name: "Work".to_string(),
            enabled: true,
            network_id: "mesh-work".to_string(),
            invite_secret: "work-secret".to_string(),
            participants: vec!["22".repeat(32)],
            admins: Vec::new(),
            listen_for_join_requests: true,
            invite_inviter: String::new(),
            outbound_join_request: None,
            inbound_join_requests: Vec::new(),
            shared_roster_updated_at: 0,
            shared_roster_signed_by: String::new(),
        },
    ];
    config.ensure_defaults();
    config.save(&config_path).expect("save temp config");

    let (loaded, network_id) =
        load_config_with_overrides(&config_path, Some("mesh-override".to_string()), Vec::new())
            .expect("load config with override");

    assert_eq!(network_id, "mesh-override");
    assert_eq!(loaded.effective_network_id(), "mesh-override");
    assert_eq!(
        loaded
            .network_by_id("home")
            .expect("home network")
            .network_id,
        "mesh-home"
    );
    assert_eq!(
        loaded
            .network_by_id("work")
            .expect("work network")
            .network_id,
        "mesh-override"
    );

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&dir);
}
