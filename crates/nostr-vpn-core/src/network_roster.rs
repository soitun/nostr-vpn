use std::collections::HashMap;

use anyhow::Result;
use nostr_sdk::prelude::{PublicKey, ToBech32};

use crate::config::{
    PendingInboundJoinRequest, PendingOutboundJoinRequest, normalize_nostr_pubkey,
};

pub(crate) fn normalize_network_admins(
    admins: Vec<String>,
    own_pubkey_hex: Option<&str>,
    join_request_admin: &str,
) -> Vec<String> {
    let mut normalized = admins
        .into_iter()
        .filter_map(|admin| normalize_nostr_pubkey(&admin).ok())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    if !normalized.is_empty() {
        return normalized;
    }

    if let Ok(join_admin) = normalize_nostr_pubkey(join_request_admin) {
        return vec![join_admin];
    }

    own_pubkey_hex
        .map(|pubkey| vec![pubkey.to_string()])
        .unwrap_or_default()
}

pub(crate) fn normalize_shared_roster_devices(
    devices: Vec<String>,
    own_pubkey_hex: Option<&str>,
) -> Result<Vec<String>> {
    let mut normalized = devices
        .into_iter()
        .map(|device| normalize_nostr_pubkey(&device))
        .collect::<Result<Vec<_>>>()?;
    if let Some(own_pubkey_hex) = own_pubkey_hex {
        normalized.retain(|device| device != own_pubkey_hex);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

pub(crate) fn canonical_npub_key(value: &str) -> Option<String> {
    let normalized = normalize_nostr_pubkey(value).ok()?;
    Some(
        PublicKey::from_hex(&normalized)
            .ok()
            .and_then(|public_key| public_key.to_bech32().ok())
            .unwrap_or(normalized),
    )
}

pub(crate) fn normalize_outbound_join_request(
    request: Option<PendingOutboundJoinRequest>,
    _devices: &[String],
) -> Option<PendingOutboundJoinRequest> {
    let request = request?;
    let recipient = normalize_nostr_pubkey(&request.recipient).ok()?;
    Some(PendingOutboundJoinRequest {
        recipient,
        requested_at: request.requested_at,
    })
}

pub(crate) fn canonicalize_outbound_join_request(
    request: Option<PendingOutboundJoinRequest>,
) -> Option<PendingOutboundJoinRequest> {
    let request = request?;
    let recipient = canonical_npub_key(&request.recipient)?;
    Some(PendingOutboundJoinRequest {
        recipient,
        requested_at: request.requested_at,
    })
}

pub(crate) fn normalize_inbound_join_requests(
    requests: Vec<PendingInboundJoinRequest>,
    devices: &[String],
) -> Vec<PendingInboundJoinRequest> {
    let mut deduped = HashMap::new();

    for request in requests {
        let Ok(requester) = normalize_nostr_pubkey(&request.requester) else {
            continue;
        };
        if devices.iter().any(|device| device == &requester) {
            continue;
        }

        let normalized = PendingInboundJoinRequest {
            requester: requester.clone(),
            requester_node_name: request.requester_node_name.trim().to_string(),
            requested_at: request.requested_at,
        };
        if deduped
            .get(&requester)
            .map(|existing: &PendingInboundJoinRequest| {
                existing.requested_at >= normalized.requested_at
            })
            .unwrap_or(false)
        {
            continue;
        }
        deduped.insert(requester, normalized);
    }

    let mut normalized = deduped.into_values().collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.requester.cmp(&right.requester));
    normalized
}

pub(crate) fn canonicalize_inbound_join_requests(
    requests: Vec<PendingInboundJoinRequest>,
) -> Vec<PendingInboundJoinRequest> {
    requests
        .into_iter()
        .filter_map(|request| {
            let requester = canonical_npub_key(&request.requester)?;
            Some(PendingInboundJoinRequest {
                requester,
                requester_node_name: request.requester_node_name,
                requested_at: request.requested_at,
            })
        })
        .collect()
}
