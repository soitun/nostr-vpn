use anyhow::{Context, Result, anyhow};
use nostr_identity::{
    NostrIdentityDeviceApprovalReceipt, nostr_identity_device_approval_relay_resource,
    nostr_identity_device_approval_request_relays,
    parse_nostr_identity_device_approval_receipt_event_for_request,
};
use nostr_sdk::prelude::{Event, JsonUtil, Keys};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, normalize_nostr_pubkey, normalize_runtime_network_id};
use crate::fips_control::SignedRoster;
use crate::identity_bridge::{
    CreateNostrIdentityDeviceApprovalRequestOptions, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE,
    NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE, NostrIdentityDeviceApprovalRequest, NostrIdentityId,
    NostrVpnJoinApprovalContext, create_nostr_identity_device_approval_request,
    encode_nostr_identity_device_approval_request,
    parse_nostr_identity_device_approval_receipt_roster_op,
    parse_nostr_vpn_join_approval_context_event,
};

pub const FIPS_JOIN_REQUEST_RETRY_SECS: u64 = 10;
pub const NOSTR_VPN_JOIN_REQUEST_TYPE: &str = "nostr-vpn.join-request";
pub const NOSTR_VPN_JOIN_APPROVAL_RELAY: &str = "wss://temp.iris.to";
pub const MAX_NOSTR_JOIN_APPROVAL_AGE_SECS: u64 = 7 * 24 * 60 * 60;
pub const MAX_NOSTR_JOIN_APPROVAL_FUTURE_SECS: u64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedNostrJoinApproval {
    pub profile_id: NostrIdentityId,
    pub network_id: String,
    pub approved_by_pubkey: String,
    pub approved_at: u64,
}

struct VerifiedNostrJoinApproval {
    context: NostrVpnJoinApprovalContext,
    signed_roster: SignedRoster,
}

struct VerifiedReceiptCandidate {
    signer_pubkey: String,
    receipt: NostrIdentityDeviceApprovalReceipt,
}

struct VerifiedContextCandidate {
    signer_pubkey: String,
    context: NostrVpnJoinApprovalContext,
}

