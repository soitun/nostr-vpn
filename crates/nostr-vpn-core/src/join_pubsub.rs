use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use nostr_pubsub::{
    Filter, FipsPubsubWireCodec, FipsPubsubWireMessage, PublicKey, SubscriptionId, VerifiedEvent,
};
use nostr_sdk::prelude::{Alphabet, Event, JsonUtil, Keys, Kind, SingleLetterTag, Timestamp};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, normalize_nostr_pubkey};
use crate::identity_bridge::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA, NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE,
    NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE, NostrIdentityDeviceApprovalAppliedAck,
    build_nostr_identity_device_approval_applied_ack_event,
    parse_nostr_identity_device_approval_applied_ack_event,
};
use crate::join_requests::{AppliedNostrJoinApproval, is_valid_nostr_join_approval_candidate};

/// FSP DataPacket service port reserved for Nostr join approval pubsub.
pub const NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT: u16 = 7_368;
pub const NOSTR_JOIN_PUBSUB_INITIAL_EVENT_LIMIT: usize = 8;
const MAX_BUFFERED_APPROVAL_EVENTS: usize = 4;
const DIRECT_APPROVAL_OUTBOX_VERSION: u8 = 1;
const MAX_DIRECT_APPROVAL_EVENTS: usize = 4;
const WEBVM_APPROVAL_ROUTE_MAGIC: &[u8; 8] = b"NVPNFWD1";
pub const NOSTR_JOIN_APPROVAL_APPLIED_ACK_MAGIC: &[u8; 8] = b"NVPNACK1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedNostrJoinApproval {
    pub version: u8,
    pub recipient_npub: String,
    pub fips_route_npub: Option<String>,
    pub request_pubkey: String,
    pub events: Vec<Event>,
}

/// One port-addressed FSP DataPacket payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrJoinFipsPubsubDatagram {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: Vec<u8>,
}

impl NostrJoinFipsPubsubDatagram {
    fn new(payload: Vec<u8>) -> Self {
        Self {
            source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
            destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
            payload,
        }
    }
}

pub fn delivered_approval_event_datagram(
    request_pubkey: &str,
    event: &Event,
) -> Result<NostrJoinFipsPubsubDatagram> {
    let request_pubkey = normalize_nostr_pubkey(request_pubkey)
        .context("invalid Nostr join approval request pubkey")?;
    if !is_targeted_approval_event(event, &request_pubkey) {
        return Err(anyhow!("event is not a targeted Nostr join approval"));
    }
    event
        .verify()
        .map_err(|error| anyhow!("invalid signed Nostr join approval event: {error}"))?;
    let subscription_suffix = request_pubkey
        .get(..16)
        .ok_or_else(|| anyhow!("Nostr join approval request pubkey is too short"))?;
    let subscription_id = SubscriptionId::new(format!("nvpn-join-{subscription_suffix}"));
    let event = VerifiedEvent::try_from(event.clone())
        .map_err(|error| anyhow!("invalid signed Nostr join approval event: {error}"))?;
    let payload = FipsPubsubWireCodec::default()
        .encode_frame(&FipsPubsubWireMessage::deliver(subscription_id, event))?;
    Ok(NostrJoinFipsPubsubDatagram::new(payload))
}

pub fn routed_approval_event_datagram(
    recipient_npub: &str,
    request_pubkey: &str,
    event: &Event,
) -> Result<NostrJoinFipsPubsubDatagram> {
    let recipient = normalize_nostr_pubkey(recipient_npub)
        .context("invalid routed Nostr join approval recipient")?;
    let recipient = hex::decode(recipient).context("invalid routed approval recipient bytes")?;
    let direct = delivered_approval_event_datagram(request_pubkey, event)?;
    let mut payload = Vec::with_capacity(
        WEBVM_APPROVAL_ROUTE_MAGIC.len() + recipient.len() + direct.payload.len(),
    );
    payload.extend_from_slice(WEBVM_APPROVAL_ROUTE_MAGIC);
    payload.extend_from_slice(&recipient);
    payload.extend_from_slice(&direct.payload);
    Ok(NostrJoinFipsPubsubDatagram::new(payload))
}

