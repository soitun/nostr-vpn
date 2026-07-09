use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::identity_bridge::{
    encode_compact_nostr_identity_device_approval_request as encode_compact_join_request,
    parse_compact_nostr_identity_device_approval_request as parse_compact_join_request,
};

pub(crate) const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinRequestQrCodeOrLink {
    pub pubkey_hex: String,
    pub node_name: String,
}

pub(crate) fn own_join_request_qr_code_or_link(config: &AppConfig) -> Result<String> {
    let requester = config.own_nostr_pubkey_hex()?;
    let mut link = encode_compact_join_request(&requester, Some(JOIN_REQUEST_LINK_PREFIX))
        .context("failed to encode join request")?;
    let node_name = config.node_name.trim();
    if !node_name.is_empty() {
        link.push_str("&name=");
        link.push_str(&percent_encode_query_component(node_name));
    }
    Ok(link)
}

pub(crate) fn parse_join_request_qr_code_or_link(value: &str) -> Result<JoinRequestQrCodeOrLink> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("join request is empty"));
    }

    let Some(request) = parse_compact_join_request(trimmed, &[JOIN_REQUEST_LINK_PREFIX])
        .context("failed to parse join request")?
    else {
        return Err(anyhow!("unsupported join request link"));
    };
    let requester = normalize_nostr_pubkey(&request.device_app_key_pubkey)?;
    let node_name = query_value(trimmed, "name")
        .or_else(|| query_value(trimmed, "nodeName"))
        .or_else(|| query_value(trimmed, "node_name"))
        .unwrap_or_default()
        .trim()
        .to_string();
    Ok(JoinRequestQrCodeOrLink {
        pubkey_hex: requester,
        node_name,
    })
}

fn query_value(input: &str, key: &str) -> Option<String> {
    let query = input.split_once('?')?.1;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == key {
            return Some(percent_decode_query_component(value));
        }
    }
    None
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn percent_decode_query_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
