use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::identity_bridge::{
    CreateNostrIdentityDeviceApprovalRequestOptions, NostrIdentityDeviceApprovalRequest,
    create_nostr_identity_device_approval_request, encode_nostr_identity_device_approval_request,
    parse_compact_nostr_identity_device_approval_request,
    parse_nostr_identity_device_approval_request,
};

pub(crate) const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
const JOIN_REQUEST_COMPACT_LINK_PREFIX: &str = "nvpn://join-request";
const JOIN_REQUEST_TYPE: &str = "nostr-vpn.join-request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinRequestQrCodeOrLink {
    pub pubkey_hex: String,
    pub node_name: String,
    pub approval_request: NostrIdentityDeviceApprovalRequest,
}

pub(crate) fn own_join_request_qr_code_or_link(config: &AppConfig) -> Result<String> {
    let app_keys = config.nostr_keys()?;
    let node_name = config.node_name.trim();
    let local_request = create_nostr_identity_device_approval_request(
        &app_keys,
        CreateNostrIdentityDeviceApprovalRequestOptions {
            request_keys: None,
            request_secret: None,
            requested_at: unix_timestamp_i64(),
            request_type: Some(JOIN_REQUEST_TYPE.to_string()),
            resources: Vec::new(),
            expires_at: None,
            profile_id: None,
            admin_app_key_pubkey: None,
            label: (!node_name.is_empty()).then(|| node_name.to_string()),
        },
    )
    .context("failed to create join request")?;

    encode_nostr_identity_device_approval_request(
        &local_request.request,
        Some(JOIN_REQUEST_LINK_PREFIX),
    )
    .context("failed to encode join request")
}

pub(crate) fn parse_join_request_qr_code_or_link(value: &str) -> Result<JoinRequestQrCodeOrLink> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("join request is empty"));
    }

    if let Some(request) =
        parse_nostr_identity_device_approval_request(trimmed, &[JOIN_REQUEST_LINK_PREFIX])
            .context("failed to parse join request")?
    {
        let requester = normalize_nostr_pubkey(&request.device_app_key_pubkey)?;
        return Ok(JoinRequestQrCodeOrLink {
            pubkey_hex: requester,
            node_name: request.label.clone().unwrap_or_default(),
            approval_request: request,
        });
    }

    if parse_compact_nostr_identity_device_approval_request(
        trimmed,
        &[JOIN_REQUEST_COMPACT_LINK_PREFIX],
    )
    .context("failed to parse legacy compact join request")?
    .is_some()
    {
        return Err(anyhow!(
            "join request is missing request secret; ask the other device to refresh its QR code"
        ));
    }

    Err(anyhow!("unsupported join request link"))
}

fn unix_timestamp_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
