use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::identity_bridge::{
    encode_compact_nostr_identity_device_approval_request,
    parse_compact_nostr_identity_device_approval_request,
};

use crate::invite::to_npub;

pub(crate) const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinRequestQrCodeOrLink {
    pub requester_npub: String,
    pub requester_pubkey_hex: String,
}

pub(crate) fn own_join_request_qr_code_or_link(config: &AppConfig) -> Result<String> {
    let requester = config.own_nostr_pubkey_hex()?;
    encode_compact_nostr_identity_device_approval_request(
        &requester,
        Some(JOIN_REQUEST_LINK_PREFIX),
    )
    .context("failed to encode NostrIdentity device approval request")
}

pub(crate) fn parse_join_request_qr_code_or_link(value: &str) -> Result<JoinRequestQrCodeOrLink> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("join request is empty"));
    }

    let Some(request) =
        parse_compact_nostr_identity_device_approval_request(trimmed, &[JOIN_REQUEST_LINK_PREFIX])
            .context("failed to parse NostrIdentity device approval request")?
    else {
        return Err(anyhow!("unsupported join request link"));
    };
    let requester = normalize_nostr_pubkey(&request.device_app_key_pubkey)?;
    Ok(JoinRequestQrCodeOrLink {
        requester_npub: to_npub(&requester),
        requester_pubkey_hex: requester,
    })
}
