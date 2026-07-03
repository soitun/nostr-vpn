use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_vpn_core::config::{
    AppConfig, NetworkConfig, PendingOutboundJoinRequest, normalize_nostr_pubkey,
    normalize_runtime_network_id,
};
use serde::{Deserialize, Serialize};

use crate::invite::to_npub;

pub(crate) const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";
const JOIN_REQUEST_LINK_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JoinRequestQrCodeOrLink {
    pub v: u8,
    pub network_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_secret: String,
    pub requester_npub: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recipient_npub: String,
    pub requested_at: u64,
}

pub(crate) fn active_join_request_qr_code_or_link(
    config: &AppConfig,
    network: &NetworkConfig,
    request: &PendingOutboundJoinRequest,
) -> Result<String> {
    let requester = config.own_nostr_pubkey_hex()?;
    encode_join_request_qr_code_or_link(&JoinRequestQrCodeOrLink {
        v: JOIN_REQUEST_LINK_VERSION,
        network_id: normalize_runtime_network_id(&network.network_id),
        invite_secret: network.invite_secret.trim().to_string(),
        requester_npub: to_npub(&requester),
        node_name: config.node_name.trim().to_string(),
        recipient_npub: to_npub(&request.recipient),
        requested_at: request.requested_at,
    })
}

pub(crate) fn encode_join_request_qr_code_or_link(
    request: &JoinRequestQrCodeOrLink,
) -> Result<String> {
    let bytes = serde_json::to_vec(request).context("failed to encode join request JSON")?;
    Ok(format!(
        "{JOIN_REQUEST_LINK_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub(crate) fn parse_join_request_qr_code_or_link(value: &str) -> Result<JoinRequestQrCodeOrLink> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("join request is empty"));
    }

    let mut request = if trimmed.starts_with('{') {
        serde_json::from_str::<JoinRequestQrCodeOrLink>(trimmed)
            .context("failed to parse join request JSON")?
    } else {
        let payload = trimmed
            .strip_prefix(JOIN_REQUEST_LINK_PREFIX)
            .unwrap_or(trimmed);
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .context("failed to decode join request payload")?;
        serde_json::from_slice::<JoinRequestQrCodeOrLink>(&decoded)
            .context("failed to parse join request payload")?
    };

    if request.v != JOIN_REQUEST_LINK_VERSION {
        return Err(anyhow!(
            "unsupported join request version {}; expected {}",
            request.v,
            JOIN_REQUEST_LINK_VERSION
        ));
    }
    request.network_id = normalize_runtime_network_id(&request.network_id);
    if request.network_id.is_empty() {
        return Err(anyhow!("join request network id is empty"));
    }
    request.invite_secret = request.invite_secret.trim().to_string();
    let requester = normalize_nostr_pubkey(&request.requester_npub)?;
    request.requester_npub = to_npub(&requester);
    request.node_name = request.node_name.trim().to_string();
    if !request.recipient_npub.trim().is_empty() {
        let recipient = normalize_nostr_pubkey(&request.recipient_npub)?;
        request.recipient_npub = to_npub(&recipient);
    }
    Ok(request)
}
