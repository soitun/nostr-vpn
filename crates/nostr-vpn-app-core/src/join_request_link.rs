use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::identity_bridge::{
    NostrIdentityDeviceApprovalBootstrap, parse_nostr_identity_device_approval_bootstrap,
};

pub const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestQrCodeOrLink {
    pub pubkey_hex: String,
    pub bootstrap: NostrIdentityDeviceApprovalBootstrap,
    pub fips_route_npub: Option<String>,
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

    let (request, fips_route_npub) = match trimmed.split_once("?r=") {
        Some((request, route)) if !route.is_empty() && !route.contains('&') => {
            let route = URL_SAFE_NO_PAD.decode(route)?;
            let route = PublicKey::from_slice(&route)?.to_bech32()?;
            (request, Some(route))
        }
        Some(_) => return Err(anyhow!("invalid FIPS return route")),
        None => (trimmed, None),
    };

    if let Some(bootstrap) =
        parse_nostr_identity_device_approval_bootstrap(request, &[JOIN_REQUEST_LINK_PREFIX])
            .context("failed to parse join request")?
    {
        let requester = normalize_nostr_pubkey(&bootstrap.device_app_key_npub)?;
        return Ok(JoinRequestQrCodeOrLink {
            pubkey_hex: requester,
            bootstrap,
            fips_route_npub,
        });
    }

    Err(anyhow!("unsupported join request link"))
}