enum VerifiedApprovalCandidate {
    Receipt(VerifiedReceiptCandidate),
    Context(VerifiedContextCandidate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingNostrJoinRequest {
    pub request: NostrIdentityDeviceApprovalRequest,
    pub request_private_key: String,
}

impl PendingNostrJoinRequest {
    pub fn request_keys(&self) -> Result<Keys> {
        let keys = Keys::parse(self.request_private_key.trim())
            .context("pending join request private key is invalid")?;
        if keys.public_key().to_hex() != self.request.request_pubkey {
            return Err(anyhow!(
                "pending join request private key does not match request pubkey"
            ));
        }
        Ok(keys)
    }

    pub fn validate_for_device(&self, device_app_key_pubkey: &str) -> Result<()> {
        let expected_device = normalize_nostr_pubkey(device_app_key_pubkey)?;
        let encoded = encode_nostr_identity_device_approval_request(&self.request, None)
            .map_err(|error| anyhow!("pending join request is invalid: {error}"))?;
        if encoded.is_empty() {
            return Err(anyhow!("pending join request encoding is empty"));
        }
        if self.request.device_app_key_pubkey != expected_device {
            return Err(anyhow!(
                "pending join request device AppKey does not match local identity"
            ));
        }
        if self.request.request_type.as_deref() != Some(NOSTR_VPN_JOIN_REQUEST_TYPE) {
            return Err(anyhow!("pending join request has invalid request type"));
        }
        if self.request.request_pubkey == self.request.device_app_key_pubkey {
            return Err(anyhow!(
                "pending join request must use a separate ephemeral request key"
            ));
        }
        let relays = nostr_identity_device_approval_request_relays(&self.request)
            .map_err(|error| anyhow!("pending join request approval relay is invalid: {error}"))?;
        if relays.as_slice() != [NOSTR_VPN_JOIN_APPROVAL_RELAY] {
            return Err(anyhow!(
                "pending join request must use the Nostr VPN approval relay"
            ));
        }
        self.request_keys()?;
        Ok(())
    }

    pub fn encode(&self, prefix: &str) -> Result<String> {
        encode_nostr_identity_device_approval_request(&self.request, Some(prefix))
            .map_err(|error| anyhow!("failed to encode pending join request: {error}"))
    }
}

impl AppConfig {
    pub fn ensure_pending_nostr_join_request(&mut self, requested_at: u64) -> Result<bool> {
        let device_keys = self.nostr_keys()?;
        let device_pubkey = device_keys.public_key().to_hex();
        if let Some(pending) = &self.pending_nostr_join_request {
            let relays = nostr_identity_device_approval_request_relays(&pending.request).map_err(
                |error| anyhow!("pending join request approval relay is invalid: {error}"),
            )?;
            if !relays.is_empty() {
                pending.validate_for_device(&device_pubkey)?;
                return Ok(false);
            }
            self.pending_nostr_join_request = None;
        }
        let requested_at =
            i64::try_from(requested_at).context("pending join request timestamp overflows i64")?;
        let node_name = self.node_name.trim();
        let local = create_nostr_identity_device_approval_request(
            &device_keys,
            CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: None,
                requested_at,
                request_type: Some(NOSTR_VPN_JOIN_REQUEST_TYPE.to_string()),
                resources: vec![
                    nostr_identity_device_approval_relay_resource(NOSTR_VPN_JOIN_APPROVAL_RELAY)
                        .map_err(|error| {
                            anyhow!("failed to build join approval relay resource: {error}")
                        })?,
                ],
                expires_at: None,
                profile_id: self.nostr.identity_profile_id,
                admin_app_key_pubkey: None,
                label: (!node_name.is_empty()).then(|| node_name.to_string()),
            },
        )
        .map_err(|error| anyhow!("failed to create pending join request: {error}"))?;
        let pending = PendingNostrJoinRequest {
            request: local.request,
            request_private_key: local.request_keys.secret_key().to_secret_hex(),
        };
        pending.validate_for_device(&device_pubkey)?;
        self.pending_nostr_join_request = Some(pending);
        Ok(true)
    }

    pub fn pending_nostr_join_request_link(&self, prefix: &str) -> Result<String> {
        let pending = self
            .pending_nostr_join_request
            .as_ref()
            .ok_or_else(|| anyhow!("no pending Nostr join request"))?;
        pending.validate_for_device(&self.own_nostr_pubkey_hex()?)?;
        pending.encode(prefix)
    }

    pub fn clear_pending_nostr_join_request(&mut self) -> bool {
        self.pending_nostr_join_request.take().is_some()
    }

    pub fn apply_nostr_join_approval_events(
        &mut self,
        events: &[Event],
        now: u64,
    ) -> Result<Option<AppliedNostrJoinApproval>> {
        let Some(pending) = self.pending_nostr_join_request.as_ref() else {
            return Ok(None);
        };
        pending.validate_for_device(&self.own_nostr_pubkey_hex()?)?;
        let verified_approvals = verify_nostr_join_approval_events(pending, events, now)?;
        for verified in verified_approvals {
            if let Ok((updated, applied)) = self.stage_verified_nostr_join_approval(verified) {
                *self = updated;
                return Ok(Some(applied));
            }
        }
        Ok(None)
    }

    fn stage_verified_nostr_join_approval(
        &self,
        verified: VerifiedNostrJoinApproval,
    ) -> Result<(Self, AppliedNostrJoinApproval)> {
        let context = &verified.context;
        let mut updated = self.clone();
        let matching_network = updated
            .networks
            .iter()
            .find(|network| {
                normalize_runtime_network_id(&network.network_id) == context.mesh_network_id
            })
            .map(|network| network.id.clone());
        let network_entry_id = if let Some(network_id) = matching_network {
            let network = updated
                .network_by_id(&network_id)
                .ok_or_else(|| anyhow!("approved network disappeared"))?;
            if !network.admins.is_empty()
                && !network
                    .admins
                    .iter()
                    .any(|admin| admin == &context.approved_by_pubkey)
            {
                return Err(anyhow!(
                    "Nostr join approval signer is not a configured network admin"
                ));
            }
            network_id
        } else {
            if updated.networks.iter().any(|network| network.enabled) {
                return Err(anyhow!(
                    "Nostr join approval targets a different active network"
                ));
            }
            let network_id =
                updated.add_network(context.network_name.as_deref().unwrap_or("Network"));
            let network = updated
                .network_by_id_mut(&network_id)
                .ok_or_else(|| anyhow!("failed to create approved network"))?;
            network.network_id = context.mesh_network_id.clone();
            network.admins = vec![context.approved_by_pubkey.clone()];
            network.shared_roster_updated_at = 0;
            network.shared_roster_signed_by.clear();
            updated.set_network_enabled(&network_id, true)?;
            network_id
        };

        if !updated.apply_verified_admin_signed_shared_roster(&verified.signed_roster)? {
            return Err(anyhow!(
                "Nostr join approval network roster was not applied"
            ));
        }
        updated.nostr.identity_profile_id = Some(context.profile_id);
        updated.exit_node = context.exit_node_pubkey.clone().unwrap_or_default();
        updated.wireguard_exit.enabled = false;
        updated.pending_nostr_join_request = None;
        updated.ensure_defaults();
        updated.set_network_enabled(&network_entry_id, true)?;
        let applied = AppliedNostrJoinApproval {
            profile_id: context.profile_id,
            network_id: context.mesh_network_id.clone(),
            approved_by_pubkey: context.approved_by_pubkey.clone(),
            approved_at: u64::try_from(context.approved_at)
                .context("Nostr join approval timestamp is negative")?,
        };
        Ok((updated, applied))
    }
}

fn verify_nostr_join_approval_events(
    pending: &PendingNostrJoinRequest,
    events: &[Event],
    now: u64,
) -> Result<Vec<VerifiedNostrJoinApproval>> {
    let request_keys = pending.request_keys()?;
    let mut receipts = Vec::new();
    let mut contexts = Vec::new();
    for event in events {
        match parse_nostr_join_approval_candidate(pending, &request_keys, event, now) {
            Ok(Some(VerifiedApprovalCandidate::Receipt(receipt))) => receipts.push(receipt),
            Ok(Some(VerifiedApprovalCandidate::Context(context))) => contexts.push(context),
            Ok(None) | Err(_) => {}
        }
    }

    let mut verified = Vec::new();
    for receipt in &receipts {
        for context in &contexts {
            if let Ok(approval) = verify_nostr_join_approval_pair(receipt, context) {
                verified.push(approval);
            }
        }
    }
    Ok(verified)
}

pub(crate) fn is_valid_nostr_join_approval_candidate(
    pending: &PendingNostrJoinRequest,
    event: &Event,
    now: u64,
) -> Result<bool> {
    let request_keys = pending.request_keys()?;
    Ok(
        parse_nostr_join_approval_candidate(pending, &request_keys, event, now)
            .is_ok_and(|value| value.is_some()),
    )
}

fn parse_nostr_join_approval_candidate(
    pending: &PendingNostrJoinRequest,
    request_keys: &Keys,
    event: &Event,
    now: u64,
) -> Result<Option<VerifiedApprovalCandidate>> {
    if event.kind.as_u16() != 7_368 {
        return Ok(None);
    }
    let is_receipt = event_has_tag(event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE);
    let is_context = event_has_tag(event, "type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE);
    match (is_receipt, is_context) {
        (true, false) => {
            let receipt = parse_nostr_identity_device_approval_receipt_event_for_request(
                event,
                request_keys,
                &pending.request,
            )
            .map_err(|error| anyhow!("invalid Nostr join approval receipt: {error}"))?;
            validate_join_approval_freshness(pending, receipt.approved_at, now)?;
            Ok(Some(VerifiedApprovalCandidate::Receipt(
                VerifiedReceiptCandidate {
                    signer_pubkey: event.pubkey.to_hex(),
                    receipt,
                },
            )))
        }
        (false, true) => {
            let context = parse_nostr_vpn_join_approval_context_event(event, request_keys)?;
            if context.request_pubkey != pending.request.request_pubkey {
                return Err(anyhow!("Nostr join approval request pubkey mismatch"));
            }
            if context.device_app_key_pubkey != pending.request.device_app_key_pubkey {
                return Err(anyhow!("Nostr join approval device AppKey mismatch"));
            }
            if context.request_secret != pending.request.request_secret {
                return Err(anyhow!("Nostr join approval request secret mismatch"));
            }
            if let Some(request_profile_id) = pending.request.profile_id
                && context.profile_id != request_profile_id
            {
                return Err(anyhow!("Nostr join approval changed requested profile"));
            }
            validate_join_approval_freshness(pending, context.approved_at, now)?;
            Ok(Some(VerifiedApprovalCandidate::Context(
                VerifiedContextCandidate {
                    signer_pubkey: event.pubkey.to_hex(),
                    context,
                },
            )))
        }
        _ => Ok(None),
    }
}

fn verify_nostr_join_approval_pair(
    receipt: &VerifiedReceiptCandidate,
    context: &VerifiedContextCandidate,
) -> Result<VerifiedNostrJoinApproval> {
    if receipt.receipt.approved_by_pubkey != context.context.approved_by_pubkey
        || receipt.signer_pubkey != context.signer_pubkey
    {
        return Err(anyhow!("Nostr join approval signer mismatch"));
    }
    if receipt.receipt.profile_id != context.context.profile_id {
        return Err(anyhow!("Nostr join approval profile mismatch"));
    }
    if receipt.receipt.approved_at != context.context.approved_at {
        return Err(anyhow!("Nostr join approval timestamp mismatch"));
    }

    if receipt.receipt.roster_op_id != context.context.roster_op_id {
        return Err(anyhow!("Nostr join approval canonical roster op mismatch"));
    }
    match (
        &receipt.receipt.signed_roster_event,
        &context.context.roster_op_id,
    ) {
        (Some(receipt_event_json), Some(roster_op_id)) => {
            let roster_op =
                parse_nostr_identity_device_approval_receipt_roster_op(&receipt.receipt)
                    .map_err(|error| anyhow!("invalid approval canonical roster op: {error}"))?;
            if roster_op.op_id != *roster_op_id
                || !context
                    .context
                    .canonical_roster_events
                    .iter()
                    .any(|event_json| {
                        Event::from_json(event_json)
                            .ok()
                            .is_some_and(|event| event.id.to_hex() == *roster_op_id)
                    })
            {
                return Err(anyhow!(
                    "Nostr join approval canonical roster context mismatch"
                ));
            }
            let embedded = Event::from_json(receipt_event_json)
                .context("invalid embedded approval canonical roster event")?;
            if embedded.id.to_hex() != *roster_op_id {
                return Err(anyhow!("Nostr join approval embedded roster op mismatch"));
            }
        }
        (None, None) if context.context.canonical_roster_events.is_empty() => {}
        _ => {
            return Err(anyhow!(
                "Nostr join approval has incomplete canonical roster claims"
            ));
        }
    }

    let signed_roster_event = Event::from_json(&context.context.signed_network_roster_event)
        .context("invalid approved network roster event JSON")?;
    let signed_roster = SignedRoster::from_event(signed_roster_event)?;
    let roster = signed_roster.roster()?;
    let approved_device = normalize_nostr_pubkey(&context.context.device_app_key_pubkey)?;
    if !roster
        .devices
        .iter()
        .chain(roster.admins.iter())
        .filter_map(|member| normalize_nostr_pubkey(member).ok())
        .any(|member| member == approved_device)
    {
        return Err(anyhow!(
            "Nostr join approval signed roster does not contain the approved device AppKey"
        ));
    }
    Ok(VerifiedNostrJoinApproval {
        context: context.context.clone(),
        signed_roster,
    })
}

fn event_has_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().is_some_and(|part| part == name)
            && parts.get(1).is_some_and(|part| part == value)
    })
}

