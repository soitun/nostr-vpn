use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, anyhow};
pub use nostr_identity::{
    CreateNostrIdentityDeviceApprovalRequestOptions, IDENTITY_GRAPH_ROSTER_TYPE,
    KIND_NOSTR_IDENTITY_ROSTER_OP, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE, NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_TYPE,
    NostrIdentityCapabilities, NostrIdentityDeviceApprovalBootstrap,
    NostrIdentityDeviceApprovalReceipt, NostrIdentityDeviceApprovalRequest, NostrIdentityError,
    NostrIdentityFacet, NostrIdentityId, NostrIdentityKeyPurpose, NostrIdentityRosterOp,
    NostrIdentityRosterOpContent, SignedIdentityLinkRequest, SignedNostrIdentityRosterOp,
    build_nostr_identity_device_approval_receipt_event,
    build_nostr_identity_device_approval_request_event,
    create_nostr_identity_device_approval_request, encode_nostr_identity_device_approval_bootstrap,
    nostr_identity_device_approval_bootstrap, nostr_identity_device_approval_bootstrap_has_prefix,
    parse_identity_link_request_event_for_invite_pubkey,
    parse_nostr_identity_device_approval_bootstrap,
    parse_nostr_identity_device_approval_receipt_event,
    parse_nostr_identity_device_approval_receipt_roster_op,
    parse_nostr_identity_device_approval_request_event, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster,
};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::prelude::{Event, JsonUtil, Keys, PublicKey};
use nostr_sdk::{EventBuilder, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};

use crate::fips_control::{NetworkRoster, SignedRoster};

