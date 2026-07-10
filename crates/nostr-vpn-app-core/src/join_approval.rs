use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use nostr_identity::{
    ApproveNostrIdentityDeviceApprovalRequestOptions, NostrIdentityCapabilities,
    approve_nostr_identity_device_approval_request, nostr_identity_device_approval_bootstrap,
    nostr_identity_device_approval_request_relays, parse_nostr_identity_roster_op_event,
};
use nostr_pubsub::{EventBus, EventSource, QueryOptions, VerifiedEvent};
use nostr_pubsub_relay::RelayEventBus;
use nostr_sdk::prelude::{
    Alphabet, Client, Event, Filter, JsonUtil, Kind, PublicKey, RelayPoolNotification,
    SingleLetterTag,
};
use nostr_vpn_core::config::{
    AppConfig, SharedNetworkRoster, normalize_nostr_pubkey, normalize_runtime_network_id,
};
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalBootstrap, NostrIdentityDeviceApprovalRequest,
    NostrIdentityDeviceApprovalSidecar, NostrIdentityId, NostrVpnJoinApprovalContextRequest,
    RosterAppKeyRole, build_device_approval_sidecar_from_shared_approval,
    build_nostr_vpn_join_approval_context_event, build_roster_app_key_sidecar_event,
    parse_nostr_identity_device_approval_request_event,
};
use nostr_vpn_core::join_requests::{
    AppliedNostrJoinApproval, NOSTR_VPN_JOIN_APPROVAL_RELAY, NOSTR_VPN_JOIN_REQUEST_TYPE,
};

#[derive(Debug, Clone)]
pub struct PreparedJoinApproval {
    pub updated_config: AppConfig,
    pub events: Vec<Event>,
    pub profile_id: NostrIdentityId,
}

pub async fn fetch_join_approval_request(
    config: &AppConfig,
    bootstrap: &NostrIdentityDeviceApprovalBootstrap,
) -> Result<NostrIdentityDeviceApprovalRequest> {
    let request_pubkey = PublicKey::parse(&bootstrap.request_npub)
        .context("join request bootstrap has an invalid ephemeral npub")?;
    let device_app_key_pubkey = PublicKey::parse(&bootstrap.device_app_key_npub)
        .context("join request bootstrap has an invalid stable app npub")?;
    let provider = RelayEventBus::with_client(
        Client::new(config.nostr_keys()?),
        [NOSTR_VPN_JOIN_APPROVAL_RELAY],
        std::time::Duration::from_secs(10),
    )
    .await
    .map_err(|error| anyhow!("failed to initialize join request pubsub provider: {error}"))?;
    let filter = Filter::new()
        .kind(Kind::Custom(7_368))
        .author(request_pubkey)
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            device_app_key_pubkey.to_hex(),
        )
        .limit(8);
    let client = provider.client();
    let mut notifications = client.notifications();
    client
        .subscribe(filter.clone(), None)
        .await
        .context("failed to subscribe for signed join request")?;
    let report = provider
        .query(vec![filter], QueryOptions { limit: Some(8) })
        .await
        .map_err(|error| anyhow!("failed to fetch signed join request: {error}"))?;
    let events = report
        .events
        .into_iter()
        .map(|candidate| candidate.event.into_event())
        .collect::<Vec<_>>();
    if let Ok(request) =
        resolve_join_approval_request_events(bootstrap, &events, unix_timestamp_for_fetch())
    {
        return Ok(request);
    }

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(20));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = &mut timeout => break,
            notification = notifications.recv() => {
                match notification {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        if let Ok(request) = resolve_join_approval_request_events(
                            bootstrap,
                            &[(*event).clone()],
                            unix_timestamp_for_fetch(),
                        ) {
                            return Ok(request);
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("signed join request subscription closed"));
                    }
                }
            }
        }
    }
    Err(anyhow!(
        "signed join request was not found for the scanned bootstrap"
    ))
}

pub fn resolve_join_approval_request_events(
    bootstrap: &NostrIdentityDeviceApprovalBootstrap,
    events: &[Event],
    now: u64,
) -> Result<NostrIdentityDeviceApprovalRequest> {
    for event in events {
        if let Ok(request) = parse_nostr_identity_device_approval_request_event(event, bootstrap) {
            validate_join_approval_request(&request, now)?;
            return Ok(request);
        }
    }
    Err(anyhow!(
        "signed join request was not found for the scanned bootstrap"
    ))
}

fn unix_timestamp_for_fetch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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

    let (profile_id, sidecar) =
        build_canonical_approval_sidecar(&signer_keys, &signer_pubkey, request, approved_at)
            .context("failed to build canonical join approval")?;

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