fn validate_join_approval_freshness(
    pending: &PendingNostrJoinRequest,
    approved_at: i64,
    now: u64,
) -> Result<()> {
    let approved_at =
        u64::try_from(approved_at).context("Nostr join approval timestamp is negative")?;
    let requested_at = u64::try_from(pending.request.requested_at)
        .context("pending Nostr join request timestamp is negative")?;
    if approved_at < requested_at {
        return Err(anyhow!("Nostr join approval predates the pending request"));
    }
    if approved_at > now.saturating_add(MAX_NOSTR_JOIN_APPROVAL_FUTURE_SECS) {
        return Err(anyhow!("Nostr join approval is too far in the future"));
    }
    if now > approved_at.saturating_add(MAX_NOSTR_JOIN_APPROVAL_AGE_SECS) {
        return Err(anyhow!("Nostr join approval is stale"));
    }
    if let Some(expires_at) = pending.request.expires_at {
        let expires_at =
            u64::try_from(expires_at).context("pending Nostr join request expiry is negative")?;
        if approved_at > expires_at || now > expires_at {
            return Err(anyhow!("pending Nostr join request has expired"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshJoinRequest {
    pub network_id: String,
    #[serde(default)]
    pub invite_secret: String,
    #[serde(default)]
    pub requester_node_name: String,
}

pub fn normalize_join_request(request: MeshJoinRequest) -> Result<MeshJoinRequest> {
    let network_id = normalize_runtime_network_id(&request.network_id);
    if network_id.is_empty() {
        return Err(anyhow!("mesh join request network_id must not be empty"));
    }

    Ok(MeshJoinRequest {
        network_id,
        invite_secret: request.invite_secret.trim().to_string(),
        requester_node_name: request.requester_node_name.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_request_normalizes_network_id_and_node_name() {
        let request = normalize_join_request(MeshJoinRequest {
            network_id: "  Mesh Home  ".to_string(),
            invite_secret: " invite-secret ".to_string(),
            requester_node_name: " alice-phone ".to_string(),
        })
        .expect("normalize");

        assert_eq!(request.network_id, "Mesh Home");
        assert_eq!(request.invite_secret, "invite-secret");
        assert_eq!(request.requester_node_name, "alice-phone");
    }
}
