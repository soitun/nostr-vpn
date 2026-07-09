use anyhow::{Context, Result, anyhow};
use nostr_identity::{
    ApproveNostrIdentityDeviceApprovalRequestOptions, NostrIdentityCapabilities,
    approve_nostr_identity_device_approval_request, parse_nostr_identity_roster_op_event,
};
use nostr_sdk::prelude::{Client, Event, JsonUtil};
use nostr_vpn_core::config::{
    AppConfig, SharedNetworkRoster, normalize_nostr_pubkey, normalize_relay_urls,
    normalize_runtime_network_id,
};
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalRequest, NostrIdentityId, NostrVpnJoinApprovalContextRequest,
    RosterAppKeyRole, build_device_approval_sidecar_from_shared_approval,
    build_nostr_vpn_join_approval_context_event, build_roster_app_key_sidecar_event,
    encode_nostr_identity_device_approval_request,
};
use nostr_vpn_core::join_requests::{AppliedNostrJoinApproval, NOSTR_VPN_JOIN_REQUEST_TYPE};

#[derive(Debug, Clone)]
pub struct PreparedJoinApproval {
    pub updated_config: AppConfig,
    pub events: Vec<Event>,
    pub profile_id: NostrIdentityId,
}

pub fn prepare_join_approval(
    config: &AppConfig,
    network_entry_id: &str,
    request: &NostrIdentityDeviceApprovalRequest,
    approved_at: u64,
) -> Result<PreparedJoinApproval> {
    validate_join_approval_request(request, approved_at)?;
    let signer_keys = config.nostr_keys()?;
    let signer_pubkey = signer_keys.public_key().to_hex();
    if let Some(admin_app_key_pubkey) = &request.admin_app_key_pubkey {
        let admin_app_key_pubkey = normalize_nostr_pubkey(admin_app_key_pubkey)?;
        if admin_app_key_pubkey != signer_pubkey {
            return Err(anyhow!(
                "join request is addressed to a different admin device"
            ));
        }
    }
    let network = config
        .network_by_id(network_entry_id)
        .ok_or_else(|| anyhow!("network not found"))?;
    if !network.admins.iter().any(|admin| admin == &signer_pubkey) {
        return Err(anyhow!("active network is not administered by this device"));
    }

    let profile_id = request.profile_id.unwrap_or_else(NostrIdentityId::new_v4);
    let canonical_roster_events = if request.profile_id.is_none() {
        if approved_at == 0 {
            return Err(anyhow!(
                "fresh identity approval timestamp must be positive"
            ));
        }
        vec![build_roster_app_key_sidecar_event(
            &signer_keys,
            profile_id,
            &signer_pubkey,
            RosterAppKeyRole::Admin,
            Vec::new(),
            None,
            approved_at - 1,
        )?]
    } else {
        Vec::new()
    };
    let canonical_roster_ops = canonical_roster_events
        .iter()
        .map(|event| {
            parse_nostr_identity_roster_op_event(event)
                .map_err(|error| anyhow!("invalid canonical approval roster op: {error}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let approved_at_i64 =
        i64::try_from(approved_at).context("join approval timestamp overflows i64")?;
    let approval_content = approve_nostr_identity_device_approval_request(
        ApproveNostrIdentityDeviceApprovalRequestOptions {
            request: request.clone(),
            profile_id,
            roster_ops: canonical_roster_ops,
            approved_by_pubkey: signer_pubkey.clone(),
            approved_at: approved_at_i64,
            client_nonce: None,
            capabilities: Some(NostrIdentityCapabilities::app_writer()),
        },
    )
    .map_err(|error| anyhow!("shared device approval rejected join request: {error}"))?;
    let sidecar = build_device_approval_sidecar_from_shared_approval(
        &signer_keys,
        request,
        approval_content,
        canonical_roster_events,
    )
    .context("failed to build shared join request approval receipt")?;

    let (updated_config, shared) = stage_approved_config(config, network_entry_id, request)?;
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
        .filter(|exit| shared.devices.contains(exit) || shared.admins.contains(exit));
    let context_event = build_nostr_vpn_join_approval_context_event(
        &signer_keys,
        NostrVpnJoinApprovalContextRequest {
            profile_id,
            request_pubkey: request.request_pubkey.clone(),
            device_app_key_pubkey: request.device_app_key_pubkey.clone(),
            request_secret: request.request_secret.clone(),
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

fn validate_join_approval_request(
    request: &NostrIdentityDeviceApprovalRequest,
    approved_at: u64,
) -> Result<()> {
    encode_nostr_identity_device_approval_request(request, None)
        .map_err(|error| anyhow!("invalid join approval request: {error}"))?;
    if request.request_type.as_deref() != Some(NOSTR_VPN_JOIN_REQUEST_TYPE) {
        return Err(anyhow!("join approval request has invalid request type"));
    }
    if request.request_pubkey == request.device_app_key_pubkey {
        return Err(anyhow!(
            "join approval request must use a separate ephemeral request key"
        ));
    }
    let requested_at = u64::try_from(request.requested_at)
        .context("join approval request timestamp is negative")?;
    if requested_at > approved_at {
        return Err(anyhow!("join approval request timestamp is in the future"));
    }
    if let Some(expires_at) = request.expires_at {
        let expires_at =
            u64::try_from(expires_at).context("join approval request expiry is negative")?;
        if approved_at > expires_at {
            return Err(anyhow!("join approval request has expired"));
        }
    }
    Ok(())
}

fn stage_approved_config(
    config: &AppConfig,
    network_entry_id: &str,
    request: &NostrIdentityDeviceApprovalRequest,
) -> Result<(AppConfig, SharedNetworkRoster)> {
    let mut updated = config.clone();
    let device_pubkey = normalize_nostr_pubkey(&request.device_app_key_pubkey)?;
    updated.add_participant_to_network(network_entry_id, &device_pubkey)?;
    if let Some(label) = request
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

pub async fn publish_join_approval_events(config: &AppConfig, events: &[Event]) -> Result<()> {
    let relays = join_approval_relays(config);
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for join request approval receipt publishing"
        ));
    }
    let client = Client::new(config.nostr_keys()?);
    for relay in &relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    for event in events {
        let output = client
            .send_event_to(relays.clone(), event)
            .await
            .map_err(|error| anyhow!("failed to publish join request approval event: {error}"))?;
        if output.success.is_empty() {
            client.disconnect().await;
            return Err(anyhow!(
                "join request approval event was not accepted by any relay"
            ));
        }
    }
    client.disconnect().await;
    Ok(())
}

fn join_approval_relays(config: &AppConfig) -> Vec<String> {
    let disabled = normalize_relay_urls(config.nostr.disabled_relays.clone());
    normalize_relay_urls(config.nostr.relays.clone())
        .into_iter()
        .filter(|relay| !disabled.contains(relay))
        .collect()
}

#[cfg(test)]
mod tests {
    use nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX;
    use nostr_sdk::prelude::Keys;
    use nostr_vpn_core::identity_bridge::{
        CreateNostrIdentityDeviceApprovalRequestOptions,
        create_nostr_identity_device_approval_request,
        parse_nostr_identity_device_approval_receipt_event,
        parse_nostr_identity_device_approval_receipt_roster_op,
    };

    use super::*;

    #[test]
    fn prepared_approval_is_auto_applied_by_joiner() {
        let requested_at = 1_778_998_000;
        let approved_at = requested_at + 30;
        let mut joiner = AppConfig::generated_without_networks();
        joiner.node_name = "WebVM Guest".to_string();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let request = joiner
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request")
            .request
            .clone();

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

        let prepared = prepare_join_approval(&admin, &network_entry_id, &request, approved_at)
            .expect("prepare approval");
        let request_keys = joiner
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request")
            .request_keys()
            .expect("request keys");
        let receipt_event = &prepared.events[prepared.events.len() - 2];
        let receipt =
            parse_nostr_identity_device_approval_receipt_event(receipt_event, &request_keys)
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

    fn signed_request(
        request_type: Option<&str>,
        expires_at: Option<i64>,
        same_request_key: bool,
    ) -> NostrIdentityDeviceApprovalRequest {
        let device_keys = Keys::generate();
        create_nostr_identity_device_approval_request(
            &device_keys,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: same_request_key.then(|| device_keys.clone()),
                request_secret: None,
                requested_at: 100,
                request_type: request_type.map(str::to_string),
                resources: Vec::new(),
                expires_at,
                profile_id: None,
                admin_app_key_pubkey: None,
                label: Some("joining device".to_string()),
            },
        )
        .expect("signed request")
        .request
    }

    #[test]
    fn approval_rejects_wrong_or_missing_request_type_before_staging_config() {
        let (admin, network_id) = approval_admin();
        let original_devices = admin.networks[0].devices.clone();

        for request_type in [None, Some("some-other-app.join-request")] {
            let request = signed_request(request_type, None, false);
            let error = prepare_join_approval(&admin, &network_id, &request, 110)
                .expect_err("request type must be rejected");
            assert!(error.to_string().contains("request type"), "{error:#}");
            assert_eq!(admin.networks[0].devices, original_devices);
        }
    }

    #[test]
    fn approval_rejects_expired_and_same_key_requests_before_staging_config() {
        let (admin, network_id) = approval_admin();
        let original_devices = admin.networks[0].devices.clone();

        let expired = signed_request(Some(NOSTR_VPN_JOIN_REQUEST_TYPE), Some(105), false);
        let error = prepare_join_approval(&admin, &network_id, &expired, 110)
            .expect_err("expired request must be rejected");
        assert!(error.to_string().contains("expired"), "{error:#}");
        assert_eq!(admin.networks[0].devices, original_devices);

        let same_key = signed_request(Some(NOSTR_VPN_JOIN_REQUEST_TYPE), None, true);
        let error = prepare_join_approval(&admin, &network_id, &same_key, 110)
            .expect_err("same request and AppKey must be rejected");
        assert!(error.to_string().contains("ephemeral"), "{error:#}");
        assert_eq!(admin.networks[0].devices, original_devices);
    }

    #[test]
    fn approval_rejects_invalid_outer_signature_before_staging_config() {
        let (admin, network_id) = approval_admin();
        let original_devices = admin.networks[0].devices.clone();
        let mut request = signed_request(Some(NOSTR_VPN_JOIN_REQUEST_TYPE), None, false);
        let mut proof: serde_json::Value =
            serde_json::from_str(&request.device_app_key_proof).expect("proof JSON");
        proof["id"] = "0".repeat(64).into();
        request.device_app_key_proof = proof.to_string();

        let error = prepare_join_approval(&admin, &network_id, &request, 110)
            .expect_err("invalid proof signature must be rejected");
        assert!(error.to_string().contains("signature"), "{error:#}");
        assert_eq!(admin.networks[0].devices, original_devices);
    }

    #[test]
    fn approval_publisher_excludes_disabled_relays() {
        let mut config = AppConfig::generated();
        config.nostr.relays = vec![
            "wss://one.example".to_string(),
            "wss://two.example".to_string(),
        ];
        config.nostr.disabled_relays = vec!["wss://one.example".to_string()];

        assert_eq!(
            join_approval_relays(&config),
            vec!["wss://two.example".to_string()]
        );
    }
}