fn build_canonical_approval_sidecar(
    signer_keys: &nostr_sdk::Keys,
    signer_pubkey: &str,
    request: &NostrIdentityDeviceApprovalRequest,
    approved_at: u64,
) -> Result<(NostrIdentityId, NostrIdentityDeviceApprovalSidecar)> {
    let profile_id = request.profile_id.unwrap_or_else(NostrIdentityId::new_v4);
    let canonical_roster_events = if request.profile_id.is_none() {
        if approved_at == 0 {
            return Err(anyhow!(
                "fresh identity approval timestamp must be positive"
            ));
        }
        vec![build_roster_app_key_sidecar_event(
            signer_keys,
            profile_id,
            signer_pubkey,
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
    let approval_content = approve_nostr_identity_device_approval_request(
        ApproveNostrIdentityDeviceApprovalRequestOptions {
            request: request.clone(),
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
    let sidecar = build_device_approval_sidecar_from_shared_approval(
        signer_keys,
        request,
        approval_content,
        canonical_roster_events,
    )
    .context("failed to build shared join request approval receipt")?;
    Ok((profile_id, sidecar))
}

fn validate_join_approval_request(
    request: &NostrIdentityDeviceApprovalRequest,
    approved_at: u64,
) -> Result<()> {
    nostr_identity_device_approval_bootstrap(request)
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

pub async fn publish_join_approval_events(
    config: &AppConfig,
    request: &NostrIdentityDeviceApprovalRequest,
    events: &[Event],
) -> Result<()> {
    let relays = join_approval_relays(request)?;
    let client = Client::new(config.nostr_keys()?);
    let provider = RelayEventBus::with_client(client, relays, std::time::Duration::from_secs(10))
        .await
        .map_err(|error| anyhow!("failed to initialize join approval pubsub provider: {error}"))?;
    let events = events
        .iter()
        .cloned()
        .map(VerifiedEvent::try_from)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("join request approval event failed signature verification")?;
    let publishes = events
        .into_iter()
        .map(|event| provider.publish(event, EventSource::local_index("nostr-vpn-join-approval")));
    let reports =
        tokio::time::timeout(std::time::Duration::from_secs(30), join_all(publishes)).await;
    provider.client().disconnect().await;
    let reports = reports.context("join request approval pubsub batch timed out")?;
    for report in reports {
        let report = report
            .map_err(|error| anyhow!("failed to publish join request approval event: {error}"))?;
        if !report.accepted {
            return Err(anyhow!(
                "join request approval pubsub provider rejected a verified event"
            ));
        }
    }
    Ok(())
}

fn join_approval_relays(request: &NostrIdentityDeviceApprovalRequest) -> Result<Vec<String>> {
    let relays = nostr_identity_device_approval_request_relays(request)
        .map_err(|error| anyhow!("invalid join request approval relay: {error}"))?;
    if relays.len() != 1 {
        return Err(anyhow!("join request must name exactly one approval relay"));
    }
    Ok(relays)
}

#[cfg(test)]
mod tests {
    use nostr_identity::{
        NOSTR_IDENTITY_DEVICE_APPROVAL_CLIENT_NONCE_PREFIX,
        nostr_identity_device_approval_relay_resource,
    };
    use nostr_sdk::prelude::Keys;
    use nostr_vpn_core::identity_bridge::{
        CreateNostrIdentityDeviceApprovalRequestOptions,
        create_nostr_identity_device_approval_request, nostr_identity_device_approval_bootstrap,
        parse_nostr_identity_device_approval_receipt_event,
        parse_nostr_identity_device_approval_receipt_roster_op,
    };

    use super::*;

    #[test]
    fn scanned_bootstrap_resolves_only_its_signed_request_event() {
        let requested_at = 1_778_998_000;
        let mut joiner = AppConfig::generated_without_networks();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let pending = joiner
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request");
        let bootstrap =
            nostr_identity_device_approval_bootstrap(&pending.request).expect("request bootstrap");
        let request_event = pending.request_event().expect("signed request event");
        let unrelated = nostr_sdk::EventBuilder::text_note("unrelated")
            .sign_with_keys(&Keys::generate())
            .expect("unrelated event");

        assert_eq!(
            resolve_join_approval_request_events(
                &bootstrap,
                &[unrelated, request_event],
                requested_at + 1,
            )
            .expect("resolve signed request"),
            pending.request
        );

        let mut wrong_secret = bootstrap;
        wrong_secret.request_secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string();
        assert!(
            resolve_join_approval_request_events(
                &wrong_secret,
                &[pending.request_event().expect("signed request event")],
                requested_at + 1,
            )
            .is_err()
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

    #[test]
    fn advertising_admin_is_joiner_exit_when_no_upstream_is_selected() {
        let requested_at = 1_778_998_000;
        let mut joiner = AppConfig::generated_without_networks();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending request");
        let request = joiner
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request")
            .request
            .clone();
        let (mut admin, network_id) = approval_admin();
        admin.node.advertise_exit_node = true;
        admin.exit_node.clear();
        let admin_pubkey = admin.own_nostr_pubkey_hex().expect("admin pubkey");

        let prepared = prepare_join_approval(&admin, &network_id, &request, requested_at + 30)
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
                resources: vec![
                    nostr_identity_device_approval_relay_resource(
                        nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY,
                    )
                    .expect("approval relay resource"),
                ],
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
        assert!(error.to_string().contains("distinct"), "{error:#}");
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
    fn approval_publisher_requires_the_signed_request_relay() {
        let mut request = signed_request(Some(NOSTR_VPN_JOIN_REQUEST_TYPE), None, false);
        assert_eq!(
            join_approval_relays(&request).expect("signed request relay"),
            vec![nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY.to_string()]
        );
        request.resources.clear();
        assert!(join_approval_relays(&request).is_err());
    }
}
