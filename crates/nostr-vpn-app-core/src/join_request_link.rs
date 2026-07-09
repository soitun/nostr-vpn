use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalRequest, parse_nostr_identity_device_approval_request,
};

pub const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestQrCodeOrLink {
    pub pubkey_hex: String,
    pub node_name: String,
    pub approval_request: NostrIdentityDeviceApprovalRequest,
}

pub fn own_join_request_qr_code_or_link(config: &AppConfig) -> Result<String> {
    config
        .pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
        .context("failed to encode join request")
}

pub fn parse_join_request_qr_code_or_link(value: &str) -> Result<JoinRequestQrCodeOrLink> {
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

    Err(anyhow!("unsupported join request link"))
}