pub fn approval_applied_ack_datagram(
    device_app_key_keys: &Keys,
    applied: &AppliedNostrJoinApproval,
) -> Result<NostrJoinFipsPubsubDatagram> {
    let event = build_nostr_identity_device_approval_applied_ack_event(
        device_app_key_keys,
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: applied.request_pubkey.clone(),
            device_app_key_pubkey: applied.device_app_key_pubkey.clone(),
            approval_event_id: applied.approval_event_id.clone(),
            approved_by_pubkey: applied.approved_by_pubkey.clone(),
            applied_at: i64::try_from(applied.applied_at)
                .context("join approval applied timestamp overflows i64")?,
        },
    )
    .map_err(|error| anyhow!("failed to build join approval applied ack: {error}"))?;
    let event_json = event.as_json();
    let mut payload =
        Vec::with_capacity(NOSTR_JOIN_APPROVAL_APPLIED_ACK_MAGIC.len() + event_json.len());
    payload.extend_from_slice(NOSTR_JOIN_APPROVAL_APPLIED_ACK_MAGIC);
    payload.extend_from_slice(event_json.as_bytes());
    Ok(NostrJoinFipsPubsubDatagram::new(payload))
}

pub fn parse_approval_applied_ack_datagram(
    datagram: &NostrJoinFipsPubsubDatagram,
) -> Result<NostrIdentityDeviceApprovalAppliedAck> {
    if datagram.source_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
        || datagram.destination_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
    {
        return Err(anyhow!(
            "Nostr join approval applied ack used the wrong FSP service port"
        ));
    }
    let event_json = datagram
        .payload
        .strip_prefix(NOSTR_JOIN_APPROVAL_APPLIED_ACK_MAGIC)
        .ok_or_else(|| anyhow!("invalid Nostr join approval applied ack magic"))?;
    let event_json =
        std::str::from_utf8(event_json).context("Nostr join approval applied ack is not UTF-8")?;
    let event = Event::from_json(event_json)
        .context("invalid Nostr join approval applied ack event JSON")?;
    parse_nostr_identity_device_approval_applied_ack_event(&event)
        .map_err(|error| anyhow!("invalid Nostr join approval applied ack: {error}"))
}

#[must_use]
pub fn approval_applied_ack_matches_queued(
    ack: &NostrIdentityDeviceApprovalAppliedAck,
    queued: &QueuedNostrJoinApproval,
) -> bool {
    let Ok(recipient) = normalize_nostr_pubkey(&queued.recipient_npub) else {
        return false;
    };
    ack.device_app_key_pubkey == recipient
        && ack.request_pubkey == queued.request_pubkey
        && queued.events.iter().any(|event| {
            event.id.to_hex() == ack.approval_event_id
                && event.pubkey.to_hex() == ack.approved_by_pubkey
                && event_has_tag(event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE)
                && event_has_tag(event, "p", &queued.request_pubkey)
        })
}

