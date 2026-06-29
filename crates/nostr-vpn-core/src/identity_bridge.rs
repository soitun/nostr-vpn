use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Result, anyhow};
pub use nostr_identity::{
    IDENTITY_GRAPH_ROSTER_TYPE, KIND_NOSTR_IDENTITY_ROSTER_OP, NostrIdentityCapabilities,
    NostrIdentityId, NostrIdentityKeyPurpose,
};
use nostr_sdk::prelude::PublicKey;
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
