use anyhow::{Context, Result, anyhow};
use nostr_identity::{
    ApproveNostrIdentityDeviceApprovalBootstrapOptions, NostrIdentityCapabilities,
    approve_nostr_identity_device_approval_bootstrap, parse_nostr_identity_roster_op_event,
};
use nostr_sdk::prelude::{Event, JsonUtil};
use nostr_vpn_core::config::{
    AppConfig, SharedNetworkRoster, normalize_nostr_pubkey, normalize_runtime_network_id,
};
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalBootstrap, NostrIdentityDeviceApprovalSidecar, NostrIdentityId,
    NostrVpnJoinApprovalContextRequest, RosterAppKeyRole,
    build_device_approval_sidecar_from_bootstrap_approval,
    build_nostr_vpn_join_approval_context_event, build_roster_app_key_sidecar_event,
};
use nostr_vpn_core::join_requests::AppliedNostrJoinApproval;

#[derive(Debug, Clone)]
pub struct PreparedJoinApproval {
    pub updated_config: AppConfig,
    pub events: Vec<Event>,
    pub profile_id: NostrIdentityId,
}

pub fn prepare_join_approval(
    config: &AppConfig,
    network_entry_id: &str,
    bootstrap: &NostrIdentityDeviceApprovalBootstrap,
    approved_at: u64,
) -> Result<PreparedJoinApproval> {
    let signer_keys = config.nostr_keys()?;
    let signer_pubkey = signer_keys.public_key().to_hex();
    let network = config
        .network_by_id(network_entry_id)
        .ok_or_else(|| anyhow!("network not found"))?;
    if !network.admins.iter().any(|admin| admin == &signer_pubkey) {
        return Err(anyhow!("active network is not administered by this device"));
    }

    let (profile_id, sidecar) =
        build_canonical_approval_sidecar(&signer_keys, &signer_pubkey, bootstrap, approved_at)
            .context("failed to build canonical join approval")?;

    let (updated_config, shared) = stage_approved_config(config, network_entry_id, bootstrap)?;
    let signed_roster = SignedRoster::sign(
        shared.network_id.clone(),
        NetworkRoster {
            network_name: shared.name.clone(),
            devices: shared.devices.clone(),
            admins: shared.admins.clone(),
            aliases: shared.aliases.clone(),
            signed_at: approved_at,
        },
        &signer_keys,
    )
    .context("failed to sign approved Nostr VPN network roster")?;
    let roster_op_id = sidecar
        .approved_device_roster_op()
        .map(|event| event.id.to_hex());
    let exit_node_pubkey = normalize_nostr_pubkey(&updated_config.exit_node)
        .ok()
        .filter(|exit| shared.devices.contains(exit) || shared.admins.contains(exit))
        .or_else(|| {
            (updated_config.node.advertise_exit_node
                && (shared.devices.contains(&signer_pubkey)
                    || shared.admins.contains(&signer_pubkey)))
            .then(|| signer_pubkey.clone())
        });
    let context_event = build_nostr_vpn_join_approval_context_event(
        &signer_keys,
        NostrVpnJoinApprovalContextRequest {
            profile_id,
            request_pubkey: normalize_nostr_pubkey(&bootstrap.request_npub)?,
            device_app_key_pubkey: normalize_nostr_pubkey(&bootstrap.device_app_key_npub)?,
            request_secret: bootstrap.request_secret.clone(),
            mesh_network_id: normalize_runtime_network_id(&shared.network_id),
            network_name: Some(shared.name),
            roster_op_id,
            canonical_roster_events: sidecar
                .canonical_roster_events
                .iter()
                .map(JsonUtil::as_json)
                .collect(),
            signed_network_roster_event: signed_roster.event.as_json(),
            exit_node_pubkey,
            approved_at,
        },
    )
    .context("failed to build Nostr VPN join approval context")?;

    let mut events = sidecar.canonical_roster_events;
    events.push(signed_roster.event);
    events.push(sidecar.receipt_event);
    events.push(context_event);
    Ok(PreparedJoinApproval {
        updated_config,
        events,
        profile_id,
    })
}