#[must_use]
pub fn approval_event_datagram_matches_ack(
    datagram: &NostrJoinFipsPubsubDatagram,
    ack: &NostrIdentityDeviceApprovalAppliedAck,
) -> bool {
    if datagram.source_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
        || datagram.destination_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
    {
        return false;
    }
    let Ok(FipsPubsubWireMessage::Event { event, .. }) =
        FipsPubsubWireCodec::default().decode_frame(&datagram.payload)
    else {
        return false;
    };
    let event = event.into_event();
    event.pubkey.to_hex() == ack.approved_by_pubkey
        && event_has_tag(&event, "p", &ack.request_pubkey)
        && (event_has_tag(&event, "type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE)
            || (event.id.to_hex() == ack.approval_event_id
                && event_has_tag(&event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE)))
}

pub fn direct_join_approval_outbox_directory(config_path: &Path) -> PathBuf {
    let mut directory_name = config_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.toml".into());
    directory_name.push(".join-approval-outbox");
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(directory_name)
}

pub fn queue_direct_join_approval(
    config_path: &Path,
    recipient_npub: &str,
    fips_route_npub: Option<&str>,
    request_pubkey: &str,
    events: &[Event],
) -> Result<PathBuf> {
    let recipient_npub =
        normalize_nostr_pubkey(recipient_npub).context("invalid direct join approval recipient")?;
    let request_pubkey = normalize_nostr_pubkey(request_pubkey)
        .context("invalid direct join approval request pubkey")?;
    let fips_route_npub = fips_route_npub
        .map(normalize_nostr_pubkey)
        .transpose()
        .context("invalid direct join approval FIPS return route")?;
    let events = events
        .iter()
        .filter(|event| is_targeted_approval_event(event, &request_pubkey))
        .cloned()
        .collect::<Vec<_>>();
    if events.is_empty() || events.len() > MAX_DIRECT_APPROVAL_EVENTS {
        return Err(anyhow!(
            "direct join approval requires 1..={MAX_DIRECT_APPROVAL_EVENTS} targeted events"
        ));
    }
    for event in &events {
        delivered_approval_event_datagram(&request_pubkey, event)?;
    }
    let queued = QueuedNostrJoinApproval {
        version: DIRECT_APPROVAL_OUTBOX_VERSION,
        recipient_npub,
        fips_route_npub,
        request_pubkey: request_pubkey.clone(),
        events,
    };
    let directory = direct_join_approval_outbox_directory(config_path);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let event_id = queued
        .events
        .last()
        .map(|event| event.id.to_hex())
        .ok_or_else(|| anyhow!("direct join approval has no event id"))?;
    let destination = directory.join(format!("{request_pubkey}-{event_id}.json"));
    if destination.exists() {
        return Ok(destination);
    }
    let temporary = directory.join(format!(
        ".{request_pubkey}-{}-{event_id}.tmp",
        std::process::id()
    ));
    let bytes = serde_json::to_vec(&queued).context("failed to encode direct join approval")?;
    write_private_file(&temporary, &bytes)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to queue {}", destination.display()));
    }
    Ok(destination)
}

pub fn load_direct_join_approvals(config_path: &Path) -> Vec<(PathBuf, QueuedNostrJoinApproval)> {
    let directory = direct_join_approval_outbox_directory(config_path);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            eprintln!("failed to scan {}: {error}", directory.display());
            return Vec::new();
        }
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.truncate(8);
    paths
        .into_iter()
        .filter_map(|path| {
            match fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<QueuedNostrJoinApproval>(&bytes)
                        .with_context(|| format!("failed to decode {}", path.display()))
                }) {
                Ok(queued) if queued.version == DIRECT_APPROVAL_OUTBOX_VERSION => {
                    Some((path, queued))
                }
                Ok(_) => {
                    eprintln!(
                        "discarding unsupported direct join approval {}",
                        path.display()
                    );
                    let _ = fs::remove_file(path);
                    None
                }
                Err(error) => {
                    eprintln!("discarding invalid direct join approval: {error:#}");
                    let _ = fs::remove_file(path);
                    None
                }
            }
        })
        .collect()
}

#[cfg(unix)]
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

/// Join-side Nostr pubsub protocol endpoint for an FSP DataPacket transport.
///
/// The transport owns routing and delivery. Each datagram payload is exactly one
/// bounded Nostr JSON `REQ`, `CLOSE`, or delivered `EVENT` frame.
#[derive(Debug, Clone)]
pub struct NostrJoinFipsPubsubClient {
    codec: FipsPubsubWireCodec,
    subscription_id: SubscriptionId,
    request_pubkey: String,
    buffered_events: Vec<Event>,
}

