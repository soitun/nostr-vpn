use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, anyhow};
pub use nostr_identity::{
    IDENTITY_GRAPH_ROSTER_TYPE, KIND_NOSTR_IDENTITY_ROSTER_OP,
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA, NostrIdentityCapabilities,
    NostrIdentityDeviceApprovalReceipt, NostrIdentityError, NostrIdentityFacet, NostrIdentityId,
    NostrIdentityKeyPurpose, NostrIdentityRosterOp, SignedIdentityLinkRequest,
    SignedNostrIdentityRosterOp, build_nostr_identity_device_approval_receipt_event,
    parse_identity_link_request_event_for_invite_pubkey,
    parse_nostr_identity_device_approval_receipt_event,
    parse_nostr_identity_device_approval_receipt_roster_op,
};
use nostr_sdk::prelude::{Event, JsonUtil, Keys, PublicKey};
use serde::{Deserialize, Serialize};

use crate::fips_control::{NetworkRoster, SignedRoster};

/// Canonical NostrIdentity/AppKey roster events live in `nostr-identity` kind 7368.
///
/// Nostr VPN keeps its legacy signed network roster as kind 30388. This module
/// is only a bridge projection from that roster into canonical-shaped identity
/// metadata; it does not write, parse, or mutate 7368 events.
pub const CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND: u16 = KIND_NOSTR_IDENTITY_ROSTER_OP;
pub const CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE: &str = IDENTITY_GRAPH_ROSTER_TYPE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NostrIdentityAppKeyFacet {
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub purposes: BTreeSet<NostrIdentityKeyPurpose>,
    #[serde(default)]
    pub capabilities: NostrIdentityCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl NostrIdentityAppKeyFacet {
    #[must_use]
    pub fn app_key(
        pubkey: String,
        profile_id: Option<NostrIdentityId>,
        capabilities: NostrIdentityCapabilities,
    ) -> Self {
        Self {
            pubkey,
            profile_id,
            purposes: [NostrIdentityKeyPurpose::AppKey].into_iter().collect(),
            capabilities,
            label: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterAppKeyRole {
    Member,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterAppKeyIdentity {
    pub role: RosterAppKeyRole,
    pub facet: NostrIdentityAppKeyFacet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_network_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityDeviceApprovalSidecar {
    pub roster_op_event: Event,
    pub receipt_event: Event,
}

pub fn roster_app_key_identities(
    roster: &NetworkRoster,
    profile_ids_by_pubkey: &BTreeMap<String, NostrIdentityId>,
) -> Result<Vec<RosterAppKeyIdentity>> {
    let admins = normalize_pubkey_set(&roster.admins, "admin")?;
    let devices = normalize_pubkey_set(&roster.devices, "member")?;
    let aliases = normalize_aliases(&roster.aliases)?;
    let profile_ids_by_pubkey = normalize_profile_ids(profile_ids_by_pubkey)?;

    let mut pubkeys = devices;
    pubkeys.extend(admins.iter().cloned());

    Ok(pubkeys
        .into_iter()
        .map(|pubkey| {
            let role = if admins.contains(&pubkey) {
                RosterAppKeyRole::Admin
            } else {
                RosterAppKeyRole::Member
            };
            let capabilities = match role {
                RosterAppKeyRole::Admin => NostrIdentityCapabilities::app_admin(),
                RosterAppKeyRole::Member => NostrIdentityCapabilities::app_writer(),
            };
            RosterAppKeyIdentity {
                role,
                facet: NostrIdentityAppKeyFacet::app_key(
                    pubkey.clone(),
                    profile_ids_by_pubkey.get(&pubkey).copied(),
                    capabilities,
                ),
                legacy_network_alias: aliases.get(&pubkey).cloned(),
            }
        })
        .collect())
}

pub fn signed_roster_app_key_identities(
    signed_roster: &SignedRoster,
    profile_ids_by_pubkey: &BTreeMap<String, NostrIdentityId>,
) -> Result<Vec<RosterAppKeyIdentity>> {
    signed_roster.verify()?;
    roster_app_key_identities(&signed_roster.roster()?, profile_ids_by_pubkey)
}

pub fn build_roster_app_key_sidecar_event(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    pubkey: &str,
    role: RosterAppKeyRole,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    created_at: u64,
) -> Result<Event> {
    let pubkey = normalize_pubkey(pubkey, "sidecar app key")?;
    let created_at_i64 = i64::try_from(created_at).context("sidecar created_at overflows i64")?;
    let capabilities = match role {
        RosterAppKeyRole::Admin => NostrIdentityCapabilities::app_admin(),
        RosterAppKeyRole::Member => NostrIdentityCapabilities::app_writer(),
    };
    let facet = NostrIdentityFacet::app_key(pubkey, created_at_i64, None, capabilities)
        .with_profile_id(profile_id);

    nostr_identity::build_nostr_identity_roster_op_event(
        signer_keys,
        profile_id,
        parents,
        actor_seq,
        NostrIdentityRosterOp::AddFacet { facet },
        created_at_i64,
    )
    .map_err(|error| anyhow!("failed to build NostrIdentity roster sidecar: {error}"))
}

pub fn parse_roster_app_key_sidecar_event(event: &Event) -> Result<Option<RosterAppKeyIdentity>> {
    let signed = nostr_identity::parse_nostr_identity_roster_op_event(event)
        .map_err(|error| anyhow!("failed to parse NostrIdentity roster sidecar: {error}"))?;
    let NostrIdentityRosterOp::AddFacet { facet } = signed.content.op else {
        return Ok(None);
    };
    if !facet.purposes.contains(&NostrIdentityKeyPurpose::AppKey) {
        return Ok(None);
    }
    let pubkey = normalize_pubkey(&facet.pubkey, "sidecar app key")?;
    let role = if facet.capabilities.can_admin_profile {
        RosterAppKeyRole::Admin
    } else {
        RosterAppKeyRole::Member
    };
    Ok(Some(RosterAppKeyIdentity {
        role,
        facet: NostrIdentityAppKeyFacet {
            pubkey,
            profile_id: facet.profile_id,
            purposes: facet.purposes,
            capabilities: facet.capabilities,
            label: facet.label,
        },
        legacy_network_alias: None,
    }))
}

pub fn build_identity_link_request_from_manual_npub(
    joining_keys: &Keys,
    profile_id: NostrIdentityId,
    admin_npub: &str,
    invite_npub: &str,
    client_nonce: impl Into<String>,
    label: Option<String>,
    requested_at: u64,
) -> Result<Event> {
    let admin_pubkey = normalize_pubkey(admin_npub, "identity link request admin")?;
    let invite_pubkey = normalize_pubkey(invite_npub, "identity link request invite")?;
    nostr_identity::build_identity_link_request_event(
        joining_keys,
        profile_id.as_uuid(),
        admin_pubkey,
        invite_pubkey,
        client_nonce.into(),
        label,
        requested_at,
    )
    .map_err(|error| anyhow!("failed to build NostrIdentity link request: {error}"))
}

pub fn build_device_approval_sidecar(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    request_pubkey: &str,
    device_app_key_pubkey: &str,
    request_secret: &str,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    approved_at: u64,
) -> Result<NostrIdentityDeviceApprovalSidecar> {
    let request_pubkey = normalize_pubkey(request_pubkey, "approval request")?;
    let device_app_key_pubkey = normalize_pubkey(device_app_key_pubkey, "approval device")?;
    let approved_at_i64 =
        i64::try_from(approved_at).context("approval approved_at overflows i64")?;
    let roster_op_event = build_roster_app_key_sidecar_event(
        signer_keys,
        profile_id,
        &device_app_key_pubkey,
        RosterAppKeyRole::Member,
        parents,
        actor_seq,
        approved_at,
    )?;
    let receipt = NostrIdentityDeviceApprovalReceipt {
        schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
        profile_id,
        request_pubkey,
        device_app_key_pubkey,
        approved_by_pubkey: signer_keys.public_key().to_hex(),
        approved_at: approved_at_i64,
        request_secret: request_secret.trim().to_string(),
        subject_pubkey: None,
        roster_op_id: Some(roster_op_event.id.to_hex()),
        signed_roster_event: Some(roster_op_event.as_json()),
    };
    let receipt_event = build_nostr_identity_device_approval_receipt_event(signer_keys, receipt)
        .map_err(|error| anyhow!("failed to build NostrIdentity approval receipt: {error}"))?;
    Ok(NostrIdentityDeviceApprovalSidecar {
        roster_op_event,
        receipt_event,
    })
}

fn normalize_pubkey_set(values: &[String], role: &str) -> Result<BTreeSet<String>> {
    values
        .iter()
        .map(|value| normalize_pubkey(value, role))
        .collect()
}

fn normalize_profile_ids(
    profile_ids_by_pubkey: &BTreeMap<String, NostrIdentityId>,
) -> Result<BTreeMap<String, NostrIdentityId>> {
    profile_ids_by_pubkey
        .iter()
        .map(|(pubkey, profile_id)| Ok((normalize_pubkey(pubkey, "profile")?, *profile_id)))
        .collect()
}

fn normalize_aliases(aliases: &HashMap<String, String>) -> Result<BTreeMap<String, String>> {
    aliases
        .iter()
        .filter_map(|(pubkey, alias)| {
            let alias = alias.trim();
            (!alias.is_empty()).then(|| {
                normalize_pubkey(pubkey, "alias").map(|pubkey| (pubkey, alias.to_string()))
            })
        })
        .collect()
}

fn normalize_pubkey(value: &str, role: &str) -> Result<String> {
    PublicKey::parse(value.trim())
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| anyhow!("invalid roster {role} pubkey: {error}"))
}
