use anyhow::{Context, Result, anyhow};
use nostr_pubsub::{
    Filter, FipsPubsubWireCodec, FipsPubsubWireMessage, PublicKey, SubscriptionId, VerifiedEvent,
};
use nostr_sdk::prelude::{Alphabet, Event, Kind, SingleLetterTag, Timestamp};

use crate::config::AppConfig;
use crate::identity_bridge::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE, NOSTR_VPN_JOIN_APPROVAL_CONTEXT_TYPE,
};
use crate::join_requests::{AppliedNostrJoinApproval, is_valid_nostr_join_approval_candidate};

/// FSP DataPacket service port reserved for Nostr join approval pubsub.
pub const NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT: u16 = 7_368;
pub const NOSTR_JOIN_PUBSUB_INITIAL_EVENT_LIMIT: usize = 8;
const MAX_BUFFERED_APPROVAL_EVENTS: usize = 4;

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

    pub fn request_event_datagram(
        &self,
        config: &AppConfig,
    ) -> Result<NostrJoinFipsPubsubDatagram> {
        let pending = config
            .pending_nostr_join_request
            .as_ref()
            .ok_or_else(|| anyhow!("no pending Nostr join request"))?;
        if pending.request.request_pubkey != self.request_pubkey {
            return Err(anyhow!("pending Nostr join request changed"));
        }
        let event = VerifiedEvent::try_from(pending.request_event()?)
            .context("pending Nostr join request event failed signature verification")?;
        let payload = self
            .codec
            .encode_frame(&FipsPubsubWireMessage::publish(event))?;
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

fn is_targeted_approval_event(event: &Event, request_pubkey: &str) -> bool {
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