impl NostrJoinFipsPubsubClient {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Self::with_codec(config, FipsPubsubWireCodec::default())
    }

    pub fn with_codec(config: &AppConfig, codec: FipsPubsubWireCodec) -> Result<Self> {
        let pending = config
            .pending_nostr_join_request
            .as_ref()
            .ok_or_else(|| anyhow!("no pending Nostr join request"))?;
        pending.validate_for_device(&config.own_nostr_pubkey_hex()?)?;
        let request_pubkey = pending.request.request_pubkey.clone();
        let subscription_suffix = request_pubkey
            .get(..16)
            .ok_or_else(|| anyhow!("pending Nostr join request pubkey is too short"))?;
        Ok(Self {
            codec,
            subscription_id: SubscriptionId::new(format!("nvpn-join-{subscription_suffix}")),
            request_pubkey,
            buffered_events: Vec::with_capacity(2),
        })
    }

    #[must_use]
    pub fn subscription_id(&self) -> &SubscriptionId {
        &self.subscription_id
    }

    #[must_use]
    pub const fn max_frame_bytes(&self) -> usize {
        self.codec.max_frame_bytes()
    }

    pub fn subscribe_datagram(&self, config: &AppConfig) -> Result<NostrJoinFipsPubsubDatagram> {
        let pending = config
            .pending_nostr_join_request
            .as_ref()
            .ok_or_else(|| anyhow!("no pending Nostr join request"))?;
        if pending.request.request_pubkey != self.request_pubkey {
            return Err(anyhow!("pending Nostr join request changed"));
        }
        let requested_at = u64::try_from(pending.request.requested_at)
            .context("pending Nostr join request timestamp is negative")?;
        let request_pubkey = PublicKey::parse(&self.request_pubkey)
            .context("pending Nostr join request pubkey is invalid")?;
        let filter = Filter::new()
            .kind(Kind::Custom(7_368))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::P),
                request_pubkey.to_hex(),
            )
            .since(Timestamp::from(requested_at))
            .limit(NOSTR_JOIN_PUBSUB_INITIAL_EVENT_LIMIT);
        let payload = self.codec.encode_frame(&FipsPubsubWireMessage::req(
            self.subscription_id.clone(),
            vec![filter],
        ))?;
        Ok(NostrJoinFipsPubsubDatagram::new(payload))
    }

    pub fn close_datagram(&self) -> Result<NostrJoinFipsPubsubDatagram> {
        let payload = self
            .codec
            .encode_frame(&FipsPubsubWireMessage::close(self.subscription_id.clone()))?;
        Ok(NostrJoinFipsPubsubDatagram::new(payload))
    }

    pub fn ingest_datagram(
        &mut self,
        config: &mut AppConfig,
        datagram: &NostrJoinFipsPubsubDatagram,
        now: u64,
    ) -> Result<Option<AppliedNostrJoinApproval>> {
        if datagram.source_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
            || datagram.destination_port != NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT
        {
            return Err(anyhow!(
                "Nostr join pubsub datagram used the wrong FSP service port"
            ));
        }
        let message = self.codec.decode_frame(&datagram.payload)?;
        let FipsPubsubWireMessage::Event {
            subscription_id: Some(subscription_id),
            event,
        } = message
        else {
            return Err(anyhow!(
                "Nostr join pubsub endpoint requires a delivered EVENT frame"
            ));
        };
        if subscription_id != self.subscription_id {
            return Err(anyhow!("Nostr join pubsub subscription id mismatch"));
        }
        let event = event.into_event();
        if !is_targeted_approval_event(&event, &self.request_pubkey) {
            return Ok(None);
        }
        if self
            .buffered_events
            .iter()
            .any(|known| known.id == event.id)
        {
            return Ok(None);
        }
        let Some(pending) = config.pending_nostr_join_request.as_ref() else {
            return Ok(None);
        };
        if !is_valid_nostr_join_approval_candidate(pending, &event, now)? {
            return Ok(None);
        }
        if self.buffered_events.len() >= MAX_BUFFERED_APPROVAL_EVENTS {
            let event_type = approval_event_type(&event);
            let replace = self
                .buffered_events
                .iter()
                .position(|buffered| approval_event_type(buffered) == event_type)
                .unwrap_or(0);
            self.buffered_events.remove(replace);
        }
        self.buffered_events.push(event);
        match config.apply_nostr_join_approval_events(&self.buffered_events, now) {
            Ok(Some(applied)) => {
                self.buffered_events.clear();
                Ok(Some(applied))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.buffered_events.clear();
                Err(error)
            }
        }
    }
}

pub fn is_targeted_approval_event(event: &Event, request_pubkey: &str) -> bool {
    event.kind.as_u16() == 7_368
        && event_has_tag(event, "p", request_pubkey)
        && (event_has_tag(event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE)
            || event_has_tag(event, "type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE))
}

fn approval_event_type(event: &Event) -> Option<&'static str> {
    if event_has_tag(event, "type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE) {
        Some(NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE)
    } else if event_has_tag(event, "type", NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE) {
        Some(NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE)
    } else {
        None
    }
}

fn event_has_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().is_some_and(|part| part == name)
            && parts.get(1).is_some_and(|part| part == value)
    })
}