/// Canonical NostrIdentity/AppKey roster events live in `nostr-identity` kind 7368.
///
/// Nostr VPN keeps its legacy signed network roster as kind 30388. This module
/// bridges that roster into canonical-shaped identity metadata and provides
/// scan/link approval helpers without replacing the accepted legacy roster.
pub const CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND: u16 = KIND_NOSTR_IDENTITY_ROSTER_OP;
pub const CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE: &str = IDENTITY_GRAPH_ROSTER_TYPE;
pub const CANONICAL_NETWORK_NAME_FACT: &str = "network_name";
pub const LEGACY_SIGNED_NETWORK_ROSTER_KIND: u16 = 30_388;
pub const NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE: &str = "nostr-vpn.join-request-approval-context";
pub const NOSTR_VPN_JOIN_APPROVAL_CONTEXT_SCHEMA: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterIdentityBridgeSource {
    LegacySignedNetworkRoster,
    NostrIdentityRosterOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsedIdentityRosterBridgeEvent {
    pub source: RosterIdentityBridgeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_name: Option<String>,
    pub op_id: String,
    pub signer_pubkey: String,
    pub signed_at: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<RosterAppKeyIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityDeviceApprovalSidecar {
    pub canonical_roster_events: Vec<Event>,
    pub receipt_event: Event,
}

impl NostrIdentityDeviceApprovalSidecar {
    #[must_use]
    pub fn approved_device_roster_op(&self) -> Option<&Event> {
        self.canonical_roster_events.last()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityDeviceApprovalSidecarRequest {
    pub profile_id: NostrIdentityId,
    pub network_name: Option<String>,
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    pub request_secret: String,
    pub canonical_profile_is_fresh: bool,
    pub approved_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NostrVpnJoinApprovalContext {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    pub approved_by_pubkey: String,
    pub approved_at: i64,
    pub request_secret: String,
    pub mesh_network_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_op_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonical_roster_events: Vec<String>,
    pub signed_network_roster_event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_node_pubkey: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrVpnJoinApprovalContextRequest {
    pub profile_id: NostrIdentityId,
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    pub request_secret: String,
    pub mesh_network_id: String,
    pub network_name: Option<String>,
    pub roster_op_id: Option<String>,
    pub canonical_roster_events: Vec<String>,
    pub signed_network_roster_event: String,
    pub exit_node_pubkey: Option<String>,
    pub approved_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterAppKeySidecarEventRequest {
    pub profile_id: NostrIdentityId,
    pub pubkey: String,
    pub role: RosterAppKeyRole,
    pub parents: Vec<String>,
    pub actor_seq: Option<u64>,
    pub created_at: u64,
    pub network_name: Option<String>,
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

pub fn parse_identity_roster_bridge_event(
    event: &Event,
    profile_ids_by_pubkey: &BTreeMap<String, NostrIdentityId>,
) -> Result<Option<ParsedIdentityRosterBridgeEvent>> {
    match u16::from(event.kind) {
        LEGACY_SIGNED_NETWORK_ROSTER_KIND => {
            let signed_roster = SignedRoster::from_event(event.clone())?;
            let roster = signed_roster.roster()?;
            Ok(Some(ParsedIdentityRosterBridgeEvent {
                source: RosterIdentityBridgeSource::LegacySignedNetworkRoster,
                network_id: Some(signed_roster.network_id()?),
                network_name: normalized_network_name(&roster.network_name),
                op_id: signed_roster.content_hash(),
                signer_pubkey: signed_roster.signer_pubkey_hex()?,
                signed_at: signed_roster.signed_at(),
                identities: roster_app_key_identities(&roster, profile_ids_by_pubkey)?,
            }))
        }
        CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND => {
            if !event.tags.iter().any(|tag| {
                let parts = tag.as_slice();
                parts.first().is_some_and(|name| name == "type")
                    && parts
                        .get(1)
                        .is_some_and(|value| value == CANONICAL_NOSTR_IDENTITY_ROSTER_TYPE)
            }) {
                return Ok(None);
            }
            let Some(identity) = parse_roster_app_key_sidecar_event(event)? else {
                return Ok(None);
            };
            let network_name = canonical_roster_event_network_name(event)?;
            Ok(Some(ParsedIdentityRosterBridgeEvent {
                source: RosterIdentityBridgeSource::NostrIdentityRosterOp,
                network_id: None,
                network_name,
                op_id: event.id.to_hex(),
                signer_pubkey: event.pubkey.to_hex(),
                signed_at: event.created_at.as_secs(),
                identities: vec![identity],
            }))
        }
        _ => Ok(None),
    }
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
    build_roster_app_key_sidecar_event_with_network_name(
        signer_keys,
        RosterAppKeySidecarEventRequest {
            profile_id,
            pubkey: pubkey.to_string(),
            role,
            parents,
            actor_seq,
            created_at,
            network_name: None,
        },
    )
}

pub fn build_roster_app_key_sidecar_event_with_network_name(
    signer_keys: &Keys,
    request: RosterAppKeySidecarEventRequest,
) -> Result<Event> {
    let pubkey = normalize_pubkey(&request.pubkey, "sidecar app key")?;
    let capabilities = match request.role {
        RosterAppKeyRole::Admin => nostr_identity::IDENTITY_ADMIN_CAPABILITIES,
        RosterAppKeyRole::Member => nostr_identity::IDENTITY_APP_KEY_CAPABILITIES,
    };
    let key = nostr_identity::IdentityKey {
        pubkey,
        subject: Some(request.profile_id.as_uuid()),
        purposes: vec![nostr_identity::IDENTITY_PURPOSE_APP.to_string()],
        capabilities: capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        added_at: request.created_at,
        label: None,
    };

    nostr_identity::build_identity_roster_op_event_with_options(
        signer_keys,
        request.profile_id.as_uuid(),
        nostr_identity::IdentityRosterOp::AddKey { key },
        nostr_identity::BuildIdentityRosterOpEventOptions {
            parents: request.parents,
            actor_seq: request.actor_seq,
            client_nonce: uuid::Uuid::new_v4().to_string(),
            created_at: request.created_at,
            extension_facts: network_name_extension_facts(request.network_name.as_deref()),
        },
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
    request: NostrIdentityDeviceApprovalSidecarRequest,
) -> Result<NostrIdentityDeviceApprovalSidecar> {
    let request_pubkey = normalize_pubkey(&request.request_pubkey, "approval request")?;
    let device_app_key_pubkey =
        normalize_pubkey(&request.device_app_key_pubkey, "approval device")?;
    let network_name = request.network_name.clone();
    let approved_at_i64 =
        i64::try_from(request.approved_at).context("approval approved_at overflows i64")?;
    let canonical_roster_events = if request.canonical_profile_is_fresh {
        if request.approved_at == 0 {
            return Err(anyhow!(
                "fresh canonical profile approval timestamp must be positive"
            ));
        }
        let genesis = build_roster_app_key_sidecar_event_with_network_name(
            signer_keys,
            RosterAppKeySidecarEventRequest {
                profile_id: request.profile_id,
                pubkey: signer_keys.public_key().to_hex(),
                role: RosterAppKeyRole::Admin,
                parents: Vec::new(),
                actor_seq: None,
                created_at: request.approved_at - 1,
                network_name: network_name.clone(),
            },
        )?;
        let member = build_roster_app_key_sidecar_event_with_network_name(
            signer_keys,
            RosterAppKeySidecarEventRequest {
                profile_id: request.profile_id,
                pubkey: device_app_key_pubkey.clone(),
                role: RosterAppKeyRole::Member,
                parents: vec![genesis.id.to_hex()],
                actor_seq: None,
                created_at: request.approved_at,
                network_name,
            },
        )?;
        let events = vec![genesis, member];
        validate_canonical_roster_chain(
            request.profile_id,
            signer_keys.public_key().to_hex().as_str(),
            &device_app_key_pubkey,
            &events,
        )?;
        events
    } else {
        Vec::new()
    };
    let approved_device_roster_op = canonical_roster_events.last();
    let receipt = NostrIdentityDeviceApprovalReceipt {
        schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
        profile_id: request.profile_id,
        request_pubkey,
        device_app_key_pubkey,
        approved_by_pubkey: signer_keys.public_key().to_hex(),
        approved_at: approved_at_i64,
        request_secret: request.request_secret.trim().to_string(),
        subject_pubkey: None,
        roster_op_id: approved_device_roster_op.map(|event| event.id.to_hex()),
        signed_roster_event: approved_device_roster_op.map(JsonUtil::as_json),
    };
    let receipt_event = build_nostr_identity_device_approval_receipt_event(signer_keys, receipt)
        .map_err(|error| anyhow!("failed to build NostrIdentity approval receipt: {error}"))?;
    Ok(NostrIdentityDeviceApprovalSidecar {
        canonical_roster_events,
        receipt_event,
    })
}

pub fn build_device_approval_sidecar_from_shared_approval(
    signer_keys: &Keys,
    approval_request: &NostrIdentityDeviceApprovalRequest,
    approval_content: NostrIdentityRosterOpContent,
    mut canonical_roster_events: Vec<Event>,
) -> Result<NostrIdentityDeviceApprovalSidecar> {
    validate_shared_device_approval_content(signer_keys, approval_request, &approval_content)?;
    let publish_canonical_member = !canonical_roster_events.is_empty();
    if publish_canonical_member {
        let member = nostr_identity::build_nostr_identity_roster_op_event_with_client_nonce(
            signer_keys,
            approval_content.profile_id,
            approval_content.parents.clone(),
            approval_content.actor_seq,
            approval_content.op.clone(),
            approval_content.created_at,
            approval_content.client_nonce.clone(),
            None,
        )
        .map_err(|error| anyhow!("failed to sign shared device approval AddFacet: {error}"))?;
        let signed = parse_nostr_identity_roster_op_event(&member)
            .map_err(|error| anyhow!("failed to parse shared device approval AddFacet: {error}"))?;
        if signed.content != approval_content {
            return Err(anyhow!(
                "signed device approval AddFacet differs from shared approval content"
            ));
        }
        canonical_roster_events.push(member);
        validate_canonical_roster_chain(
            approval_content.profile_id,
            signer_keys.public_key().to_hex().as_str(),
            &approval_request.device_app_key_pubkey,
            &canonical_roster_events,
        )?;
    }

    let approved_device_roster_op = if publish_canonical_member {
        canonical_roster_events.last()
    } else {
        None
    };
    let receipt = NostrIdentityDeviceApprovalReceipt {
        schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
        profile_id: approval_content.profile_id,
        request_pubkey: normalize_pubkey(&approval_request.request_pubkey, "approval request")?,
        device_app_key_pubkey: normalize_pubkey(
            &approval_request.device_app_key_pubkey,
            "approval device",
        )?,
        approved_by_pubkey: signer_keys.public_key().to_hex(),
        approved_at: approval_content.created_at,
        request_secret: approval_request.request_secret.trim().to_string(),
        subject_pubkey: None,
        roster_op_id: approved_device_roster_op.map(|event| event.id.to_hex()),
        signed_roster_event: approved_device_roster_op.map(JsonUtil::as_json),
    };
    let receipt_event = build_nostr_identity_device_approval_receipt_event(signer_keys, receipt)
        .map_err(|error| anyhow!("failed to build NostrIdentity approval receipt: {error}"))?;
    Ok(NostrIdentityDeviceApprovalSidecar {
        canonical_roster_events,
        receipt_event,
    })
}

fn validate_shared_device_approval_content(
    signer_keys: &Keys,
    approval_request: &NostrIdentityDeviceApprovalRequest,
    approval_content: &NostrIdentityRosterOpContent,
) -> Result<()> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if approval_content.actor_pubkey != signer_pubkey {
        return Err(anyhow!("shared device approval actor mismatch"));
    }
    if let Some(profile_id) = approval_request.profile_id
        && approval_content.profile_id != profile_id
    {
        return Err(anyhow!("shared device approval profile mismatch"));
    }
    let NostrIdentityRosterOp::AddFacet { facet } = &approval_content.op else {
        return Err(anyhow!("shared device approval is not an AddFacet op"));
    };
    let device_pubkey =
        normalize_pubkey(&approval_request.device_app_key_pubkey, "approval device")?;
    if facet.pubkey != device_pubkey
        || facet.profile_id != Some(approval_content.profile_id)
        || !facet.purposes.contains(&NostrIdentityKeyPurpose::AppKey)
        || facet.capabilities != NostrIdentityCapabilities::app_writer()
    {
        return Err(anyhow!(
            "shared device approval AddFacet does not match requested AppKey"
        ));
    }
    Ok(())
}

pub fn build_nostr_vpn_join_approval_context_event(
    signer_keys: &Keys,
    request: NostrVpnJoinApprovalContextRequest,
) -> Result<Event> {
    let context = normalize_nostr_vpn_join_approval_context(signer_keys, request)?;
    let request_pubkey = PublicKey::parse(&context.request_pubkey)
        .context("invalid Nostr VPN approval context request pubkey")?;
    let encrypted = nip44::encrypt(
        signer_keys.secret_key(),
        &request_pubkey,
        serde_json::to_string(&context).context("failed to encode Nostr VPN approval context")?,
        Nip44Version::V2,
    )
    .map_err(|error| anyhow!("failed to encrypt Nostr VPN approval context: {error}"))?;
    let profile_id = context.profile_id.to_string();
    let approved_at = u64::try_from(context.approved_at)
        .context("Nostr VPN approval context approved_at is negative")?;
    EventBuilder::new(Kind::from(CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND), encrypted)
        .tag(
            Tag::parse(["type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE])
                .map_err(|error| anyhow!("failed to tag Nostr VPN approval context: {error}"))?,
        )
        .tag(
            Tag::parse(["p", context.request_pubkey.as_str()])
                .map_err(|error| anyhow!("failed to tag Nostr VPN approval context: {error}"))?,
        )
        .tag(
            Tag::parse(["i", profile_id.as_str(), "subject"])
                .map_err(|error| anyhow!("failed to tag Nostr VPN approval context: {error}"))?,
        )
        .custom_created_at(Timestamp::from(approved_at))
        .sign_with_keys(signer_keys)
        .map_err(|error| anyhow!("failed to sign Nostr VPN approval context: {error}"))
}

pub fn parse_nostr_vpn_join_approval_context_event(
    event: &Event,
    request_keys: &Keys,
) -> Result<NostrVpnJoinApprovalContext> {
    event
        .verify()
        .map_err(|error| anyhow!("invalid Nostr VPN approval context signature: {error}"))?;
    if u16::from(event.kind) != CANONICAL_NOSTR_IDENTITY_FACT_OP_KIND {
        return Err(anyhow!("Nostr VPN approval context has invalid kind"));
    }
    require_event_tag(event, "type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE)?;
    let request_pubkey = request_keys.public_key().to_hex();
    require_event_tag(event, "p", &request_pubkey)?;
    let plaintext = nip44::decrypt(request_keys.secret_key(), &event.pubkey, &event.content)
        .map_err(|error| anyhow!("failed to decrypt Nostr VPN approval context: {error}"))?;
    let context: NostrVpnJoinApprovalContext =
        serde_json::from_str(&plaintext).context("failed to parse Nostr VPN approval context")?;
    validate_nostr_vpn_join_approval_context(&context)?;
    if context.request_pubkey != request_pubkey {
        return Err(anyhow!("Nostr VPN approval context request mismatch"));
    }
    if context.approved_by_pubkey != event.pubkey.to_hex() {
        return Err(anyhow!("Nostr VPN approval context signer mismatch"));
    }
    let event_created_at = i64::try_from(event.created_at.as_secs())
        .context("Nostr VPN approval context created_at overflows i64")?;
    if context.approved_at != event_created_at {
        return Err(anyhow!("Nostr VPN approval context approved_at mismatch"));
    }
    require_subject_tag(event, &context.profile_id.to_string())?;
    Ok(context)
}

fn normalize_nostr_vpn_join_approval_context(
    signer_keys: &Keys,
    request: NostrVpnJoinApprovalContextRequest,
) -> Result<NostrVpnJoinApprovalContext> {
    let approved_at = i64::try_from(request.approved_at)
        .context("Nostr VPN approval context approved_at overflows i64")?;
    let context = NostrVpnJoinApprovalContext {
        schema: NOSTR_VPN_JOIN_APPROVAL_CONTEXT_SCHEMA,
        profile_id: request.profile_id,
        request_pubkey: normalize_pubkey(&request.request_pubkey, "Nostr VPN approval request")?,
        device_app_key_pubkey: normalize_pubkey(
            &request.device_app_key_pubkey,
            "Nostr VPN approval device",
        )?,
        approved_by_pubkey: signer_keys.public_key().to_hex(),
        approved_at,
        request_secret: request.request_secret.trim().to_string(),
        mesh_network_id: request.mesh_network_id.trim().to_string(),
        network_name: request
            .network_name
            .and_then(|value| normalized_network_name(&value)),
        roster_op_id: request.roster_op_id.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        }),
        canonical_roster_events: request.canonical_roster_events,
        signed_network_roster_event: request.signed_network_roster_event.trim().to_string(),
        exit_node_pubkey: request
            .exit_node_pubkey
            .map(|value| normalize_pubkey(&value, "Nostr VPN approval exit"))
            .transpose()?,
    };
    validate_nostr_vpn_join_approval_context(&context)?;
    Ok(context)
}

fn validate_nostr_vpn_join_approval_context(context: &NostrVpnJoinApprovalContext) -> Result<()> {
    if context.schema != NOSTR_VPN_JOIN_APPROVAL_CONTEXT_SCHEMA {
        return Err(anyhow!(
            "unsupported Nostr VPN approval context schema {}",
            context.schema
        ));
    }
    normalize_pubkey(&context.request_pubkey, "Nostr VPN approval request")?;
    normalize_pubkey(&context.device_app_key_pubkey, "Nostr VPN approval device")?;
    normalize_pubkey(&context.approved_by_pubkey, "Nostr VPN approval signer")?;
    if context.approved_at < 0 {
        return Err(anyhow!(
            "Nostr VPN approval context approved_at is negative"
        ));
    }
    if context.request_secret.trim().is_empty() {
        return Err(anyhow!(
            "Nostr VPN approval context request_secret is empty"
        ));
    }
    if context.mesh_network_id.trim().is_empty() {
        return Err(anyhow!(
            "Nostr VPN approval context mesh_network_id is empty"
        ));
    }
    if let Some(roster_op_id) = &context.roster_op_id
        && !is_hex_event_id(roster_op_id)
    {
        return Err(anyhow!(
            "Nostr VPN approval context roster_op_id is invalid"
        ));
    }
    validate_nostr_vpn_network_roster(context)?;
    if context.canonical_roster_events.is_empty() {
        if context.roster_op_id.is_some() {
            return Err(anyhow!(
                "Nostr VPN approval context roster_op_id has no canonical roster chain"
            ));
        }
    } else {
        validate_canonical_roster_event_jsons(context)?;
    }
    Ok(())
}

fn require_event_tag(event: &Event, name: &str, value: &str) -> Result<()> {
    if event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().is_some_and(|part| part == name)
            && parts.get(1).is_some_and(|part| part == value)
    }) {
        return Ok(());
    }
    Err(anyhow!("Nostr VPN approval context missing {name} tag"))
}

fn require_subject_tag(event: &Event, profile_id: &str) -> Result<()> {
    if event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().is_some_and(|part| part == "i")
            && parts.get(1).is_some_and(|part| part == profile_id)
            && parts.get(2).is_some_and(|part| part == "subject")
    }) {
        return Ok(());
    }
    Err(anyhow!(
        "Nostr VPN approval context missing profile subject tag"
    ))
}

fn validate_nostr_vpn_network_roster(context: &NostrVpnJoinApprovalContext) -> Result<()> {
    if context.signed_network_roster_event.trim().is_empty() {
        return Err(anyhow!(
            "Nostr VPN approval context signed network roster is empty"
        ));
    }
    let event = Event::from_json(&context.signed_network_roster_event)
        .context("invalid Nostr VPN approval signed network roster event")?;
    let signed = SignedRoster::from_event(event)?;
    if signed.signer_pubkey_hex()? != context.approved_by_pubkey {
        return Err(anyhow!("Nostr VPN approval network roster signer mismatch"));
    }
    if signed.network_id()? != context.mesh_network_id {
        return Err(anyhow!("Nostr VPN approval network roster id mismatch"));
    }
    let approved_at = u64::try_from(context.approved_at)
        .context("Nostr VPN approval context approved_at is negative")?;
    if signed.signed_at() != approved_at {
        return Err(anyhow!(
            "Nostr VPN approval network roster timestamp mismatch"
        ));
    }
    let roster = signed.roster()?;
    let network_name = normalized_network_name(&roster.network_name);
    if network_name != context.network_name {
        return Err(anyhow!("Nostr VPN approval network name mismatch"));
    }
    let admins = normalize_pubkey_set(&roster.admins, "approval admin")?;
    let devices = normalize_pubkey_set(&roster.devices, "approval member")?;
    if !admins.contains(&context.approved_by_pubkey) {
        return Err(anyhow!(
            "Nostr VPN approval signer is not a network roster admin"
        ));
    }
    if !devices.contains(&context.device_app_key_pubkey)
        && !admins.contains(&context.device_app_key_pubkey)
    {
        return Err(anyhow!(
            "Nostr VPN approval device is missing from network roster"
        ));
    }
    if let Some(exit_node) = &context.exit_node_pubkey
        && !devices.contains(exit_node)
        && !admins.contains(exit_node)
    {
        return Err(anyhow!(
            "Nostr VPN approval exit is missing from network roster"
        ));
    }
    Ok(())
}

fn validate_canonical_roster_event_jsons(context: &NostrVpnJoinApprovalContext) -> Result<()> {
    let events = context
        .canonical_roster_events
        .iter()
        .map(|value| Event::from_json(value).context("invalid canonical roster event JSON"))
        .collect::<Result<Vec<_>>>()?;
    validate_canonical_roster_chain(
        context.profile_id,
        &context.approved_by_pubkey,
        &context.device_app_key_pubkey,
        &events,
    )?;
    let expected_op_id = context
        .roster_op_id
        .as_deref()
        .ok_or_else(|| anyhow!("Nostr VPN approval canonical roster op id is missing"))?;
    if events.last().map(|event| event.id.to_hex()).as_deref() != Some(expected_op_id) {
        return Err(anyhow!(
            "Nostr VPN approval canonical roster op id mismatch"
        ));
    }
    Ok(())
}

fn validate_canonical_roster_chain(
    profile_id: NostrIdentityId,
    approved_by_pubkey: &str,
    device_app_key_pubkey: &str,
    events: &[Event],
) -> Result<()> {
    let signed = events
        .iter()
        .map(|event| {
            parse_nostr_identity_roster_op_event(event)
                .map_err(|error| anyhow!("invalid canonical roster event: {error}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if signed
        .iter()
        .any(|op| op.content.profile_id != profile_id || op.signer_pubkey != approved_by_pubkey)
    {
        return Err(anyhow!("canonical roster profile or signer mismatch"));
    }
    let projection = project_nostr_identity_roster(profile_id, signed);
    if projection.accepted_op_ids.len() != events.len() || !projection.rejected_op_ids.is_empty() {
        return Err(anyhow!("canonical roster chain does not project cleanly"));
    }
    if !projection
        .active_facets
        .get(approved_by_pubkey)
        .is_some_and(|facet| facet.capabilities.can_admin_profile)
    {
        return Err(anyhow!("canonical roster chain has no admin genesis"));
    }
    if !projection
        .active_facets
        .get(device_app_key_pubkey)
        .is_some_and(|facet| facet.purposes.contains(&NostrIdentityKeyPurpose::AppKey))
    {
        return Err(anyhow!(
            "canonical roster chain does not add approval device"
        ));
    }
    Ok(())
}

fn is_hex_event_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn build_device_approval_for_link_request(
    signer_keys: &Keys,
    link_request: &SignedIdentityLinkRequest,
    _parents: Vec<String>,
    _actor_seq: Option<u64>,
    approved_at: u64,
) -> Result<NostrIdentityDeviceApprovalSidecar> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if signer_pubkey != link_request.content.admin_pubkey {
        return Err(anyhow!(
            "identity link request admin does not match approval signer"
        ));
    }
    if link_request.signer_pubkey != link_request.content.joining_pubkey {
        return Err(anyhow!(
            "identity link request signer does not match joining device"
        ));
    }
    build_device_approval_sidecar(
        signer_keys,
        NostrIdentityDeviceApprovalSidecarRequest {
            profile_id: NostrIdentityId::from_uuid(link_request.content.identity),
            network_name: None,
            request_pubkey: link_request.signer_pubkey.clone(),
            device_app_key_pubkey: link_request.content.joining_pubkey.clone(),
            request_secret: link_request.content.client_nonce.clone(),
            canonical_profile_is_fresh: false,
            approved_at,
        },
    )
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

fn network_name_extension_facts(network_name: Option<&str>) -> Vec<nostr_identity::Fact> {
    let Some(network_name) = network_name.and_then(normalized_network_name) else {
        return Vec::new();
    };
    vec![nostr_identity::fact(
        CANONICAL_NETWORK_NAME_FACT,
        &[&network_name],
    )]
}

fn canonical_roster_event_network_name(event: &Event) -> Result<Option<String>> {
    let op = nostr_identity::parse_fact_op_event(event)
        .map_err(|error| anyhow!("failed to parse NostrIdentity roster facts: {error}"))?;
    let mut names = op
        .facts
        .iter()
        .filter(|fact| fact.predicate == CANONICAL_NETWORK_NAME_FACT)
        .filter_map(|fact| fact.values.first())
        .filter_map(|value| normalized_network_name(value))
        .collect::<BTreeSet<_>>();
    match names.len() {
        0 => Ok(None),
        1 => Ok(names.pop_first()),
        _ => Err(anyhow!(
            "canonical roster event has conflicting network_name facts"
        )),
    }
}

fn normalized_network_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