fn build_canonical_approval_sidecar(
    signer_keys: &nostr_sdk::Keys,
    signer_pubkey: &str,
    bootstrap: &NostrIdentityDeviceApprovalBootstrap,
    approved_at: u64,
) -> Result<(NostrIdentityId, NostrIdentityDeviceApprovalSidecar)> {
    let profile_id = NostrIdentityId::new_v4();
    if approved_at == 0 {
        return Err(anyhow!(
            "fresh identity approval timestamp must be positive"
        ));
    }
    let canonical_roster_events = vec![build_roster_app_key_sidecar_event(
        signer_keys,
        profile_id,
        signer_pubkey,
        RosterAppKeyRole::Admin,
        Vec::new(),
        None,
        approved_at - 1,
    )?];
    let canonical_roster_ops = canonical_roster_events
        .iter()
        .map(|event| {
            parse_nostr_identity_roster_op_event(event)
                .map_err(|error| anyhow!("invalid canonical approval roster op: {error}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let approval_content = approve_nostr_identity_device_approval_bootstrap(
        ApproveNostrIdentityDeviceApprovalBootstrapOptions {
            bootstrap: bootstrap.clone(),
            profile_id,
            roster_ops: canonical_roster_ops,
            approved_by_pubkey: signer_pubkey.to_string(),
            approved_at: i64::try_from(approved_at)
                .context("join approval timestamp overflows i64")?,
            client_nonce: None,
            capabilities: Some(NostrIdentityCapabilities::app_writer()),
        },
    )
    .map_err(|error| anyhow!("shared device approval rejected join request: {error}"))?;
    let sidecar = build_device_approval_sidecar_from_bootstrap_approval(
        signer_keys,
        bootstrap,
        approval_content,
        canonical_roster_events,
    )
    .context("failed to build shared join request approval receipt")?;
    Ok((profile_id, sidecar))
}

fn stage_approved_config(
    config: &AppConfig,
    network_entry_id: &str,
    bootstrap: &NostrIdentityDeviceApprovalBootstrap,
) -> Result<(AppConfig, SharedNetworkRoster)> {
    let mut updated = config.clone();
    let device_pubkey = normalize_nostr_pubkey(&bootstrap.device_app_key_npub)?;
    updated.add_participant_to_network(network_entry_id, &device_pubkey)?;
    if let Some(label) = bootstrap
        .label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let _ = updated.set_peer_alias(&device_pubkey, label);
    }
    if let Some(network) = updated.network_by_id_mut(network_entry_id) {
        network
            .inbound_join_requests
            .retain(|pending| pending.requester != device_pubkey);
    }
    let shared = updated.shared_network_roster(network_entry_id)?;
    Ok((updated, shared))
}

pub fn apply_join_approval_events(
    config: &mut AppConfig,
    events: &[Event],
    now: u64,
) -> Result<Option<AppliedNostrJoinApproval>> {
    config.apply_nostr_join_approval_events(events, now)
}

#[cfg(test)]
mod tests {
    use nostr_identity::{
        NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX,
        NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_TYPE,
        parse_nostr_identity_device_approval_receipt_event_for_bootstrap,
    };
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::identity_bridge::{
        NostrIdentityDeviceApprovalBootstrap, nostr_identity_device_approval_bootstrap,
        parse_nostr_identity_device_approval_receipt_roster_op,
    };

    use super::*;

    fn has_tag(event: &Event, name: &str, value: &str) -> bool {
        event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.first().is_some_and(|part| part == name)
                && parts.get(1).is_some_and(|part| part == value)
        })
    }

    #[test]
    fn compact_bootstrap_is_the_complete_approval_input() {
        let requested_at = 1_778_998_000;
        let mut joiner = AppConfig::generated_without_networks();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let bootstrap = pending_bootstrap(&joiner);
        let (admin, network_id) = approval_admin();

        let prepared = prepare_join_approval(&admin, &network_id, &bootstrap, requested_at + 1)
            .expect("prepare approval from bootstrap");
        assert!(
            prepared.events.iter().all(|event| !has_tag(
                event,
                "type",
                NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_TYPE
            )),
            "approval output must not contain a request event"
        );
    }

    #[test]
    fn prepared_approval_is_auto_applied_by_joiner() {
        let requested_at = 1_778_998_000;
        let approved_at = requested_at + 30;
        let mut joiner = AppConfig::generated_without_networks();
        joiner.node_name = "WebVM Guest".to_string();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let bootstrap = pending_bootstrap(&joiner);

        let admin_keys = Keys::generate();
        let exit_keys = Keys::generate();
        let mut admin = AppConfig::generated();
        admin.nostr.secret_key = admin_keys.secret_key().to_secret_hex();
        admin.nostr.public_key = admin_keys.public_key().to_hex();
        admin.networks[0].name = "Home".to_string();
        admin.networks[0].enabled = true;
        admin.networks[0].network_id = "8d4f34f5425bc50e".to_string();
        admin.networks[0].devices = vec![
            admin_keys.public_key().to_hex(),
            exit_keys.public_key().to_hex(),
        ];
        admin.networks[0].admins = vec![admin_keys.public_key().to_hex()];
        admin.exit_node = exit_keys.public_key().to_hex();
        admin.ensure_defaults();
        let network_entry_id = admin.networks[0].id.clone();

        let prepared = prepare_join_approval(&admin, &network_entry_id, &bootstrap, approved_at)
            .expect("prepare approval");
        let request_keys = joiner
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request")
            .request_keys()
            .expect("request keys");
        let receipt_event = &prepared.events[prepared.events.len() - 2];
        let receipt = parse_nostr_identity_device_approval_receipt_event_for_bootstrap(
            receipt_event,
            &request_keys,
            &bootstrap,
        )
        .expect("approval receipt");
        let member_op = parse_nostr_identity_device_approval_receipt_roster_op(&receipt)
            .expect("canonical member op");
        assert!(
            member_op
                .content
                .client_nonce
                .starts_with(NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX)
        );
        let applied = apply_join_approval_events(&mut joiner, &prepared.events, approved_at + 1)
            .expect("apply approval")
            .expect("approval detected");

        assert_eq!(applied.approved_by_pubkey, admin_keys.public_key().to_hex());
        assert!(joiner.pending_nostr_join_request.is_none());
        assert_eq!(joiner.active_network().network_id, "8d4f34f5425bc50e");
        assert_eq!(
            joiner.active_network().admins,
            vec![admin_keys.public_key().to_hex()]
        );
        assert_eq!(joiner.exit_node, exit_keys.public_key().to_hex());
    }

    #[test]
    fn advertising_admin_is_joiner_exit_when_no_upstream_is_selected() {
        let requested_at = 1_778_998_000;
        let mut joiner = AppConfig::generated_without_networks();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let bootstrap = pending_bootstrap(&joiner);
        let (mut admin, network_id) = approval_admin();
        admin.node.advertise_exit_node = true;
        admin.exit_node.clear();
        let admin_pubkey = admin.own_nostr_pubkey_hex().expect("admin pubkey");

        let prepared = prepare_join_approval(&admin, &network_id, &bootstrap, requested_at + 30)
            .expect("prepare approval");
        apply_join_approval_events(&mut joiner, &prepared.events, requested_at + 31)
            .expect("apply approval")
            .expect("approval detected");

        assert_eq!(joiner.exit_node, admin_pubkey);
    }

    fn approval_admin() -> (AppConfig, String) {
        let admin_keys = Keys::generate();
        let mut admin = AppConfig::generated();
        admin.nostr.secret_key = admin_keys.secret_key().to_secret_hex();
        admin.nostr.public_key = admin_keys.public_key().to_hex();
        admin.networks[0].enabled = true;
        admin.networks[0].devices = vec![admin_keys.public_key().to_hex()];
        admin.networks[0].admins = vec![admin_keys.public_key().to_hex()];
        admin.ensure_defaults();
        let network_id = admin.networks[0].id.clone();
        (admin, network_id)
    }

    fn pending_bootstrap(config: &AppConfig) -> NostrIdentityDeviceApprovalBootstrap {
        let pending = config
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request");
        nostr_identity_device_approval_bootstrap(&pending.request).expect("request bootstrap")
    }

    #[test]
    fn approval_rejects_reused_stable_and_ephemeral_key() {
        let (admin, network_id) = approval_admin();
        let original_devices = admin.networks[0].devices.clone();
        let device = Keys::generate();
        let npub = device.public_key().to_bech32().expect("device npub");
        let bootstrap = NostrIdentityDeviceApprovalBootstrap {
            device_app_key_npub: npub.clone(),
            request_npub: npub,
            request_secret: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            label: Some("WebVM".to_string()),
        };

        let error = prepare_join_approval(&admin, &network_id, &bootstrap, 110)
            .expect_err("same request and AppKey must be rejected");
        assert!(format!("{error:#}").contains("distinct"), "{error:#}");
        assert_eq!(admin.networks[0].devices, original_devices);
    }

    #[test]
    fn approval_applies_the_bounded_bootstrap_label() {
        let (admin, network_id) = approval_admin();
        let device = Keys::generate();
        let request = Keys::generate();
        let device_hex = device.public_key().to_hex();
        let bootstrap = NostrIdentityDeviceApprovalBootstrap {
            device_app_key_npub: device.public_key().to_bech32().expect("device npub"),
            request_npub: request.public_key().to_bech32().expect("request npub"),
            request_secret: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            label: Some("WebVM".to_string()),
        };

        let prepared = prepare_join_approval(&admin, &network_id, &bootstrap, 110)
            .expect("prepare labeled approval");
        assert_eq!(
            prepared.updated_config.peer_alias(&device_hex).as_deref(),
            Some("webvm")
        );
    }
}
