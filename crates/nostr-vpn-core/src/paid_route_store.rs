use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use cashu_service::{
    CashuSpilmanPayment, CashuSpilmanPaymentReceiver, CashuSpilmanPaymentSigner,
    STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION, StreamingRouteBalanceUpdate,
    StreamingRouteCashuPaymentKind, StreamingRouteCashuPaymentRequest,
    StreamingRouteCashuTokenLease, StreamingRouteCashuTokenLeaseRequest, StreamingRouteChannelOpen,
    StreamingRouteCooperativeClose, StreamingRoutePaymentEnvelope, StreamingRoutePaymentPayload,
    create_streaming_route_cashu_payment, create_streaming_route_cashu_token_lease,
    process_streaming_route_cashu_payment_with_receiver, streaming_route_cashu_balance_for_msat,
    streaming_route_cashu_balance_msat, streaming_route_cashu_capacity_for_sat,
    streaming_route_cashu_capacity_sat, validate_streaming_route_cashu_payment_claim,
    validate_streaming_route_cashu_payment_progress,
};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use serde::{Deserialize, Serialize};

use crate::config::normalize_nostr_pubkey;
use crate::paid_routes::{
    PaidExitConfig, PaidRouteAccessState, PaidRouteLease, PaidRouteMeter, PaidRouteOffer,
    PaidRoutePaymentMode, PaidRoutePaymentState, PaidRouteQualityMetrics, PaidRouteQuote,
    PaidRouteSession, PaidRouteUsage, SignedPaidRouteOffer,
};

const CURRENT_VERSION: u8 = 1;
const SELLER_CONNECTION_MINIMUM_PAYMENT_SKEW_MILLIS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteStore {
    #[serde(default = "default_version")]
    pub version: u8,
    #[serde(default)]
    pub wallet: PaidRouteWalletState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offers: BTreeMap<String, PaidRouteOfferRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotes: BTreeMap<String, PaidRouteQuoteRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub leases: BTreeMap<String, PaidRouteLeaseRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, PaidRouteChannelRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, PaidRouteSessionRecord>,
}

impl Default for PaidRouteStore {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            wallet: PaidRouteWalletState::default(),
            offers: BTreeMap::new(),
            quotes: BTreeMap::new(),
            leases: BTreeMap::new(),
            channels: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteWalletState {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_mint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mints: Vec<PaidRouteWalletMint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteWalletMint {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_msat: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_checked_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteOfferRecord {
    pub signed_offer: SignedPaidRouteOffer,
    pub offer: PaidRouteOffer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relay_urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_score: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rating_updated_at_unix: u64,
    pub first_seen_unix: u64,
    pub last_seen_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteQuoteRecord {
    pub quote: PaidRouteQuote,
    pub created_at_unix: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteLeaseRecord {
    pub lease: PaidRouteLease,
    pub status: PaidRouteLifecycleStatus,
    pub created_at_unix: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteChannelRecord {
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offer_id: String,
    pub role: PaidRouteChannelRole,
    pub status: PaidRouteLifecycleStatus,
    #[serde(default)]
    pub payment: PaidRoutePaymentState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mint_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub counterparty_npub: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated_at_unix: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteChannelRole {
    Buyer,
    Seller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteLifecycleStatus {
    Opening,
    Probing,
    Active,
    Paused,
    Closing,
    Closed,
    Expired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteSessionRecord {
    pub session: PaidRouteSession,
    pub created_at_unix: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePaidRouteSessionProbeRequest {
    pub session_id: String,
    pub realized_exit_ip: Option<String>,
    pub observed_country_code: Option<String>,
    pub observed_asn: Option<u32>,
    pub quality: Option<PaidRouteQualityMetrics>,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePaidRouteSessionProbeResult {
    pub session_id: String,
    pub changed: bool,
    pub realized_exit_ip: Option<String>,
    pub observed_country_code: Option<String>,
    pub observed_asn: Option<u32>,
    pub quality: Option<PaidRouteQualityMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPaidRouteBuyerSessionRequest {
    pub offer_selector: String,
    pub buyer_npub: String,
    pub mint_url: Option<String>,
    pub channel_capacity_sat: Option<u64>,
    pub initial_paid_msat: u64,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPaidRouteBuyerSessionResult {
    pub offer_key: String,
    pub offer_id: String,
    pub seller_npub: String,
    pub mint_url: String,
    pub quote_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub session_id: String,
    pub channel_capacity_sat: u64,
    pub expires_at_unix: u64,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildPaidRouteBuyerPaymentEnvelopeKind {
    ChannelOpen,
    BalanceUpdate,
    CooperativeClose,
}

impl BuildPaidRouteBuyerPaymentEnvelopeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelOpen => "channel_open",
            Self::BalanceUpdate => "balance_update",
            Self::CooperativeClose => "cooperative_close",
        }
    }
}

impl From<BuildPaidRouteBuyerPaymentEnvelopeKind> for StreamingRouteCashuPaymentKind {
    fn from(kind: BuildPaidRouteBuyerPaymentEnvelopeKind) -> Self {
        match kind {
            BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen => Self::ChannelOpen,
            BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate => Self::BalanceUpdate,
            BuildPaidRouteBuyerPaymentEnvelopeKind::CooperativeClose => Self::CooperativeClose,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPaidRouteBuyerPaymentEnvelopeRequest {
    pub session_id: String,
    pub buyer_npub: String,
    pub kind: BuildPaidRouteBuyerPaymentEnvelopeKind,
    pub payment: CashuSpilmanPayment,
    pub delivered_units: Option<u64>,
    pub paid_msat: Option<u64>,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPaidRouteBuyerTokenLeaseEnvelopeRequest {
    pub session_id: String,
    pub buyer_npub: String,
    pub mint_url: String,
    pub cashu_unit: String,
    pub amount: u64,
    pub paid_msat: Option<u64>,
    pub token: String,
    pub expires_at_unix: Option<u64>,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachPaidRouteBuyerSpilmanChannelRequest {
    pub session_id: String,
    pub channel_id: String,
    pub cashu_unit: String,
    pub capacity_sat: u64,
    pub paid_msat: Option<u64>,
    pub payment: CashuSpilmanPayment,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachPaidRouteBuyerSpilmanChannelResult {
    pub previous_channel_id: String,
    pub channel_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub paid_msat: u64,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPaidRouteBuyerSignedPaymentEnvelopeRequest {
    pub session_id: String,
    pub buyer_npub: String,
    pub kind: BuildPaidRouteBuyerPaymentEnvelopeKind,
    pub delivered_units: Option<u64>,
    pub paid_msat: Option<u64>,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaidRouteBuyerPaymentSigningPlan {
    channel_id: String,
    unit: String,
    previous_paid_msat: u64,
    capacity_sat: u64,
    delivered_units: u64,
    paid_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaidRouteBuyerUsageSession {
    seller_pubkey: String,
    seller_npub: String,
    session_id: String,
    lease_id: String,
    channel_id: String,
    config: PaidExitConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildPaidRouteBuyerPaymentEnvelopeResult {
    pub envelope: StreamingRoutePaymentEnvelope,
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub offer_id: String,
    pub buyer_npub: String,
    pub seller_npub: String,
    pub payload_type: String,
    pub paid_msat: u64,
    pub delivered_units: u64,
    pub amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPaidRouteBuyerSignedPaymentEnvelopeForDueResult {
    pub due: PaidRouteBuyerPaymentUpdateDue,
    pub payment: BuildPaidRouteBuyerPaymentEnvelopeResult,
    pub store: PaidRouteStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPaidRouteSellerPaymentRequest {
    pub envelope: StreamingRoutePaymentEnvelope,
    pub seller_npub: String,
    pub config: PaidExitConfig,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPaidRouteSellerPaymentResult {
    pub service_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub session_id: String,
    pub buyer_npub: String,
    pub seller_npub: String,
    pub payload_type: String,
    pub paid_msat: u64,
    pub delivered_units: u64,
    pub amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordPaidRouteSellerUsageRequest {
    pub buyer_pubkey: String,
    pub config: PaidExitConfig,
    pub usage_delta: PaidRouteUsage,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPaidRouteSellerUsageResult {
    pub buyer_pubkey: String,
    pub buyer_npub: String,
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub usage: PaidRouteUsage,
    pub paid_msat: u64,
    pub amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordPaidRouteBuyerUsageRequest {
    pub seller_pubkey: String,
    pub usage_delta: PaidRouteUsage,
    pub now_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPaidRouteBuyerUsageResult {
    pub seller_pubkey: String,
    pub seller_npub: String,
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub usage: PaidRouteUsage,
    pub paid_msat: u64,
    pub amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaidRouteBuyerPaymentUpdatesDueRequest {
    pub now_unix: u64,
    pub min_increment_msat: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteBuyerPaymentUpdateDue {
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub offer_id: String,
    pub seller_npub: String,
    pub delivered_units: u64,
    pub paid_msat: u64,
    pub amount_due_msat: u64,
    pub target_paid_msat: u64,
    pub payment_increment_msat: u64,
    pub unpaid_msat: u64,
    pub remaining_unpaid_msat: u64,
    pub capacity_msat: u64,
    pub capacity_exhausted: bool,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub expires_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteSellerAdmission {
    pub buyer_pubkey: String,
    pub buyer_npub: String,
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub state: PaidRouteAccessState,
    pub allow_routing: bool,
    pub amount_due_msat: u64,
    pub paid_msat: u64,
    pub unpaid_msat: u64,
    pub expires_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteSellerCollectionState {
    pub buyer_npub: String,
    pub session_id: String,
    pub lease_id: String,
    pub channel_id: String,
    pub collectable: bool,
    pub manual_collect: bool,
    pub auto_collect_due: bool,
    pub reason: String,
    pub paid_msat: u64,
    pub expires_at_unix: u64,
    pub due_at_unix: u64,
    pub updated_at_unix: u64,
}

impl PaidRouteStore {
    pub fn upsert_wallet_mint(
        &mut self,
        url: impl AsRef<str>,
        label: impl AsRef<str>,
        balance_msat: Option<u64>,
        checked_at_unix: u64,
    ) -> bool {
        let url = url.as_ref().trim();
        if url.is_empty() {
            return false;
        }
        let label = label.as_ref().trim();
        if self.wallet.default_mint.trim().is_empty() {
            self.wallet.default_mint = url.to_string();
        }

        if let Some(existing) = self.wallet.mints.iter_mut().find(|mint| mint.url == url) {
            let before = existing.clone();
            existing.label = label.to_string();
            existing.balance_msat = balance_msat;
            existing.last_checked_unix = checked_at_unix;
            return *existing != before;
        }

        self.wallet.mints.push(PaidRouteWalletMint {
            url: url.to_string(),
            label: label.to_string(),
            balance_msat,
            last_checked_unix: checked_at_unix,
        });
        self.wallet
            .mints
            .sort_by(|left, right| left.url.cmp(&right.url));
        true
    }

    pub fn set_default_mint(&mut self, url: impl AsRef<str>) -> bool {
        let url = url.as_ref().trim();
        if url.is_empty() {
            return false;
        }
        let mut changed = false;
        if !self.wallet.mints.iter().any(|mint| mint.url == url) {
            self.wallet.mints.push(PaidRouteWalletMint {
                url: url.to_string(),
                label: String::new(),
                balance_msat: None,
                last_checked_unix: 0,
            });
            self.wallet
                .mints
                .sort_by(|left, right| left.url.cmp(&right.url));
            changed = true;
        }
        if self.wallet.default_mint != url {
            self.wallet.default_mint = url.to_string();
            changed = true;
        }
        changed
    }

    pub fn remove_wallet_mint(&mut self, url: impl AsRef<str>) -> bool {
        let url = url.as_ref().trim();
        if url.is_empty() {
            return false;
        }
        let before_len = self.wallet.mints.len();
        self.wallet.mints.retain(|mint| mint.url != url);
        let removed = self.wallet.mints.len() != before_len;
        if self.wallet.default_mint == url {
            self.wallet.default_mint = self
                .wallet
                .mints
                .first()
                .map(|mint| mint.url.clone())
                .unwrap_or_default();
            return true;
        }
        removed
    }

    pub fn upsert_signed_offer(
        &mut self,
        signed_offer: SignedPaidRouteOffer,
        relay_urls: Vec<String>,
        seen_at_unix: u64,
    ) -> Result<bool> {
        signed_offer.verify()?;
        let offer = signed_offer.offer()?;
        let key = paid_route_offer_store_key(&offer.seller_npub, &offer.offer_id);
        let relay_urls = normalize_relay_list(relay_urls);
        let incoming_created_at = signed_offer.event.created_at.as_secs();
        let incoming_event_id = signed_offer.event.id.to_string();

        let replace = match self.offers.get(&key) {
            None => true,
            Some(existing) if existing.signed_offer.verify().is_err() => true,
            Some(existing)
                if existing.signed_offer.event.created_at.as_secs() < incoming_created_at =>
            {
                true
            }
            Some(existing) if existing.signed_offer.event.id.to_string() == incoming_event_id => {
                false
            }
            Some(_) => false,
        };

        if let Some(existing) = self.offers.get_mut(&key)
            && !replace
        {
            let before = existing.clone();
            existing.last_seen_unix = existing.last_seen_unix.max(seen_at_unix);
            existing.relay_urls = merge_sorted_strings(&existing.relay_urls, relay_urls);
            return Ok(*existing != before);
        }

        let first_seen_unix = self
            .offers
            .get(&key)
            .map(|record| record.first_seen_unix)
            .unwrap_or(seen_at_unix);
        let (rating_score, rating_updated_at_unix) = self
            .offers
            .get(&key)
            .map(|record| (record.rating_score, record.rating_updated_at_unix))
            .unwrap_or((None, 0));
        self.offers.insert(
            key,
            PaidRouteOfferRecord {
                signed_offer,
                offer,
                relay_urls,
                rating_score,
                rating_updated_at_unix,
                first_seen_unix,
                last_seen_unix: seen_at_unix,
            },
        );
        Ok(true)
    }

    pub fn upsert_offer_rating_score(
        &mut self,
        seller_npub: &str,
        score: i64,
        updated_at_unix: u64,
    ) -> bool {
        let seller_npub = seller_npub.trim();
        if seller_npub.is_empty() {
            return false;
        }
        let score = score.clamp(-100, 100);
        let mut changed = false;
        for record in self.offers.values_mut() {
            if record.offer.seller_npub != seller_npub
                || record.rating_updated_at_unix > updated_at_unix
            {
                continue;
            }
            let before = (record.rating_score, record.rating_updated_at_unix);
            record.rating_score = Some(score);
            record.rating_updated_at_unix = updated_at_unix;
            changed |= before != (record.rating_score, record.rating_updated_at_unix);
        }
        changed
    }

    pub fn upsert_quote(&mut self, quote: PaidRouteQuote, updated_at_unix: u64) -> bool {
        let key = quote.quote_id.trim().to_string();
        if key.is_empty() {
            return false;
        }
        let record = PaidRouteQuoteRecord {
            quote,
            created_at_unix: updated_at_unix,
            updated_at_unix,
        };
        upsert_record(&mut self.quotes, key, record)
    }

    pub fn upsert_lease(
        &mut self,
        lease: PaidRouteLease,
        status: PaidRouteLifecycleStatus,
        updated_at_unix: u64,
    ) -> bool {
        let key = lease.lease_id.trim().to_string();
        if key.is_empty() {
            return false;
        }
        let record = PaidRouteLeaseRecord {
            lease,
            status,
            created_at_unix: updated_at_unix,
            updated_at_unix,
        };
        upsert_record(&mut self.leases, key, record)
    }

    pub fn upsert_channel(&mut self, channel: PaidRouteChannelRecord) -> bool {
        let key = channel.channel_id.trim();
        if key.is_empty() {
            return false;
        }
        upsert_record(&mut self.channels, key.to_string(), channel)
    }

    pub fn upsert_session(&mut self, session: PaidRouteSession, updated_at_unix: u64) -> bool {
        let key = session.session_id.trim().to_string();
        if key.is_empty() {
            return false;
        }
        let record = PaidRouteSessionRecord {
            session,
            created_at_unix: updated_at_unix,
            updated_at_unix,
        };
        upsert_record(&mut self.sessions, key, record)
    }

    pub fn mark_seller_channel_closed(
        &mut self,
        channel_id: &str,
        paid_msat: u64,
        updated_at_unix: u64,
    ) -> Result<bool> {
        let channel_id = trimmed_required(channel_id, "paid route channel id")?;
        let channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
        if channel.role != PaidRouteChannelRole::Seller {
            return Err(anyhow!(
                "paid route channel {channel_id} is not a seller channel"
            ));
        }

        let mut changed = false;
        if channel.status != PaidRouteLifecycleStatus::Closed {
            channel.status = PaidRouteLifecycleStatus::Closed;
            changed = true;
        }
        if paid_msat > channel.payment.paid_msat {
            channel.payment.paid_msat = paid_msat;
            changed = true;
        }
        if channel.payment.mode != PaidRoutePaymentMode::CashuSpilman {
            channel.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            changed = true;
        }
        if changed {
            channel.payment.updated_at_unix = updated_at_unix;
            channel.updated_at_unix = updated_at_unix;
        }

        let mut lease_ids = Vec::new();
        for record in self.sessions.values_mut() {
            if record.session.payment.channel_id != channel_id {
                continue;
            }
            lease_ids.push(record.session.lease_id.clone());
            let before = record.session.payment.clone();
            if paid_msat > record.session.payment.paid_msat {
                record.session.payment.paid_msat = paid_msat;
            }
            record.session.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            if record.session.payment != before {
                record.session.payment.updated_at_unix = updated_at_unix;
                record.updated_at_unix = updated_at_unix;
                changed = true;
            }
        }
        lease_ids.sort();
        lease_ids.dedup();
        for lease_id in lease_ids {
            let Some(lease) = self.leases.get_mut(&lease_id) else {
                continue;
            };
            if lease.status != PaidRouteLifecycleStatus::Closed {
                lease.status = PaidRouteLifecycleStatus::Closed;
                lease.updated_at_unix = updated_at_unix;
                changed = true;
            }
        }

        Ok(changed)
    }

    pub fn update_session_probe(
        &mut self,
        request: UpdatePaidRouteSessionProbeRequest,
    ) -> Result<UpdatePaidRouteSessionProbeResult> {
        let session_id = trimmed_required(&request.session_id, "paid route session id")?;
        let record = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} does not exist"))?;
        let before = record.session.clone();

        if let Some(realized_exit_ip) = normalize_optional_probe_string(request.realized_exit_ip) {
            record.session.realized_exit_ip = Some(realized_exit_ip);
        }
        if let Some(country) = normalize_optional_country_code(request.observed_country_code) {
            record.session.observed_country_code = Some(country);
        }
        if let Some(asn) = request.observed_asn {
            record.session.observed_asn = Some(asn);
        }
        if let Some(mut quality) = request.quality
            && !quality.is_empty()
        {
            if quality.last_seen_unix.is_none() {
                quality.last_seen_unix = Some(request.now_unix);
            }
            record
                .session
                .quality
                .get_or_insert_with(PaidRouteQualityMetrics::default)
                .merge_patch(quality);
        }

        let changed = record.session != before;
        if changed {
            record.updated_at_unix = request.now_unix;
        }

        Ok(UpdatePaidRouteSessionProbeResult {
            session_id,
            changed,
            realized_exit_ip: record.session.realized_exit_ip.clone(),
            observed_country_code: record.session.observed_country_code.clone(),
            observed_asn: record.session.observed_asn,
            quality: record.session.quality.clone(),
        })
    }

    pub fn open_buyer_session(
        &mut self,
        request: OpenPaidRouteBuyerSessionRequest,
    ) -> Result<OpenPaidRouteBuyerSessionResult> {
        let (offer_key, offer) = self
            .resolve_offer(&request.offer_selector)
            .map(|(key, record)| (key, record.offer.clone()))?;
        let buyer_npub = normalize_paid_route_npub(&request.buyer_npub, "buyer")?;
        let mint_url = select_buyer_mint(&offer, &self.wallet, request.mint_url.as_deref())?;
        let capacity_sat = requested_channel_capacity(&offer, request.channel_capacity_sat)?;
        let now_unix = request.now_unix;
        let expires_at_unix = now_unix.saturating_add(offer.channel.channel_expiry_secs.max(1));
        let seller_pubkey = PublicKey::parse(&offer.seller_npub)
            .map_err(|error| anyhow!("invalid paid route seller npub: {error}"))?;
        let receiver_pubkey_hex = paid_route_offer_receiver_pubkey_hex(&offer, &seller_pubkey)?;
        let id_suffix = paid_route_buyer_session_id_suffix(&offer_key, &offer.offer_id, now_unix);
        let quote_id = format!("quote-{id_suffix}");
        let lease_id = format!("lease-{id_suffix}");
        let channel_id = format!("channel-{id_suffix}");
        let session_id = format!("session-{id_suffix}");
        let status = initial_buyer_session_status(&offer, request.initial_paid_msat);
        let payment = PaidRoutePaymentState {
            mode: PaidRoutePaymentMode::CashuSpilman,
            channel_id: channel_id.clone(),
            cashu_unit: "sat".to_string(),
            capacity_sat,
            paid_msat: request.initial_paid_msat,
            updated_at_unix: now_unix,
            cashu_spilman_payment: None,
            cashu_token_lease: None,
        };

        let mut changed = self.ensure_buyer_mint_present(&mint_url, now_unix);
        changed |= self.upsert_quote(
            PaidRouteQuote {
                quote_id: quote_id.clone(),
                offer_id: offer.offer_id.clone(),
                payment_mode: PaidRoutePaymentMode::CashuSpilman,
                channel_capacity_sat: capacity_sat,
                expires_at_unix,
                receiver_pubkey_hex,
            },
            now_unix,
        );
        changed |= self.upsert_lease(
            PaidRouteLease {
                lease_id: lease_id.clone(),
                offer_id: offer.offer_id.clone(),
                quote_id: quote_id.clone(),
                buyer_npub,
                starts_at_unix: now_unix,
                expires_at_unix,
            },
            status,
            now_unix,
        );
        changed |= self.upsert_channel(PaidRouteChannelRecord {
            channel_id: channel_id.clone(),
            offer_id: offer.offer_id.clone(),
            role: PaidRouteChannelRole::Buyer,
            status,
            payment: payment.clone(),
            mint_url: mint_url.clone(),
            counterparty_npub: offer.seller_npub.clone(),
            created_at_unix: now_unix,
            expires_at_unix,
            updated_at_unix: now_unix,
            error: String::new(),
        });
        changed |= self.upsert_session(
            PaidRouteSession {
                session_id: session_id.clone(),
                lease_id: lease_id.clone(),
                usage: PaidRouteUsage::default(),
                payment,
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: None,
            },
            now_unix,
        );

        Ok(OpenPaidRouteBuyerSessionResult {
            offer_key,
            offer_id: offer.offer_id,
            seller_npub: offer.seller_npub,
            mint_url,
            quote_id,
            lease_id,
            channel_id,
            session_id,
            channel_capacity_sat: capacity_sat,
            expires_at_unix,
            changed,
        })
    }

    pub fn buyer_session_seller_npub(&self, session_id: &str) -> Result<String> {
        let session_id = trimmed_required(session_id, "paid route session id")?;
        let record = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} not found"))?;
        if let Some(channel) = self.channels.get(&record.session.payment.channel_id) {
            if channel.role != PaidRouteChannelRole::Buyer {
                return Err(anyhow!(
                    "paid route session {session_id} is not a buyer session"
                ));
            }
            let seller = channel.counterparty_npub.trim();
            if !seller.is_empty() {
                return normalize_paid_route_npub(seller, "seller");
            }
        }

        let lease = self
            .leases
            .get(&record.session.lease_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} has no lease"))?;
        self.offers
            .values()
            .find(|candidate| candidate.offer.offer_id == lease.lease.offer_id)
            .map(|candidate| candidate.offer.seller_npub.clone())
            .filter(|seller| !seller.trim().is_empty())
            .ok_or_else(|| anyhow!("paid route session {session_id} has no seller offer"))
            .and_then(|seller| normalize_paid_route_npub(&seller, "seller"))
    }

    pub fn buyer_session_allows_routing(&self, session_id: &str, now_unix: u64) -> Result<bool> {
        let session_id = trimmed_required(session_id, "paid route session id")?;
        let record = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease_record = self.leases.get(&record.session.lease_id).ok_or_else(|| {
            anyhow!(
                "paid route lease {} does not exist",
                record.session.lease_id
            )
        })?;
        let channel = self
            .channels
            .get(&record.session.payment.channel_id)
            .ok_or_else(|| {
                anyhow!(
                    "paid route channel {} does not exist",
                    record.session.payment.channel_id
                )
            })?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!(
                "paid route channel {} is not a buyer channel",
                channel.channel_id
            ));
        }
        let expires_at_unix = lease_record
            .lease
            .expires_at_unix
            .min(channel.expires_at_unix);
        if expires_at_unix <= now_unix {
            return Ok(false);
        }
        let offer = self.buyer_offer_for_session(lease_record, channel)?;
        let config = PaidExitConfig::from_paid_route_offer(&offer);
        let decision = record.session.routing_decision(&config);
        if !paid_route_lifecycle_allows_routing(lease_record.status)
            || !paid_route_lifecycle_allows_routing(channel.status)
            || !decision.allow_routing
        {
            return Ok(false);
        }
        if paid_route_offer_requires_payment_before_routing(&offer)
            && !paid_route_session_has_payment_material(&record.session, channel)
        {
            return Ok(false);
        }
        Ok(true)
    }

    pub fn build_buyer_payment_envelope(
        &mut self,
        request: BuildPaidRouteBuyerPaymentEnvelopeRequest,
    ) -> Result<BuildPaidRouteBuyerPaymentEnvelopeResult> {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.build_buyer_payment_envelope_inner(request)?;
        result.changed = next != before;
        *self = next;
        Ok(result)
    }

    pub fn best_rated_offer_key(&self) -> Result<String> {
        self.offers
            .iter()
            .max_by(|(left_key, left), (right_key, right)| {
                paid_route_offer_autoselect_score(left)
                    .cmp(&paid_route_offer_autoselect_score(right))
                    .then_with(|| left.last_seen_unix.cmp(&right.last_seen_unix))
                    .then_with(|| right_key.cmp(left_key))
            })
            .map(|(key, _)| key.clone())
            .ok_or_else(|| {
                anyhow!("no paid route offers are stored; discover offers before buying")
            })
    }

    pub fn attach_buyer_spilman_channel(
        &mut self,
        request: AttachPaidRouteBuyerSpilmanChannelRequest,
    ) -> Result<AttachPaidRouteBuyerSpilmanChannelResult> {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.attach_buyer_spilman_channel_inner(request)?;
        result.changed = next != before;
        *self = next;
        Ok(result)
    }

    pub fn build_buyer_signed_payment_envelope<S: CashuSpilmanPaymentSigner>(
        &mut self,
        signer: &S,
        request: BuildPaidRouteBuyerSignedPaymentEnvelopeRequest,
    ) -> Result<BuildPaidRouteBuyerPaymentEnvelopeResult> {
        let plan = self.buyer_payment_signing_plan(&request)?;
        let signed = create_streaming_route_cashu_payment(
            signer,
            StreamingRouteCashuPaymentRequest {
                kind: request.kind.into(),
                channel_id: plan.channel_id.clone(),
                unit: plan.unit,
                paid_msat: plan.paid_msat,
                previous_paid_msat: plan.previous_paid_msat,
                capacity_sat: plan.capacity_sat,
            },
        )
        .map_err(|error| anyhow!("{error}"))?;

        self.build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
            session_id: request.session_id,
            buyer_npub: request.buyer_npub,
            kind: request.kind,
            payment: signed.payment,
            delivered_units: Some(plan.delivered_units),
            paid_msat: Some(signed.paid_msat),
            now_unix: request.now_unix,
        })
    }

    pub fn build_buyer_signed_payment_envelope_for_due<S: CashuSpilmanPaymentSigner>(
        &self,
        signer: &S,
        buyer_npub: &str,
        update_due: &PaidRouteBuyerPaymentUpdateDue,
        now_unix: u64,
    ) -> Result<BuildPaidRouteBuyerSignedPaymentEnvelopeForDueResult> {
        let mut store = self.clone();
        let payment = store.build_buyer_signed_payment_envelope(
            signer,
            BuildPaidRouteBuyerSignedPaymentEnvelopeRequest {
                session_id: update_due.session_id.clone(),
                buyer_npub: buyer_npub.to_string(),
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                delivered_units: Some(update_due.delivered_units),
                paid_msat: Some(update_due.target_paid_msat),
                now_unix,
            },
        )?;
        Ok(BuildPaidRouteBuyerSignedPaymentEnvelopeForDueResult {
            due: update_due.clone(),
            payment,
            store,
        })
    }

    pub fn build_buyer_token_lease_envelope(
        &mut self,
        request: BuildPaidRouteBuyerTokenLeaseEnvelopeRequest,
    ) -> Result<BuildPaidRouteBuyerPaymentEnvelopeResult> {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.build_buyer_token_lease_envelope_inner(request)?;
        result.changed = next != before;
        *self = next;
        Ok(result)
    }

    pub fn record_buyer_usage(
        &mut self,
        request: RecordPaidRouteBuyerUsageRequest,
    ) -> Result<Option<RecordPaidRouteBuyerUsageResult>> {
        if request.usage_delta.is_empty() {
            return Ok(None);
        }
        let seller_pubkey = normalize_nostr_pubkey(&request.seller_pubkey)
            .unwrap_or_else(|_| request.seller_pubkey.trim().to_string());
        if seller_pubkey.is_empty() {
            return Err(anyhow!("paid route seller pubkey is empty"));
        }
        let seller_npub = normalize_paid_route_npub(&seller_pubkey, "seller")?;
        let Some(target) = self.buyer_usage_session_for_seller(&seller_npub, request.now_unix)
        else {
            return Ok(None);
        };

        let Some(record) = self.sessions.get_mut(&target.session_id) else {
            return Ok(None);
        };
        let before = record.session.usage.clone();
        apply_usage_delta(&mut record.session.usage, &request.usage_delta);
        let changed = record.session.usage != before;
        if changed {
            record.updated_at_unix = request.now_unix;
        }

        let decision = record.session.routing_decision(&target.config);
        Ok(Some(RecordPaidRouteBuyerUsageResult {
            seller_pubkey: target.seller_pubkey,
            seller_npub: target.seller_npub,
            session_id: target.session_id,
            lease_id: target.lease_id,
            channel_id: target.channel_id,
            usage: record.session.usage.clone(),
            paid_msat: decision.paid_msat,
            amount_due_msat: decision.amount_due_msat,
            unpaid_msat: decision.unpaid_msat,
            allow_routing: decision.allow_routing,
            state: decision.state,
            changed,
        }))
    }

    pub fn buyer_payment_updates_due(
        &self,
        request: PaidRouteBuyerPaymentUpdatesDueRequest,
    ) -> Vec<PaidRouteBuyerPaymentUpdateDue> {
        let mut due = Vec::new();
        for record in self.sessions.values() {
            let Some(lease_record) = self.leases.get(&record.session.lease_id) else {
                continue;
            };
            let Some(channel) = self.channels.get(&record.session.payment.channel_id) else {
                continue;
            };
            if channel.role != PaidRouteChannelRole::Buyer
                || record.session.payment.mode != PaidRoutePaymentMode::CashuSpilman
            {
                continue;
            }
            if !paid_route_lifecycle_allows_routing(lease_record.status)
                || !paid_route_lifecycle_allows_routing(channel.status)
            {
                continue;
            }
            let expires_at_unix = lease_record
                .lease
                .expires_at_unix
                .min(channel.expires_at_unix);
            if expires_at_unix <= request.now_unix {
                continue;
            }
            let Ok(offer) = self.buyer_offer_for_session(lease_record, channel) else {
                continue;
            };
            let config = PaidExitConfig::from_paid_route_offer(&offer);
            let decision = record.session.routing_decision(&config);
            let capacity_msat = record.session.payment.capacity_sat.saturating_mul(1_000);
            let raw_target_paid_msat = if capacity_msat == 0 {
                decision.amount_due_msat
            } else {
                decision.amount_due_msat.min(capacity_msat)
            };
            let unit = paid_route_payment_cashu_unit(&record.session.payment);
            let Ok(target_paid_msat) = cashu_payment_target_msat(&unit, raw_target_paid_msat)
            else {
                continue;
            };
            if target_paid_msat <= record.session.payment.paid_msat {
                continue;
            }
            let payment_increment_msat =
                target_paid_msat.saturating_sub(record.session.payment.paid_msat);
            if payment_increment_msat < request.min_increment_msat {
                continue;
            }
            due.push(PaidRouteBuyerPaymentUpdateDue {
                session_id: record.session.session_id.clone(),
                lease_id: lease_record.lease.lease_id.clone(),
                channel_id: channel.channel_id.clone(),
                offer_id: offer.offer_id,
                seller_npub: offer.seller_npub,
                delivered_units: decision.delivered_units,
                paid_msat: record.session.payment.paid_msat,
                amount_due_msat: decision.amount_due_msat,
                target_paid_msat,
                payment_increment_msat,
                unpaid_msat: decision.unpaid_msat,
                remaining_unpaid_msat: decision.amount_due_msat.saturating_sub(target_paid_msat),
                capacity_msat,
                capacity_exhausted: capacity_msat > 0 && decision.amount_due_msat > capacity_msat,
                allow_routing: decision.allow_routing,
                state: decision.state,
                expires_at_unix,
                updated_at_unix: record.updated_at_unix.max(channel.updated_at_unix),
            });
        }
        due
    }

    fn attach_buyer_spilman_channel_inner(
        &mut self,
        request: AttachPaidRouteBuyerSpilmanChannelRequest,
    ) -> Result<AttachPaidRouteBuyerSpilmanChannelResult> {
        let session_id = trimmed_required(&request.session_id, "paid route session id")?;
        let channel_id = trimmed_required(&request.channel_id, "Cashu Spilman channel id")?;
        let unit = request.cashu_unit.trim();
        if unit.is_empty() {
            return Err(anyhow!("missing Cashu Spilman channel unit"));
        }
        if request.payment.channel_id.trim() != channel_id {
            return Err(anyhow!(
                "Cashu Spilman payment channel {} does not match attached channel {}",
                request.payment.channel_id,
                channel_id
            ));
        }
        let inferred_paid_msat = cashu_payment_balance_msat(unit, request.payment.balance)?;
        let paid_msat = request.paid_msat.unwrap_or(inferred_paid_msat);
        validate_cashu_spilman_payment_claim(
            &request.payment,
            &channel_id,
            unit,
            paid_msat,
            request.capacity_sat,
            true,
        )?;
        let session_record = self
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease_record = self
            .leases
            .get(&session_record.session.lease_id)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "paid route lease {} does not exist",
                    session_record.session.lease_id
                )
            })?;
        let previous_channel_id = session_record.session.payment.channel_id.clone();
        let mut channel = self
            .channels
            .remove(&previous_channel_id)
            .or_else(|| self.channels.remove(&channel_id))
            .ok_or_else(|| anyhow!("paid route channel {previous_channel_id} does not exist"))?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!(
                "paid route channel {} is not a buyer channel",
                channel.channel_id
            ));
        }
        let offer = self.buyer_offer_for_session(&lease_record, &channel)?;
        validate_paid_route_payment_progress(
            "paid route payment",
            paid_msat,
            session_record.session.payment.paid_msat,
            request.capacity_sat,
        )?;
        let status = preserve_terminal_status(
            channel.status,
            initial_buyer_session_status(&offer, paid_msat),
        );
        let payment = PaidRoutePaymentState {
            mode: PaidRoutePaymentMode::CashuSpilman,
            channel_id: channel_id.to_string(),
            cashu_unit: unit.to_string(),
            capacity_sat: request.capacity_sat,
            paid_msat,
            updated_at_unix: request.now_unix,
            cashu_spilman_payment: Some(request.payment),
            cashu_token_lease: None,
        };

        channel.channel_id = channel_id.to_string();
        channel.status = status;
        channel.payment = payment.clone();
        channel.updated_at_unix = request.now_unix;
        self.channels.insert(channel_id.to_string(), channel);

        let mut session = session_record;
        session.session.payment = payment;
        session.updated_at_unix = request.now_unix;
        self.sessions.insert(session_id.clone(), session);

        if let Some(lease) = self.leases.get_mut(&lease_record.lease.lease_id) {
            lease.status = preserve_terminal_status(lease.status, status);
            lease.updated_at_unix = request.now_unix;
        }

        Ok(AttachPaidRouteBuyerSpilmanChannelResult {
            previous_channel_id,
            channel_id: channel_id.to_string(),
            session_id,
            lease_id: lease_record.lease.lease_id,
            paid_msat,
            changed: false,
        })
    }

    fn buyer_payment_signing_plan(
        &self,
        request: &BuildPaidRouteBuyerSignedPaymentEnvelopeRequest,
    ) -> Result<PaidRouteBuyerPaymentSigningPlan> {
        let session_id = trimmed_required(&request.session_id, "paid route session id")?;
        let buyer_npub = normalize_paid_route_npub(&request.buyer_npub, "buyer")?;
        let session_record = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease_record = self
            .leases
            .get(&session_record.session.lease_id)
            .ok_or_else(|| {
                anyhow!(
                    "paid route lease {} does not exist",
                    session_record.session.lease_id
                )
            })?;
        let channel_id = session_record.session.payment.channel_id.clone();
        let channel = self
            .channels
            .get(&channel_id)
            .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!(
                "paid route channel {channel_id} is not a buyer channel"
            ));
        }
        if normalize_paid_route_npub(&lease_record.lease.buyer_npub, "buyer")? != buyer_npub {
            return Err(anyhow!(
                "paid route payment buyer does not match lease buyer"
            ));
        }
        ensure_open_buyer_channel(channel, lease_record)?;

        let offer = self.buyer_offer_for_session(lease_record, channel)?;
        let config = PaidExitConfig::from_paid_route_offer(&offer);
        let current_units = session_record
            .session
            .usage
            .billable_units_for_meter(config.pricing.meter);
        let delivered_units = request.delivered_units.unwrap_or(current_units);
        if delivered_units < current_units {
            return Err(anyhow!(
                "paid route buyer payment delivered units regressed: {} < {}",
                delivered_units,
                current_units
            ));
        }

        let amount_due_msat = paid_route_amount_due_for_delivered_units(
            &config,
            &session_record.session.usage,
            delivered_units,
        );
        let previous_paid_msat = session_record.session.payment.paid_msat;
        let paid_msat = request
            .paid_msat
            .unwrap_or_else(|| previous_paid_msat.max(amount_due_msat));
        validate_paid_route_payment_progress(
            "paid route buyer payment",
            paid_msat,
            previous_paid_msat,
            session_record.session.payment.capacity_sat,
        )?;

        Ok(PaidRouteBuyerPaymentSigningPlan {
            channel_id,
            unit: paid_route_payment_cashu_unit(&session_record.session.payment),
            previous_paid_msat,
            capacity_sat: session_record.session.payment.capacity_sat,
            delivered_units,
            paid_msat,
        })
    }

    fn build_buyer_payment_envelope_inner(
        &mut self,
        request: BuildPaidRouteBuyerPaymentEnvelopeRequest,
    ) -> Result<BuildPaidRouteBuyerPaymentEnvelopeResult> {
        let session_id = trimmed_required(&request.session_id, "paid route session id")?;
        let buyer_npub = normalize_paid_route_npub(&request.buyer_npub, "buyer")?;
        let session_record = self
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease_record = self
            .leases
            .get(&session_record.session.lease_id)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "paid route lease {} does not exist",
                    session_record.session.lease_id
                )
            })?;
        let channel_id = session_record.session.payment.channel_id.clone();
        let channel = self
            .channels
            .get(&channel_id)
            .cloned()
            .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!(
                "paid route channel {channel_id} is not a buyer channel"
            ));
        }
        if normalize_paid_route_npub(&lease_record.lease.buyer_npub, "buyer")? != buyer_npub {
            return Err(anyhow!(
                "paid route payment buyer does not match lease buyer"
            ));
        }
        if request.payment.channel_id.trim() != channel_id {
            return Err(anyhow!(
                "paid route payment channel {} does not match session channel {}",
                request.payment.channel_id,
                channel_id
            ));
        }
        ensure_open_buyer_channel(&channel, &lease_record)?;

        let offer = self.buyer_offer_for_session(&lease_record, &channel)?;
        let config = PaidExitConfig::from_paid_route_offer(&offer);
        let unit = paid_route_payment_cashu_unit(&session_record.session.payment);
        let current_units = session_record
            .session
            .usage
            .billable_units_for_meter(config.pricing.meter);
        let delivered_units = request.delivered_units.unwrap_or(current_units);
        if delivered_units < current_units {
            return Err(anyhow!(
                "paid route buyer payment delivered units regressed: {} < {}",
                delivered_units,
                current_units
            ));
        }
        let inferred_paid_msat = cashu_payment_balance_msat(&unit, request.payment.balance)?;
        let paid_msat = request.paid_msat.unwrap_or(inferred_paid_msat);
        validate_paid_route_payment_progress(
            "paid route buyer payment",
            paid_msat,
            session_record.session.payment.paid_msat,
            session_record.session.payment.capacity_sat,
        )?;
        validate_cashu_spilman_payment_claim(
            &request.payment,
            &channel_id,
            &unit,
            paid_msat,
            session_record.session.payment.capacity_sat,
            request.kind == BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen,
        )?;

        let amount_due_msat = paid_route_amount_due_for_delivered_units(
            &config,
            &session_record.session.usage,
            delivered_units,
        );
        let expires_at_unix = channel
            .expires_at_unix
            .min(lease_record.lease.expires_at_unix);
        let payload = match request.kind {
            BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen => {
                StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                    mint_url: channel.mint_url.clone(),
                    unit: unit.clone(),
                    capacity: cashu_channel_capacity_for_unit(
                        session_record.session.payment.capacity_sat,
                        &unit,
                    )?,
                    expires_unix: expires_at_unix,
                    receiver_pubkey_hex: self
                        .quotes
                        .get(&lease_record.lease.quote_id)
                        .map(|record| record.quote.receiver_pubkey_hex.clone())
                        .unwrap_or_else(|| {
                            normalize_nostr_pubkey(&offer.seller_npub).unwrap_or_default()
                        }),
                    paid_msat,
                    payment: request.payment.clone(),
                })
            }
            BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate => {
                StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                    delivered_units,
                    amount_due_msat,
                    paid_msat,
                    payment: request.payment.clone(),
                })
            }
            BuildPaidRouteBuyerPaymentEnvelopeKind::CooperativeClose => {
                StreamingRoutePaymentPayload::CooperativeClose(StreamingRouteCooperativeClose {
                    final_paid_msat: paid_msat,
                    payment: request.payment.clone(),
                })
            }
        };
        let payload_type = request.kind.as_str().to_string();

        self.apply_buyer_payment_state(
            &session_id,
            &channel_id,
            &lease_record.lease.lease_id,
            config.pricing.meter,
            request.kind,
            delivered_units,
            paid_msat,
            &unit,
            request.payment.clone(),
            request.now_unix,
        )?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} was not updated"))?;
        let decision = session.session.routing_decision(&config);
        let envelope = StreamingRoutePaymentEnvelope::new(
            offer.offer_id.clone(),
            lease_record.lease.lease_id.clone(),
            buyer_npub.clone(),
            offer.seller_npub.clone(),
            request.now_unix,
            payload,
        );

        Ok(BuildPaidRouteBuyerPaymentEnvelopeResult {
            envelope,
            session_id,
            lease_id: lease_record.lease.lease_id,
            channel_id,
            offer_id: offer.offer_id,
            buyer_npub,
            seller_npub: offer.seller_npub,
            payload_type,
            paid_msat,
            delivered_units: decision.delivered_units,
            amount_due_msat: decision.amount_due_msat,
            unpaid_msat: decision.unpaid_msat,
            allow_routing: decision.allow_routing,
            state: decision.state,
            changed: false,
        })
    }

    fn build_buyer_token_lease_envelope_inner(
        &mut self,
        request: BuildPaidRouteBuyerTokenLeaseEnvelopeRequest,
    ) -> Result<BuildPaidRouteBuyerPaymentEnvelopeResult> {
        let session_id = trimmed_required(&request.session_id, "paid route session id")?;
        let buyer_npub = normalize_paid_route_npub(&request.buyer_npub, "buyer")?;
        let session_record = self
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease_record = self
            .leases
            .get(&session_record.session.lease_id)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "paid route lease {} does not exist",
                    session_record.session.lease_id
                )
            })?;
        let channel_id = session_record.session.payment.channel_id.clone();
        let channel = self
            .channels
            .get(&channel_id)
            .cloned()
            .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!(
                "paid route channel {channel_id} is not a buyer channel"
            ));
        }
        if normalize_paid_route_npub(&lease_record.lease.buyer_npub, "buyer")? != buyer_npub {
            return Err(anyhow!(
                "paid route payment buyer does not match lease buyer"
            ));
        }
        ensure_open_buyer_channel(&channel, &lease_record)?;

        let offer = self.buyer_offer_for_session(&lease_record, &channel)?;
        let config = PaidExitConfig::from_paid_route_offer(&offer);
        let mint_url = if request.mint_url.trim().is_empty() {
            channel.mint_url.clone()
        } else {
            request.mint_url.trim().to_string()
        };
        let cashu_unit = if request.cashu_unit.trim().is_empty() {
            "sat".to_string()
        } else {
            request.cashu_unit.trim().to_string()
        };
        let expires_at_unix = request.expires_at_unix.unwrap_or_else(|| {
            channel
                .expires_at_unix
                .min(lease_record.lease.expires_at_unix)
        });
        if expires_at_unix <= request.now_unix {
            return Err(anyhow!("paid route token lease is already expired"));
        }
        let token_lease =
            create_streaming_route_cashu_token_lease(StreamingRouteCashuTokenLeaseRequest {
                channel_id: channel_id.clone(),
                mint_url,
                unit: cashu_unit,
                amount: request.amount,
                paid_msat: request.paid_msat,
                expires_unix: expires_at_unix,
                token: request.token.clone(),
            })
            .map_err(|error| anyhow!("{error}"))?;
        validate_paid_route_payment_progress(
            "paid route token lease",
            token_lease.paid_msat,
            session_record.session.payment.paid_msat,
            session_record.session.payment.capacity_sat,
        )?;
        let capacity_sat = paid_route_channel_capacity_sat(&token_lease.unit, token_lease.amount)?;
        let status = preserve_terminal_status(
            channel.status,
            initial_buyer_session_status(&offer, token_lease.paid_msat),
        );
        let payment = PaidRoutePaymentState {
            mode: PaidRoutePaymentMode::CashuTokenLease,
            channel_id: channel_id.clone(),
            cashu_unit: token_lease.unit.clone(),
            capacity_sat,
            paid_msat: token_lease.paid_msat,
            updated_at_unix: request.now_unix,
            cashu_spilman_payment: None,
            cashu_token_lease: Some(token_lease.clone()),
        };

        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.status = status;
            channel.payment = payment.clone();
            channel.mint_url = token_lease.mint_url.clone();
            channel.updated_at_unix = request.now_unix;
        }
        if let Some(lease) = self.leases.get_mut(&lease_record.lease.lease_id) {
            lease.status = preserve_terminal_status(lease.status, status);
            lease.updated_at_unix = request.now_unix;
        }
        {
            let record = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| anyhow!("paid route session {session_id} does not exist"))?;
            record.session.payment = payment;
            record.updated_at_unix = request.now_unix;
        }

        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} was not updated"))?;
        let decision = session.session.routing_decision(&config);
        let envelope = StreamingRoutePaymentEnvelope::new(
            offer.offer_id.clone(),
            lease_record.lease.lease_id.clone(),
            buyer_npub.clone(),
            offer.seller_npub.clone(),
            request.now_unix,
            StreamingRoutePaymentPayload::CashuTokenLease(token_lease),
        );

        Ok(BuildPaidRouteBuyerPaymentEnvelopeResult {
            envelope,
            session_id,
            lease_id: lease_record.lease.lease_id,
            channel_id,
            offer_id: offer.offer_id,
            buyer_npub,
            seller_npub: offer.seller_npub,
            payload_type: "cashu_token_lease".to_string(),
            paid_msat: decision.paid_msat,
            delivered_units: decision.delivered_units,
            amount_due_msat: decision.amount_due_msat,
            unpaid_msat: decision.unpaid_msat,
            allow_routing: decision.allow_routing,
            state: decision.state,
            changed: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_buyer_payment_state(
        &mut self,
        session_id: &str,
        channel_id: &str,
        lease_id: &str,
        meter: PaidRouteMeter,
        kind: BuildPaidRouteBuyerPaymentEnvelopeKind,
        delivered_units: u64,
        paid_msat: u64,
        unit: &str,
        payment: CashuSpilmanPayment,
        now_unix: u64,
    ) -> Result<()> {
        if let Some(channel) = self.channels.get_mut(channel_id) {
            channel.payment.cashu_unit = unit.to_string();
            channel.payment.paid_msat = paid_msat;
            channel.payment.updated_at_unix = now_unix;
            channel.payment.cashu_spilman_payment = Some(payment.clone());
            channel.payment.cashu_token_lease = None;
            channel.updated_at_unix = now_unix;
            if kind == BuildPaidRouteBuyerPaymentEnvelopeKind::CooperativeClose {
                channel.status = PaidRouteLifecycleStatus::Closed;
            }
        }
        if kind == BuildPaidRouteBuyerPaymentEnvelopeKind::CooperativeClose
            && let Some(lease) = self.leases.get_mut(lease_id)
        {
            lease.status = PaidRouteLifecycleStatus::Closed;
            lease.updated_at_unix = now_unix;
        }
        let record = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} does not exist"))?;
        apply_delivered_units_for_meter(&mut record.session.usage, meter, delivered_units);
        record.session.payment.cashu_unit = unit.to_string();
        record.session.payment.paid_msat = paid_msat;
        record.session.payment.updated_at_unix = now_unix;
        record.session.payment.cashu_spilman_payment = Some(payment);
        record.session.payment.cashu_token_lease = None;
        record.updated_at_unix = now_unix;
        Ok(())
    }

    fn buyer_offer_for_session(
        &self,
        lease_record: &PaidRouteLeaseRecord,
        channel: &PaidRouteChannelRecord,
    ) -> Result<PaidRouteOffer> {
        self.offers
            .values()
            .find(|record| {
                record.offer.offer_id == lease_record.lease.offer_id
                    && record.offer.seller_npub == channel.counterparty_npub
            })
            .or_else(|| {
                self.offers
                    .values()
                    .find(|record| record.offer.offer_id == lease_record.lease.offer_id)
            })
            .map(|record| record.offer.clone())
            .ok_or_else(|| {
                anyhow!(
                    "paid route offer {} for buyer session was not found",
                    lease_record.lease.offer_id
                )
            })
    }

    fn buyer_usage_session_for_seller(
        &self,
        seller_npub: &str,
        now_unix: u64,
    ) -> Option<PaidRouteBuyerUsageSession> {
        let mut best = None::<(u64, PaidRouteBuyerUsageSession)>;
        for record in self.sessions.values() {
            let Some(lease_record) = self.leases.get(&record.session.lease_id) else {
                continue;
            };
            let Some(channel) = self.channels.get(&record.session.payment.channel_id) else {
                continue;
            };
            if channel.role != PaidRouteChannelRole::Buyer {
                continue;
            }
            if !paid_route_lifecycle_allows_routing(lease_record.status)
                || !paid_route_lifecycle_allows_routing(channel.status)
            {
                continue;
            }
            if lease_record
                .lease
                .expires_at_unix
                .min(channel.expires_at_unix)
                <= now_unix
            {
                continue;
            }
            let Some(channel_seller_npub) =
                normalize_paid_route_npub(&channel.counterparty_npub, "seller").ok()
            else {
                continue;
            };
            if channel_seller_npub != seller_npub {
                continue;
            }
            let Some(offer) = self.buyer_offer_for_session(lease_record, channel).ok() else {
                continue;
            };
            let Some(offer_seller_npub) =
                normalize_paid_route_npub(&offer.seller_npub, "seller").ok()
            else {
                continue;
            };
            if offer_seller_npub != seller_npub {
                continue;
            }
            let Some(seller_pubkey) = normalize_nostr_pubkey(&offer_seller_npub).ok() else {
                continue;
            };
            let updated_at = record.updated_at_unix.max(channel.updated_at_unix);
            let candidate = PaidRouteBuyerUsageSession {
                seller_pubkey,
                seller_npub: offer_seller_npub,
                session_id: record.session.session_id.clone(),
                lease_id: lease_record.lease.lease_id.clone(),
                channel_id: channel.channel_id.clone(),
                config: PaidExitConfig::from_paid_route_offer(&offer),
            };
            if best
                .as_ref()
                .is_none_or(|(best_updated_at, _)| updated_at > *best_updated_at)
            {
                best = Some((updated_at, candidate));
            }
        }
        best.map(|(_, candidate)| candidate)
    }

    pub fn apply_seller_payment(
        &mut self,
        request: ApplyPaidRouteSellerPaymentRequest,
    ) -> Result<ApplyPaidRouteSellerPaymentResult> {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.apply_seller_payment_inner(request)?;
        result.changed = next != before;
        *self = next;
        Ok(result)
    }

    pub fn apply_seller_payment_with_spilman_receiver<R, C>(
        &mut self,
        request: ApplyPaidRouteSellerPaymentRequest,
        receiver: &R,
        context: &C,
    ) -> Result<ApplyPaidRouteSellerPaymentResult>
    where
        R: CashuSpilmanPaymentReceiver<C>,
    {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.apply_seller_payment_inner(request.clone())?;
        result.changed = next != before;
        if !result.changed {
            return Ok(result);
        }
        next.process_seller_spilman_receiver_payment(&request, receiver, context)?;
        *self = next;
        Ok(result)
    }

    fn process_seller_spilman_receiver_payment<R, C>(
        &self,
        request: &ApplyPaidRouteSellerPaymentRequest,
        receiver: &R,
        context: &C,
    ) -> Result<()>
    where
        R: CashuSpilmanPaymentReceiver<C>,
    {
        let mut config = request.config.clone();
        config.normalize();
        let envelope = &request.envelope;
        if envelope.version != STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION {
            return Err(anyhow!(
                "unsupported paid route payment protocol version {}",
                envelope.version
            ));
        }

        let seller_npub = normalize_paid_route_npub(&request.seller_npub, "seller")?;
        let envelope_seller = normalize_paid_route_npub(&envelope.seller, "seller")?;
        if envelope_seller != seller_npub {
            return Err(anyhow!(
                "paid route payment seller does not match local seller"
            ));
        }
        let seller_pubkey_hex = normalize_nostr_pubkey(&seller_npub)?;
        let buyer_npub = normalize_paid_route_npub(&envelope.buyer, "buyer")?;
        let service_id = trimmed_required(&envelope.service_id, "paid route service id")?;
        let lease_id = trimmed_required(&envelope.lease_id, "paid route lease id")?;
        let channel_id = trimmed_required(envelope.channel_id(), "paid route channel id")?;

        match &envelope.payload {
            StreamingRoutePaymentPayload::ChannelOpen(open) => {
                validate_seller_open_payment(&config, &seller_pubkey_hex, &channel_id, open)?;
                let capacity_sat = paid_route_channel_capacity_sat(&open.unit, open.capacity)?;
                process_streaming_route_cashu_payment_with_receiver(
                    receiver,
                    &open.payment,
                    &channel_id,
                    &open.unit,
                    open.paid_msat,
                    capacity_sat,
                    true,
                    context,
                )
                .map_err(|error| anyhow!("{error}"))?;
            }
            StreamingRoutePaymentPayload::BalanceUpdate(update) => {
                self.ensure_existing_seller_session(
                    &service_id,
                    &lease_id,
                    &channel_id,
                    &buyer_npub,
                )?;
                let channel = self.channels.get(&channel_id).expect("validated channel");
                let cashu_unit = paid_route_payment_cashu_unit(&channel.payment);
                process_streaming_route_cashu_payment_with_receiver(
                    receiver,
                    &update.payment,
                    &channel_id,
                    &cashu_unit,
                    update.paid_msat,
                    channel.payment.capacity_sat,
                    false,
                    context,
                )
                .map_err(|error| anyhow!("{error}"))?;
            }
            StreamingRoutePaymentPayload::CooperativeClose(close) => {
                let lease = self
                    .leases
                    .get(&lease_id)
                    .ok_or_else(|| anyhow!("paid route lease {lease_id} does not exist"))?;
                if lease.lease.offer_id != service_id
                    || normalize_paid_route_npub(&lease.lease.buyer_npub, "buyer")? != buyer_npub
                {
                    return Err(anyhow!(
                        "paid route close does not match existing seller lease"
                    ));
                }
                let channel = self
                    .channels
                    .get(&channel_id)
                    .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
                ensure_seller_channel_matches(channel, &service_id, &buyer_npub)?;
                let cashu_unit = paid_route_payment_cashu_unit(&channel.payment);
                process_streaming_route_cashu_payment_with_receiver(
                    receiver,
                    &close.payment,
                    &channel_id,
                    &cashu_unit,
                    close.final_paid_msat,
                    channel.payment.capacity_sat,
                    false,
                    context,
                )
                .map_err(|error| anyhow!("{error}"))?;
            }
            StreamingRoutePaymentPayload::CashuTokenLease(_)
            | StreamingRoutePaymentPayload::CooperativeCloseAck(_) => {}
        }

        Ok(())
    }

    fn apply_seller_payment_inner(
        &mut self,
        request: ApplyPaidRouteSellerPaymentRequest,
    ) -> Result<ApplyPaidRouteSellerPaymentResult> {
        let mut config = request.config;
        config.normalize();
        let envelope = request.envelope;
        if envelope.version != STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION {
            return Err(anyhow!(
                "unsupported paid route payment protocol version {}",
                envelope.version
            ));
        }

        let seller_npub = normalize_paid_route_npub(&request.seller_npub, "seller")?;
        let envelope_seller = normalize_paid_route_npub(&envelope.seller, "seller")?;
        if envelope_seller != seller_npub {
            return Err(anyhow!(
                "paid route payment seller does not match local seller"
            ));
        }
        let seller_pubkey_hex = normalize_nostr_pubkey(&seller_npub)?;
        let buyer_npub = normalize_paid_route_npub(&envelope.buyer, "buyer")?;
        let service_id = trimmed_required(&envelope.service_id, "paid route service id")?;
        let lease_id = trimmed_required(&envelope.lease_id, "paid route lease id")?;
        let channel_id = trimmed_required(envelope.channel_id(), "paid route channel id")?;
        let payload_type = paid_route_payment_payload_type(&envelope.payload).to_string();

        match &envelope.payload {
            StreamingRoutePaymentPayload::ChannelOpen(open) => {
                validate_seller_open_payment(&config, &seller_pubkey_hex, &channel_id, open)?;
                let capacity_sat = paid_route_channel_capacity_sat(&open.unit, open.capacity)?;
                self.apply_seller_channel_open(
                    &config,
                    &service_id,
                    &lease_id,
                    &channel_id,
                    &buyer_npub,
                    open,
                    capacity_sat,
                    request.now_unix,
                )?;
            }
            StreamingRoutePaymentPayload::BalanceUpdate(update) => {
                self.apply_seller_balance_update(
                    &config,
                    &service_id,
                    &lease_id,
                    &channel_id,
                    &buyer_npub,
                    update,
                    request.now_unix,
                )?;
            }
            StreamingRoutePaymentPayload::CooperativeClose(close) => {
                self.apply_seller_cooperative_close(
                    &config,
                    &service_id,
                    &lease_id,
                    &channel_id,
                    &buyer_npub,
                    close.final_paid_msat,
                    &close.payment,
                    request.now_unix,
                )?;
            }
            StreamingRoutePaymentPayload::CashuTokenLease(token_lease) => {
                validate_seller_token_lease(&config, token_lease, request.now_unix)?;
                return Err(anyhow!(
                    "paid route Cashu token leases require seller-side token redemption before routing; use Cashu Spilman channel payments"
                ));
            }
            StreamingRoutePaymentPayload::CooperativeCloseAck(_) => {
                return Err(anyhow!(
                    "seller cannot apply paid route cooperative close ack from buyer"
                ));
            }
        }

        let session_id = seller_session_id_for_lease(&lease_id);
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} was not created"))?;
        let decision = session.session.routing_decision(&config);
        let channel = self
            .channels
            .get(&channel_id)
            .ok_or_else(|| anyhow!("paid route channel {channel_id} was not created"))?;
        let lease = self
            .leases
            .get(&lease_id)
            .ok_or_else(|| anyhow!("paid route lease {lease_id} was not created"))?;
        let expires_at_unix = channel.expires_at_unix.min(lease.lease.expires_at_unix);
        let lifecycle_allows = paid_route_lifecycle_allows_routing(channel.status)
            && paid_route_lifecycle_allows_routing(lease.status);
        let allow_routing =
            lifecycle_allows && expires_at_unix > request.now_unix && decision.allow_routing;
        let state = if allow_routing {
            decision.state
        } else {
            PaidRouteAccessState::Suspended
        };

        Ok(ApplyPaidRouteSellerPaymentResult {
            service_id,
            lease_id,
            channel_id,
            session_id,
            buyer_npub,
            seller_npub,
            payload_type,
            paid_msat: session.session.payment.paid_msat,
            delivered_units: decision.delivered_units,
            amount_due_msat: decision.amount_due_msat,
            unpaid_msat: decision.unpaid_msat,
            allow_routing,
            state,
            changed: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_seller_channel_open(
        &mut self,
        config: &PaidExitConfig,
        service_id: &str,
        lease_id: &str,
        channel_id: &str,
        buyer_npub: &str,
        open: &cashu_service::StreamingRouteChannelOpen,
        capacity_sat: u64,
        now_unix: u64,
    ) -> Result<()> {
        self.ensure_seller_lease_slot_available(service_id, lease_id, channel_id, buyer_npub)?;
        let existing_channel_payment = self
            .channels
            .get(channel_id)
            .map(|channel| {
                ensure_seller_channel_matches(channel, service_id, buyer_npub)?;
                Ok::<u64, anyhow::Error>(channel.payment.paid_msat)
            })
            .transpose()?
            .unwrap_or(0);
        let paid_msat = existing_channel_payment.max(open.paid_msat);
        let status = initial_seller_session_status(config, paid_msat);
        let expires_at_unix = seller_channel_open_expiry(
            now_unix,
            config.channel.channel_expiry_secs,
            open.expires_unix,
        )?;
        let quote_id = seller_quote_id_for_lease(lease_id);
        let session_id = seller_session_id_for_lease(lease_id);
        let payment = PaidRoutePaymentState {
            mode: PaidRoutePaymentMode::CashuSpilman,
            channel_id: channel_id.to_string(),
            cashu_unit: open.unit.trim().to_string(),
            capacity_sat,
            paid_msat,
            updated_at_unix: now_unix,
            cashu_spilman_payment: Some(open.payment.clone()),
            cashu_token_lease: None,
        };

        self.upsert_quote(
            PaidRouteQuote {
                quote_id: quote_id.clone(),
                offer_id: service_id.to_string(),
                payment_mode: PaidRoutePaymentMode::CashuSpilman,
                channel_capacity_sat: capacity_sat,
                expires_at_unix,
                receiver_pubkey_hex: open.receiver_pubkey_hex.trim().to_string(),
            },
            now_unix,
        );
        self.upsert_lease(
            PaidRouteLease {
                lease_id: lease_id.to_string(),
                offer_id: service_id.to_string(),
                quote_id: quote_id.clone(),
                buyer_npub: buyer_npub.to_string(),
                starts_at_unix: now_unix,
                expires_at_unix,
            },
            status,
            now_unix,
        );

        let created_at_unix = self
            .channels
            .get(channel_id)
            .map(|channel| channel.created_at_unix)
            .unwrap_or(now_unix);
        let channel_status = self
            .channels
            .get(channel_id)
            .map(|channel| preserve_terminal_status(channel.status, status))
            .unwrap_or(status);
        self.upsert_channel(PaidRouteChannelRecord {
            channel_id: channel_id.to_string(),
            offer_id: service_id.to_string(),
            role: PaidRouteChannelRole::Seller,
            status: channel_status,
            payment: payment.clone(),
            mint_url: open.mint_url.trim().to_string(),
            counterparty_npub: buyer_npub.to_string(),
            created_at_unix,
            expires_at_unix,
            updated_at_unix: now_unix,
            error: String::new(),
        });

        if let Some(record) = self.sessions.get_mut(&session_id) {
            record.session.payment = payment;
            record.updated_at_unix = now_unix;
        } else {
            self.upsert_session(
                PaidRouteSession {
                    session_id,
                    lease_id: lease_id.to_string(),
                    usage: PaidRouteUsage::default(),
                    payment,
                    realized_exit_ip: None,
                    observed_country_code: None,
                    observed_asn: None,
                    quality: None,
                },
                now_unix,
            );
        }

        Ok(())
    }

    fn apply_seller_balance_update(
        &mut self,
        config: &PaidExitConfig,
        service_id: &str,
        lease_id: &str,
        channel_id: &str,
        buyer_npub: &str,
        update: &cashu_service::StreamingRouteBalanceUpdate,
        now_unix: u64,
    ) -> Result<()> {
        let session_id = seller_session_id_for_lease(lease_id);
        self.ensure_existing_seller_session(service_id, lease_id, channel_id, buyer_npub)?;
        if !self.sessions.contains_key(&session_id) {
            return Err(anyhow!("paid route session {session_id} does not exist"));
        }
        let (cashu_unit, capacity_sat) = {
            let channel = self.channels.get(channel_id).expect("validated channel");
            (
                paid_route_payment_cashu_unit(&channel.payment),
                channel.payment.capacity_sat,
            )
        };
        validate_streaming_route_cashu_payment_claim(
            &update.payment,
            channel_id,
            &cashu_unit,
            update.paid_msat,
            capacity_sat,
            false,
        )
        .map_err(|error| anyhow!("{error}"))?;
        let current_units = self.sessions[&session_id]
            .session
            .usage
            .billable_units_for_meter(config.pricing.meter);
        // Buyer and seller usage flushes are independent; keep seller-observed
        // usage authoritative. The buyer's delivered_units/amount_due_msat can
        // explain the signed balance update, but they must not inflate or gate
        // seller billing. A lagging update is still useful partial credit; the
        // admission decision below is based on seller-computed unpaid balance.
        let effective_delivered_units = current_units;
        let current_paid = self.sessions[&session_id].session.payment.paid_msat;
        validate_paid_route_payment_progress(
            "paid route balance update",
            update.paid_msat,
            current_paid,
            capacity_sat,
        )?;

        let status = initial_seller_session_status(config, update.paid_msat);
        {
            let channel = self
                .channels
                .get_mut(channel_id)
                .expect("validated channel");
            channel.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            channel.payment.paid_msat = update.paid_msat;
            channel.payment.updated_at_unix = now_unix;
            channel.payment.cashu_spilman_payment = Some(update.payment.clone());
            channel.payment.cashu_token_lease = None;
            channel.status = preserve_terminal_status(channel.status, status);
            channel.updated_at_unix = now_unix;
        }
        if let Some(lease) = self.leases.get_mut(lease_id) {
            lease.status = preserve_terminal_status(lease.status, status);
            lease.updated_at_unix = now_unix;
        }
        {
            let record = self
                .sessions
                .get_mut(&session_id)
                .expect("validated session");
            apply_delivered_units_for_meter(
                &mut record.session.usage,
                config.pricing.meter,
                effective_delivered_units,
            );
            record.session.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            record.session.payment.paid_msat = update.paid_msat;
            record.session.payment.updated_at_unix = now_unix;
            record.session.payment.cashu_spilman_payment = Some(update.payment.clone());
            record.session.payment.cashu_token_lease = None;
            record.updated_at_unix = now_unix;
        }

        Ok(())
    }

    fn apply_seller_cooperative_close(
        &mut self,
        config: &PaidExitConfig,
        service_id: &str,
        lease_id: &str,
        channel_id: &str,
        buyer_npub: &str,
        final_paid_msat: u64,
        payment: &CashuSpilmanPayment,
        now_unix: u64,
    ) -> Result<()> {
        let session_id = seller_session_id_for_lease(lease_id);
        self.ensure_existing_seller_session(service_id, lease_id, channel_id, buyer_npub)?;
        let (cashu_unit, capacity_sat) = {
            let channel = self.channels.get(channel_id).expect("validated channel");
            (
                paid_route_payment_cashu_unit(&channel.payment),
                channel.payment.capacity_sat,
            )
        };
        validate_streaming_route_cashu_payment_claim(
            payment,
            channel_id,
            &cashu_unit,
            final_paid_msat,
            capacity_sat,
            false,
        )
        .map_err(|error| anyhow!("{error}"))?;
        let current_paid = self.sessions[&session_id].session.payment.paid_msat;
        validate_paid_route_payment_progress(
            "paid route close",
            final_paid_msat,
            current_paid,
            capacity_sat,
        )?;
        let session_usage = self.sessions[&session_id].session.usage.clone();
        let computed_due = config.amount_due_msat(&session_usage);
        let tolerated_due = config.amount_due_msat_with_connection_minimum_skew(
            &session_usage,
            SELLER_CONNECTION_MINIMUM_PAYMENT_SKEW_MILLIS,
        );
        if final_paid_msat < tolerated_due {
            return Err(anyhow!(
                "paid route close underpays amount due: {} msat < {} msat",
                final_paid_msat,
                computed_due
            ));
        }

        {
            let channel = self
                .channels
                .get_mut(channel_id)
                .expect("validated channel");
            channel.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            channel.payment.paid_msat = final_paid_msat;
            channel.payment.updated_at_unix = now_unix;
            channel.payment.cashu_spilman_payment = Some(payment.clone());
            channel.payment.cashu_token_lease = None;
            channel.status = PaidRouteLifecycleStatus::Closing;
            channel.updated_at_unix = now_unix;
        }
        if let Some(lease) = self.leases.get_mut(lease_id) {
            lease.status = PaidRouteLifecycleStatus::Closing;
            lease.updated_at_unix = now_unix;
        }
        {
            let record = self
                .sessions
                .get_mut(&session_id)
                .expect("validated session");
            record.session.payment.mode = PaidRoutePaymentMode::CashuSpilman;
            record.session.payment.paid_msat = final_paid_msat;
            record.session.payment.updated_at_unix = now_unix;
            record.session.payment.cashu_spilman_payment = Some(payment.clone());
            record.session.payment.cashu_token_lease = None;
            record.updated_at_unix = now_unix;
        }

        Ok(())
    }

    fn ensure_seller_lease_slot_available(
        &self,
        service_id: &str,
        lease_id: &str,
        channel_id: &str,
        buyer_npub: &str,
    ) -> Result<()> {
        let expected_quote_id = seller_quote_id_for_lease(lease_id);
        let expected_session_id = seller_session_id_for_lease(lease_id);

        if let Some(quote) = self.quotes.get(&expected_quote_id)
            && quote.quote.offer_id != service_id
        {
            return Err(anyhow!(
                "paid route lease {lease_id} quote belongs to service {}, not {}",
                quote.quote.offer_id,
                service_id
            ));
        }

        if let Some(lease) = self.leases.get(lease_id) {
            if lease.lease.offer_id != service_id {
                return Err(anyhow!(
                    "paid route lease {} belongs to service {}, not {}",
                    lease_id,
                    lease.lease.offer_id,
                    service_id
                ));
            }
            if lease.lease.quote_id != expected_quote_id {
                return Err(anyhow!(
                    "paid route lease {lease_id} does not match expected seller quote"
                ));
            }
            if normalize_paid_route_npub(&lease.lease.buyer_npub, "buyer")? != buyer_npub {
                return Err(anyhow!(
                    "paid route payment buyer does not match existing lease buyer"
                ));
            }
        }

        if let Some(session) = self.sessions.get(&expected_session_id) {
            if session.session.lease_id != lease_id {
                return Err(anyhow!(
                    "paid route session {expected_session_id} does not match lease"
                ));
            }
            if session.session.payment.channel_id != channel_id {
                return Err(anyhow!(
                    "paid route lease {lease_id} is already bound to channel {}, not {}",
                    session.session.payment.channel_id,
                    channel_id
                ));
            }
        } else if self.leases.contains_key(lease_id) {
            return Err(anyhow!(
                "paid route lease {lease_id} already exists without a matching seller session"
            ));
        }

        for record in self.sessions.values() {
            if record.session.payment.channel_id == channel_id
                && record.session.lease_id != lease_id
            {
                return Err(anyhow!(
                    "paid route channel {channel_id} is already bound to lease {}",
                    record.session.lease_id
                ));
            }
        }

        Ok(())
    }

    fn ensure_existing_seller_session(
        &self,
        service_id: &str,
        lease_id: &str,
        channel_id: &str,
        buyer_npub: &str,
    ) -> Result<()> {
        let lease = self
            .leases
            .get(lease_id)
            .ok_or_else(|| anyhow!("paid route lease {lease_id} does not exist"))?;
        if lease.lease.offer_id != service_id {
            return Err(anyhow!(
                "paid route lease {} belongs to service {}, not {}",
                lease_id,
                lease.lease.offer_id,
                service_id
            ));
        }
        if normalize_paid_route_npub(&lease.lease.buyer_npub, "buyer")? != buyer_npub {
            return Err(anyhow!(
                "paid route payment buyer does not match lease buyer"
            ));
        }
        if matches!(
            lease.status,
            PaidRouteLifecycleStatus::Closed
                | PaidRouteLifecycleStatus::Closing
                | PaidRouteLifecycleStatus::Expired
                | PaidRouteLifecycleStatus::Failed
        ) {
            return Err(anyhow!("paid route lease {lease_id} is not open"));
        }

        let channel = self
            .channels
            .get(channel_id)
            .ok_or_else(|| anyhow!("paid route channel {channel_id} does not exist"))?;
        ensure_seller_channel_matches(channel, service_id, buyer_npub)?;
        if matches!(
            channel.status,
            PaidRouteLifecycleStatus::Closed
                | PaidRouteLifecycleStatus::Closing
                | PaidRouteLifecycleStatus::Expired
                | PaidRouteLifecycleStatus::Failed
        ) {
            return Err(anyhow!("paid route channel {channel_id} is not open"));
        }

        let session_id = seller_session_id_for_lease(lease_id);
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route session {session_id} does not exist"))?;
        if session.session.lease_id != lease_id || session.session.payment.channel_id != channel_id
        {
            return Err(anyhow!(
                "paid route session {session_id} does not match lease/channel"
            ));
        }
        Ok(())
    }

    pub fn record_seller_usage(
        &mut self,
        request: RecordPaidRouteSellerUsageRequest,
    ) -> Result<Option<RecordPaidRouteSellerUsageResult>> {
        if request.usage_delta.is_empty() {
            return Ok(None);
        }
        let buyer_pubkey = normalize_nostr_pubkey(&request.buyer_pubkey)
            .unwrap_or_else(|_| request.buyer_pubkey.trim().to_string());
        if buyer_pubkey.is_empty() {
            return Err(anyhow!("paid route buyer pubkey is empty"));
        }
        let Some(admission) =
            self.seller_admission_for_buyer(&request.config, request.now_unix, &buyer_pubkey)
        else {
            return Ok(None);
        };

        let Some(record) = self.sessions.get_mut(&admission.session_id) else {
            return Ok(None);
        };
        let before = record.session.usage.clone();
        apply_usage_delta(&mut record.session.usage, &request.usage_delta);
        let changed = record.session.usage != before;
        if changed {
            record.updated_at_unix = request.now_unix;
        }

        let decision = record.session.routing_decision(&request.config);
        Ok(Some(RecordPaidRouteSellerUsageResult {
            buyer_pubkey,
            buyer_npub: admission.buyer_npub,
            session_id: admission.session_id,
            lease_id: admission.lease_id,
            channel_id: admission.channel_id,
            usage: record.session.usage.clone(),
            paid_msat: decision.paid_msat,
            amount_due_msat: decision.amount_due_msat,
            unpaid_msat: decision.unpaid_msat,
            allow_routing: decision.allow_routing,
            state: decision.state,
            changed,
        }))
    }

    pub fn seller_admissions(
        &self,
        config: &PaidExitConfig,
        now_unix: u64,
    ) -> Vec<PaidRouteSellerAdmission> {
        let mut by_buyer = BTreeMap::<String, PaidRouteSellerAdmission>::new();
        for record in self.sessions.values() {
            let Some(admission) = self.seller_admission_for_session(config, now_unix, record)
            else {
                continue;
            };
            match by_buyer.get(&admission.buyer_pubkey) {
                None => {
                    by_buyer.insert(admission.buyer_pubkey.clone(), admission);
                }
                Some(existing) if seller_admission_preferred(&admission, existing) => {
                    by_buyer.insert(admission.buyer_pubkey.clone(), admission);
                }
                Some(_) => {}
            }
        }
        by_buyer.into_values().collect()
    }

    pub fn seller_collection_states(
        &self,
        config: &PaidExitConfig,
        now_unix: u64,
    ) -> Vec<PaidRouteSellerCollectionState> {
        if !config.enabled {
            return Vec::new();
        }
        let mut states = self
            .sessions
            .values()
            .filter_map(|record| self.seller_collection_state_for_record(config, now_unix, record))
            .collect::<Vec<_>>();
        states.sort_by(|left, right| {
            right
                .auto_collect_due
                .cmp(&left.auto_collect_due)
                .then_with(|| right.updated_at_unix.cmp(&left.updated_at_unix))
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        states
    }

    pub fn seller_collection_state_for_session(
        &self,
        config: &PaidExitConfig,
        now_unix: u64,
        session_id: &str,
    ) -> Option<PaidRouteSellerCollectionState> {
        if !config.enabled {
            return None;
        }
        self.sessions
            .get(session_id)
            .and_then(|record| self.seller_collection_state_for_record(config, now_unix, record))
    }

    fn seller_admission_for_buyer(
        &self,
        config: &PaidExitConfig,
        now_unix: u64,
        buyer_pubkey: &str,
    ) -> Option<PaidRouteSellerAdmission> {
        let buyer_pubkey = normalize_nostr_pubkey(buyer_pubkey)
            .unwrap_or_else(|_| buyer_pubkey.trim().to_string());
        self.sessions
            .values()
            .filter_map(|record| self.seller_admission_for_session(config, now_unix, record))
            .filter(|admission| admission.buyer_pubkey == buyer_pubkey)
            .max_by(|left, right| {
                if seller_admission_preferred(left, right) {
                    std::cmp::Ordering::Greater
                } else if seller_admission_preferred(right, left) {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
    }

    fn seller_admission_for_session(
        &self,
        config: &PaidExitConfig,
        now_unix: u64,
        record: &PaidRouteSessionRecord,
    ) -> Option<PaidRouteSellerAdmission> {
        let session = &record.session;
        let lease_record = self.leases.get(&session.lease_id)?;
        let channel = self.channels.get(&session.payment.channel_id)?;
        if channel.role != PaidRouteChannelRole::Seller {
            return None;
        }

        let buyer_npub = normalize_paid_route_npub(&lease_record.lease.buyer_npub, "buyer").ok()?;
        let buyer_pubkey = normalize_nostr_pubkey(&buyer_npub).ok()?;
        let decision = session.routing_decision(config);
        let expires_at_unix = lease_record
            .lease
            .expires_at_unix
            .min(channel.expires_at_unix);
        let lifecycle_allows = paid_route_lifecycle_allows_routing(lease_record.status)
            && paid_route_lifecycle_allows_routing(channel.status);
        let not_expired = expires_at_unix > now_unix;
        let allow_routing = lifecycle_allows && not_expired && decision.allow_routing;
        let state = if allow_routing {
            decision.state
        } else {
            PaidRouteAccessState::Suspended
        };

        Some(PaidRouteSellerAdmission {
            buyer_pubkey,
            buyer_npub,
            session_id: session.session_id.clone(),
            lease_id: session.lease_id.clone(),
            channel_id: session.payment.channel_id.clone(),
            state,
            allow_routing,
            amount_due_msat: decision.amount_due_msat,
            paid_msat: decision.paid_msat,
            unpaid_msat: decision.unpaid_msat,
            expires_at_unix,
            updated_at_unix: record.updated_at_unix.max(channel.updated_at_unix),
        })
    }

    fn seller_collection_state_for_record(
        &self,
        _config: &PaidExitConfig,
        now_unix: u64,
        record: &PaidRouteSessionRecord,
    ) -> Option<PaidRouteSellerCollectionState> {
        let session = &record.session;
        let lease_record = self.leases.get(&session.lease_id)?;
        let channel = self.channels.get(&session.payment.channel_id)?;
        if channel.role != PaidRouteChannelRole::Seller {
            return None;
        }

        let buyer_npub = normalize_paid_route_npub(&lease_record.lease.buyer_npub, "buyer").ok()?;
        let expires_at_unix = lease_record
            .lease
            .expires_at_unix
            .min(channel.expires_at_unix);
        let expired = expires_at_unix > 0 && expires_at_unix <= now_unix;
        let terminally_collected = matches!(
            channel.status,
            PaidRouteLifecycleStatus::Closed | PaidRouteLifecycleStatus::Failed
        ) || matches!(
            lease_record.status,
            PaidRouteLifecycleStatus::Closed | PaidRouteLifecycleStatus::Failed
        );
        let has_spilman_payment =
            matches!(session.payment.mode, PaidRoutePaymentMode::CashuSpilman)
                && (session.payment.cashu_spilman_payment.is_some()
                    || channel.payment.cashu_spilman_payment.is_some());
        let paid_msat = session.payment.paid_msat.max(channel.payment.paid_msat);
        let collectable = !terminally_collected
            && has_spilman_payment
            && paid_msat > 0
            && !channel.channel_id.trim().is_empty();
        let auto_collect_due = collectable && expired;
        let reason = if auto_collect_due {
            "expired"
        } else if collectable {
            "manual"
        } else if terminally_collected {
            "closed"
        } else {
            ""
        }
        .to_string();

        Some(PaidRouteSellerCollectionState {
            buyer_npub,
            session_id: session.session_id.clone(),
            lease_id: session.lease_id.clone(),
            channel_id: session.payment.channel_id.clone(),
            collectable,
            manual_collect: collectable,
            auto_collect_due,
            reason,
            paid_msat,
            expires_at_unix,
            due_at_unix: if collectable { expires_at_unix } else { 0 },
            updated_at_unix: record.updated_at_unix.max(channel.updated_at_unix),
        })
        .filter(|state| state.collectable)
    }

    fn resolve_offer(&self, selector: &str) -> Result<(String, &PaidRouteOfferRecord)> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err(anyhow!("paid route offer selector is empty"));
        }
        if let Some(record) = self.offers.get(selector) {
            return Ok((selector.to_string(), record));
        }

        let matches = self
            .offers
            .iter()
            .filter(|(_, record)| {
                record.offer.offer_id == selector || record.offer.seller_npub == selector
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err(anyhow!("paid route offer '{selector}' was not found")),
            [(key, record)] => Ok(((*key).clone(), *record)),
            _ => Err(anyhow!(
                "paid route offer selector '{selector}' is ambiguous; use the full offer key"
            )),
        }
    }

    fn ensure_buyer_mint_present(&mut self, mint_url: &str, now_unix: u64) -> bool {
        let mint_url = mint_url.trim();
        if mint_url.is_empty() {
            return false;
        }
        if !self.wallet.mints.iter().any(|mint| mint.url == mint_url) {
            return self.upsert_wallet_mint(mint_url, "", None, now_unix);
        }
        if self.wallet.default_mint.trim().is_empty() {
            self.wallet.default_mint = mint_url.to_string();
            return true;
        }
        false
    }

    fn retain_valid(&mut self) {
        for record in self.offers.values_mut() {
            if let Ok(offer) = record.signed_offer.offer() {
                record.offer = offer;
            }
            record.relay_urls = normalize_relay_list(record.relay_urls.clone());
            if let Some(score) = record.rating_score {
                record.rating_score = Some(score.clamp(-100, 100));
            } else {
                record.rating_updated_at_unix = 0;
            }
        }
        self.offers.retain(|key, record| {
            record.signed_offer.verify().is_ok()
                && record.signed_offer.offer().is_ok_and(|offer| {
                    paid_route_offer_store_key(&offer.seller_npub, &offer.offer_id) == *key
                })
        });
        self.wallet.mints.retain(|mint| !mint.url.trim().is_empty());
        self.wallet
            .mints
            .sort_by(|left, right| left.url.cmp(&right.url));
        self.wallet
            .mints
            .dedup_by(|left, right| left.url == right.url);
        if self.wallet.default_mint.trim().is_empty()
            && let Some(first) = self.wallet.mints.first()
        {
            self.wallet.default_mint = first.url.clone();
        }
    }
}

pub fn paid_route_store_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("paid-routes.json")
}

pub fn load_paid_route_store(path: &Path) -> Result<PaidRouteStore> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(PaidRouteStore::default()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read paid route store {}", path.display()));
        }
    };

    let mut store = match serde_json::from_str::<PaidRouteStore>(&raw) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "discarding unreadable paid route store {}: {error}",
                path.display()
            );
            return Ok(PaidRouteStore::default());
        }
    };
    store.retain_valid();
    Ok(store)
}

pub fn write_paid_route_store(path: &Path, store: &PaidRouteStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(store)
        .with_context(|| format!("failed to serialize paid route store {}", path.display()))?;
    let mut tmp = path.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("paid-routes.json"));
    name.push(".tmp");
    tmp.set_file_name(name);

    fs::write(&tmp, raw)
        .with_context(|| format!("failed to write paid route temp {}", tmp.display()))?;
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "failed to rename paid route store {} -> {}: {error}",
                tmp.display(),
                path.display()
            ),
        )
        .into());
    }
    Ok(())
}

pub fn upsert_paid_route_offer(
    path: &Path,
    signed_offer: SignedPaidRouteOffer,
    relay_urls: Vec<String>,
    seen_at_unix: u64,
) -> Result<bool> {
    let mut store = load_paid_route_store(path)?;
    let changed = store.upsert_signed_offer(signed_offer, relay_urls, seen_at_unix)?;
    if changed {
        write_paid_route_store(path, &store)?;
    }
    Ok(changed)
}

pub fn paid_route_offer_store_key(seller_npub: &str, offer_id: &str) -> String {
    format!("{}:{}", seller_npub.trim(), offer_id.trim())
}

fn default_version() -> u8 {
    CURRENT_VERSION
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn paid_route_offer_autoselect_score(record: &PaidRouteOfferRecord) -> i64 {
    record.rating_score.unwrap_or_default()
}

fn normalize_relay_list(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn merge_sorted_strings(left: &[String], right: Vec<String>) -> Vec<String> {
    let mut out = left.to_vec();
    out.extend(right);
    normalize_relay_list(out)
}

fn upsert_record<T: PartialEq>(records: &mut BTreeMap<String, T>, key: String, record: T) -> bool {
    if records
        .get(&key)
        .is_some_and(|existing| existing == &record)
    {
        return false;
    }
    records.insert(key, record);
    true
}

fn select_buyer_mint(
    offer: &PaidRouteOffer,
    wallet: &PaidRouteWalletState,
    requested: Option<&str>,
) -> Result<String> {
    let accepted_mints = normalize_mint_list(&offer.channel.accepted_mints);
    if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        if !accepted_mints.is_empty() && !accepted_mints.iter().any(|mint| mint == requested) {
            return Err(anyhow!(
                "Cashu mint {requested} is not accepted by paid route offer {}",
                offer.offer_id
            ));
        }
        return Ok(requested.to_string());
    }

    let default_mint = wallet.default_mint.trim();
    if !default_mint.is_empty()
        && (accepted_mints.is_empty() || accepted_mints.iter().any(|mint| mint == default_mint))
    {
        return Ok(default_mint.to_string());
    }

    if let Some(wallet_mint) = wallet.mints.iter().find(|wallet_mint| {
        let url = wallet_mint.url.trim();
        !url.is_empty()
            && (accepted_mints.is_empty() || accepted_mints.iter().any(|mint| mint == url))
    }) {
        return Ok(wallet_mint.url.trim().to_string());
    }

    if let Some(first_accepted) = accepted_mints.first() {
        return Ok(first_accepted.clone());
    }

    if !paid_route_offer_requires_payment(offer) {
        return Ok(String::new());
    }

    Err(anyhow!(
        "paid route offer {} has no accepted Cashu mint",
        offer.offer_id
    ))
}

fn requested_channel_capacity(offer: &PaidRouteOffer, requested: Option<u64>) -> Result<u64> {
    let max_capacity = offer.channel.max_channel_capacity_sat.max(1);
    let capacity = requested.unwrap_or(max_capacity);
    if capacity == 0 {
        return Err(anyhow!(
            "paid route channel capacity must be greater than zero"
        ));
    }
    if capacity > max_capacity {
        return Err(anyhow!(
            "paid route channel capacity {capacity} sat exceeds offer maximum {max_capacity} sat"
        ));
    }
    Ok(capacity)
}

fn paid_route_offer_requires_payment(offer: &PaidRouteOffer) -> bool {
    offer.pricing.price_msat > 0 || offer.pricing.connection_minimum_msat_per_day > 0
}

fn paid_route_offer_requires_payment_before_routing(offer: &PaidRouteOffer) -> bool {
    paid_route_offer_requires_payment(offer) && offer.channel.free_probe_units == 0
}

fn paid_exit_config_requires_payment(config: &PaidExitConfig) -> bool {
    config.pricing.price_msat > 0 || config.pricing.connection_minimum_msat_per_day > 0
}

fn initial_buyer_session_status(
    offer: &PaidRouteOffer,
    initial_paid_msat: u64,
) -> PaidRouteLifecycleStatus {
    if !paid_route_offer_requires_payment(offer) || initial_paid_msat > 0 {
        PaidRouteLifecycleStatus::Active
    } else if offer.channel.free_probe_units > 0 {
        PaidRouteLifecycleStatus::Probing
    } else {
        PaidRouteLifecycleStatus::Opening
    }
}

fn initial_seller_session_status(
    config: &PaidExitConfig,
    paid_msat: u64,
) -> PaidRouteLifecycleStatus {
    if !paid_exit_config_requires_payment(config) || paid_msat > 0 {
        PaidRouteLifecycleStatus::Active
    } else if config.channel.free_probe_units > 0 {
        PaidRouteLifecycleStatus::Probing
    } else {
        PaidRouteLifecycleStatus::Opening
    }
}

fn paid_route_lifecycle_allows_routing(status: PaidRouteLifecycleStatus) -> bool {
    matches!(
        status,
        PaidRouteLifecycleStatus::Opening
            | PaidRouteLifecycleStatus::Probing
            | PaidRouteLifecycleStatus::Active
    )
}

fn paid_route_session_has_payment_material(
    session: &PaidRouteSession,
    channel: &PaidRouteChannelRecord,
) -> bool {
    match session.payment.mode {
        PaidRoutePaymentMode::CashuSpilman => {
            session.payment.cashu_spilman_payment.is_some()
                || channel.payment.cashu_spilman_payment.is_some()
        }
        PaidRoutePaymentMode::CashuTokenLease => {
            session.payment.cashu_token_lease.is_some()
                || channel.payment.cashu_token_lease.is_some()
        }
    }
}

fn preserve_terminal_status(
    current: PaidRouteLifecycleStatus,
    incoming: PaidRouteLifecycleStatus,
) -> PaidRouteLifecycleStatus {
    match current {
        PaidRouteLifecycleStatus::Closing
        | PaidRouteLifecycleStatus::Closed
        | PaidRouteLifecycleStatus::Expired
        | PaidRouteLifecycleStatus::Failed => current,
        _ => incoming,
    }
}

fn ensure_open_buyer_channel(
    channel: &PaidRouteChannelRecord,
    lease: &PaidRouteLeaseRecord,
) -> Result<()> {
    if matches!(
        channel.status,
        PaidRouteLifecycleStatus::Closed
            | PaidRouteLifecycleStatus::Closing
            | PaidRouteLifecycleStatus::Expired
            | PaidRouteLifecycleStatus::Failed
    ) {
        return Err(anyhow!(
            "paid route buyer channel {} is not open",
            channel.channel_id
        ));
    }
    if matches!(
        lease.status,
        PaidRouteLifecycleStatus::Closed
            | PaidRouteLifecycleStatus::Closing
            | PaidRouteLifecycleStatus::Expired
            | PaidRouteLifecycleStatus::Failed
    ) {
        return Err(anyhow!(
            "paid route buyer lease {} is not open",
            lease.lease.lease_id
        ));
    }
    Ok(())
}

fn seller_admission_preferred(
    candidate: &PaidRouteSellerAdmission,
    existing: &PaidRouteSellerAdmission,
) -> bool {
    match (candidate.allow_routing, existing.allow_routing) {
        (true, false) => true,
        (false, true) => false,
        _ => candidate.updated_at_unix > existing.updated_at_unix,
    }
}

fn validate_seller_open_payment(
    config: &PaidExitConfig,
    _seller_pubkey_hex: &str,
    channel_id: &str,
    open: &cashu_service::StreamingRouteChannelOpen,
) -> Result<()> {
    let mint_url = open.mint_url.trim();
    if paid_exit_config_requires_payment(config) {
        let accepted_mints = normalize_mint_list(&config.channel.accepted_mints);
        if accepted_mints.is_empty() {
            return Err(anyhow!(
                "paid exit seller config must accept at least one Cashu mint"
            ));
        }
        if !accepted_mints.iter().any(|mint| mint == mint_url) {
            return Err(anyhow!(
                "Cashu mint {mint_url} is not accepted by this paid exit"
            ));
        }
    }

    let capacity_sat = paid_route_channel_capacity_sat(&open.unit, open.capacity)?;
    if capacity_sat == 0 {
        return Err(anyhow!(
            "paid route channel capacity must be greater than zero"
        ));
    }
    if capacity_sat > config.channel.max_channel_capacity_sat {
        return Err(anyhow!(
            "paid route channel capacity {capacity_sat} sat exceeds seller maximum {} sat",
            config.channel.max_channel_capacity_sat
        ));
    }
    validate_paid_route_payment_progress(
        "paid route opening balance",
        open.paid_msat,
        0,
        capacity_sat,
    )?;
    validate_streaming_route_cashu_payment_claim(
        &open.payment,
        channel_id,
        &open.unit,
        open.paid_msat,
        capacity_sat,
        true,
    )
    .map_err(|error| anyhow!("{error}"))?;
    if open.receiver_pubkey_hex.trim().is_empty() {
        return Err(anyhow!("paid route channel receiver pubkey is empty"));
    }
    normalize_paid_route_receiver_pubkey(open.receiver_pubkey_hex.trim())
        .map_err(|error| anyhow!("invalid paid route channel receiver pubkey: {error}"))?;
    Ok(())
}

fn paid_route_offer_receiver_pubkey_hex(
    offer: &PaidRouteOffer,
    seller_pubkey: &PublicKey,
) -> Result<String> {
    if offer.receiver_pubkey_hex.trim().is_empty() {
        return Ok(seller_pubkey.to_hex());
    }
    normalize_paid_route_receiver_pubkey(&offer.receiver_pubkey_hex)
}

fn normalize_paid_route_receiver_pubkey(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex.len() == 66
        && matches!(&hex[..2], "02" | "03")
        && hex.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Ok(hex.to_ascii_lowercase());
    }
    if hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(hex.to_ascii_lowercase());
    }
    Err(anyhow!("invalid paid route receiver pubkey"))
}

fn validate_seller_token_lease(
    config: &PaidExitConfig,
    token_lease: &StreamingRouteCashuTokenLease,
    now_unix: u64,
) -> Result<()> {
    let normalized =
        create_streaming_route_cashu_token_lease(StreamingRouteCashuTokenLeaseRequest {
            channel_id: token_lease.channel_id.clone(),
            mint_url: token_lease.mint_url.clone(),
            unit: token_lease.unit.clone(),
            amount: token_lease.amount,
            paid_msat: Some(token_lease.paid_msat),
            expires_unix: token_lease.expires_unix,
            token: token_lease.token.clone(),
        })
        .map_err(|error| anyhow!("{error}"))?;
    if normalized.expires_unix <= now_unix {
        return Err(anyhow!("paid route token lease is already expired"));
    }

    let mint_url = normalized.mint_url.trim();
    if paid_exit_config_requires_payment(config) {
        let accepted_mints = normalize_mint_list(&config.channel.accepted_mints);
        if accepted_mints.is_empty() {
            return Err(anyhow!(
                "paid exit seller config must accept at least one Cashu mint"
            ));
        }
        if !accepted_mints.iter().any(|mint| mint == mint_url) {
            return Err(anyhow!(
                "Cashu mint {mint_url} is not accepted by this paid exit"
            ));
        }
    }

    let capacity_sat = paid_route_channel_capacity_sat(&normalized.unit, normalized.amount)?;
    if capacity_sat == 0 {
        return Err(anyhow!(
            "paid route token lease amount must be greater than zero"
        ));
    }
    if capacity_sat > config.channel.max_channel_capacity_sat {
        return Err(anyhow!(
            "paid route token lease amount {capacity_sat} sat exceeds seller maximum {} sat",
            config.channel.max_channel_capacity_sat
        ));
    }
    Ok(())
}

fn paid_route_channel_capacity_sat(unit: &str, capacity: u64) -> Result<u64> {
    streaming_route_cashu_capacity_sat(unit, capacity).map_err(|error| anyhow!("{error}"))
}

fn paid_route_payment_cashu_unit(payment: &PaidRoutePaymentState) -> String {
    let unit = payment.cashu_unit.trim();
    if unit.is_empty() {
        "sat".to_string()
    } else {
        unit.to_string()
    }
}

fn cashu_payment_balance_msat(unit: &str, balance: u64) -> Result<u64> {
    streaming_route_cashu_balance_msat(unit, balance).map_err(|error| anyhow!("{error}"))
}

fn cashu_payment_target_msat(unit: &str, paid_msat: u64) -> Result<u64> {
    let balance = streaming_route_cashu_balance_for_msat(unit, paid_msat)
        .map_err(|error| anyhow!("{error}"))?;
    cashu_payment_balance_msat(unit, balance)
}

fn validate_cashu_spilman_payment_claim(
    payment: &CashuSpilmanPayment,
    channel_id: &str,
    unit: &str,
    paid_msat: u64,
    capacity_sat: u64,
    require_funding: bool,
) -> Result<()> {
    validate_streaming_route_cashu_payment_claim(
        payment,
        channel_id,
        unit,
        paid_msat,
        capacity_sat,
        require_funding,
    )
    .map(|_| ())
    .map_err(|error| anyhow!("{error}"))
}

fn cashu_channel_capacity_for_unit(capacity_sat: u64, unit: &str) -> Result<u64> {
    streaming_route_cashu_capacity_for_sat(unit, capacity_sat).map_err(|error| anyhow!("{error}"))
}

fn validate_paid_route_payment_progress(
    label: &str,
    paid_msat: u64,
    previous_paid_msat: u64,
    capacity_sat: u64,
) -> Result<()> {
    validate_streaming_route_cashu_payment_progress(
        label,
        paid_msat,
        previous_paid_msat,
        capacity_sat,
    )
    .map(|_| ())
    .map_err(|error| anyhow!("{error}"))
}

fn seller_channel_open_expiry(
    now_unix: u64,
    configured_expiry_secs: u64,
    requested_expires_unix: u64,
) -> Result<u64> {
    let configured = now_unix.saturating_add(configured_expiry_secs.max(1));
    let expires_at_unix = if requested_expires_unix == 0 {
        configured
    } else {
        configured.min(requested_expires_unix)
    };
    if expires_at_unix <= now_unix {
        return Err(anyhow!("paid route channel opening is already expired"));
    }
    Ok(expires_at_unix)
}

fn seller_quote_id_for_lease(lease_id: &str) -> String {
    format!("seller-quote-{}", sanitize_id_component(lease_id))
}

fn seller_session_id_for_lease(lease_id: &str) -> String {
    format!("seller-session-{}", sanitize_id_component(lease_id))
}

fn paid_route_payment_payload_type(payload: &StreamingRoutePaymentPayload) -> &'static str {
    match payload {
        StreamingRoutePaymentPayload::ChannelOpen(_) => "channel_open",
        StreamingRoutePaymentPayload::BalanceUpdate(_) => "balance_update",
        StreamingRoutePaymentPayload::CooperativeClose(_) => "cooperative_close",
        StreamingRoutePaymentPayload::CooperativeCloseAck(_) => "cooperative_close_ack",
        StreamingRoutePaymentPayload::CashuTokenLease(_) => "cashu_token_lease",
    }
}

fn trimmed_required(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(anyhow!("{label} is empty"))
    } else {
        Ok(value.to_string())
    }
}

fn normalize_optional_probe_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_country_code(value: Option<String>) -> Option<String> {
    normalize_optional_probe_string(value).map(|value| value.to_ascii_uppercase())
}

fn ensure_seller_channel_matches(
    channel: &PaidRouteChannelRecord,
    service_id: &str,
    buyer_npub: &str,
) -> Result<()> {
    if channel.role != PaidRouteChannelRole::Seller {
        return Err(anyhow!(
            "paid route channel {} is not a seller channel",
            channel.channel_id
        ));
    }
    if channel.offer_id != service_id {
        return Err(anyhow!(
            "paid route channel {} belongs to service {}, not {}",
            channel.channel_id,
            channel.offer_id,
            service_id
        ));
    }
    if normalize_paid_route_npub(&channel.counterparty_npub, "buyer")? != buyer_npub {
        return Err(anyhow!(
            "paid route payment buyer does not match channel counterparty"
        ));
    }
    Ok(())
}

fn apply_delivered_units_for_meter(
    usage: &mut PaidRouteUsage,
    meter: PaidRouteMeter,
    delivered_units: u64,
) {
    match meter {
        PaidRouteMeter::Milliseconds => {
            usage.active_millis = usage.active_millis.max(delivered_units);
        }
        PaidRouteMeter::Bytes => {
            usage.billable_bytes = usage.billable_bytes.max(delivered_units);
        }
        PaidRouteMeter::Packets => {
            usage.billable_packets = usage.billable_packets.max(delivered_units);
        }
    }
}

fn paid_route_amount_due_for_delivered_units(
    config: &PaidExitConfig,
    usage: &PaidRouteUsage,
    delivered_units: u64,
) -> u64 {
    let mut usage = usage.clone();
    apply_delivered_units_for_meter(&mut usage, config.pricing.meter, delivered_units);
    config.amount_due_msat(&usage)
}

fn apply_usage_delta(usage: &mut PaidRouteUsage, delta: &PaidRouteUsage) {
    usage.add_assign(delta);
}

fn paid_route_buyer_session_id_suffix(offer_key: &str, offer_id: &str, now_unix: u64) -> String {
    let mut hasher = DefaultHasher::new();
    offer_key.hash(&mut hasher);
    offer_id.hash(&mut hasher);
    now_unix.hash(&mut hasher);
    let readable = sanitize_id_component(offer_id);
    format!("{readable}-{now_unix}-{:016x}", hasher.finish())
}

fn sanitize_id_component(value: &str) -> String {
    let mut out = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "route".to_string()
    } else if out.len() > 48 {
        out.chars().take(48).collect()
    } else {
        out
    }
}

fn normalize_mint_list(values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .flat_map(|value| value.split([',', '\n', '\r', '\t']))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn normalize_paid_route_npub(value: &str, role: &str) -> Result<String> {
    let public_key = PublicKey::parse(value.trim())
        .map_err(|error| anyhow!("invalid paid route {role} npub: {error}"))?;
    public_key
        .to_bech32()
        .context("failed to encode paid route npub")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paid_routes::{
        PaidExitConfig, PaidRouteAccessPolicy, PaidRouteChannelTerms, PaidRouteIpSupport,
        PaidRouteLocationHint, PaidRouteMeter, PaidRoutePaymentMode, PaidRoutePaymentState,
        PaidRoutePricing, PaidRoutePrivateVpnAccess, PaidRouteQualityMetrics, PaidRouteUsage,
        signed_paid_exit_offer_from_config, signed_paid_exit_offer_from_config_with_receiver,
    };
    use cashu_service::{
        CashuSpilmanPayment, CashuSpilmanPaymentReceiver, CashuSpilmanPaymentReceiverValidation,
        CashuSpilmanPaymentSigner, StreamingRouteBalanceUpdate, StreamingRouteChannelOpen,
        StreamingRouteCooperativeClose, StreamingRoutePaymentEnvelope,
        StreamingRoutePaymentPayload,
    };
    use nostr_sdk::prelude::{Keys, ToBech32};
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(name: &str) -> Self {
            let seq = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("nvpn-paid-route-store-{name}-{now}-{seq}"));
            fs::create_dir_all(&path).expect("create scratch dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn paid_buyer_session_without_payment_does_not_allow_routing() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.connection_minimum_msat_per_day = 1;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let (store, session_id, _) = buyer_store_with_session(&seller, &buyer, &config);

        assert!(paid_route_lifecycle_allows_routing(
            PaidRouteLifecycleStatus::Opening
        ));
        assert!(
            !store
                .buyer_session_allows_routing(&session_id, 121)
                .expect("route readiness")
        );
    }

    #[test]
    fn paid_buyer_session_with_opening_payment_allows_routing() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.connection_minimum_msat_per_day = 1;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let (mut store, session_id, _) = buyer_store_with_session(&seller, &buyer, &config);
        let channel_id = "spilman-real-channel-1";
        store
            .attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
                session_id: session_id.clone(),
                channel_id: channel_id.to_string(),
                cashu_unit: "sat".to_string(),
                capacity_sat: 10,
                paid_msat: Some(0),
                payment: sample_spilman_payment(channel_id, 0),
                now_unix: 130,
            })
            .expect("attach funded channel open");

        assert_eq!(
            store.channels[channel_id].status,
            PaidRouteLifecycleStatus::Opening
        );
        assert!(
            store
                .buyer_session_allows_routing(&session_id, 131)
                .expect("route readiness")
        );
    }

    #[test]
    fn paid_route_store_path_sits_next_to_config() {
        let path = paid_route_store_file_path(Path::new("/tmp/nvpn/config.toml"));

        assert_eq!(path, PathBuf::from("/tmp/nvpn/paid-routes.json"));
    }

    #[test]
    fn paid_route_store_persists_wallet_offer_session_and_channel_state() {
        let scratch = ScratchDir::new("roundtrip");
        let store_path = scratch.path().join("paid-routes.json");
        let seller = Keys::generate();
        let signed_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            100,
        )
        .expect("signed offer");

        let mut store = PaidRouteStore::default();
        assert!(store.upsert_wallet_mint(
            " https://mint.minibits.cash/Bitcoin ",
            "Minibits",
            Some(123_000),
            110
        ));
        assert!(
            store
                .upsert_signed_offer(
                    signed_offer.clone(),
                    vec!["wss://relay.example".to_string()],
                    111
                )
                .expect("upsert offer")
        );
        assert!(store.upsert_channel(PaidRouteChannelRecord {
            channel_id: "channel-1".to_string(),
            offer_id: "internet-exit".to_string(),
            role: PaidRouteChannelRole::Buyer,
            status: PaidRouteLifecycleStatus::Active,
            payment: PaidRoutePaymentState {
                mode: PaidRoutePaymentMode::CashuSpilman,
                channel_id: "channel-1".to_string(),
                capacity_sat: 100,
                paid_msat: 42_000,
                updated_at_unix: 112,
                ..PaidRoutePaymentState::default()
            },
            mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
            counterparty_npub: signed_offer.offer().expect("offer").seller_npub,
            created_at_unix: 111,
            expires_at_unix: 711,
            updated_at_unix: 112,
            error: String::new(),
        }));
        assert!(store.upsert_session(
            PaidRouteSession {
                session_id: "session-1".to_string(),
                lease_id: "lease-1".to_string(),
                usage: PaidRouteUsage {
                    active_millis: 1000,
                    tx_bytes: 10,
                    rx_bytes: 20,
                    tx_packets: 1,
                    rx_packets: 2,
                    billable_bytes: 30,
                    billable_packets: 3,
                },
                payment: PaidRoutePaymentState {
                    mode: PaidRoutePaymentMode::CashuSpilman,
                    channel_id: "channel-1".to_string(),
                    capacity_sat: 100,
                    paid_msat: 42_000,
                    updated_at_unix: 112,
                    ..PaidRoutePaymentState::default()
                },
                realized_exit_ip: Some("198.51.100.42".to_string()),
                observed_country_code: Some("FI".to_string()),
                observed_asn: Some(14593),
                quality: Some(PaidRouteQualityMetrics {
                    latency_ms: Some(42),
                    jitter_ms: Some(7),
                    packet_loss_ppm: Some(500),
                    down_bps: Some(10_000_000),
                    up_bps: Some(1_000_000),
                    uptime_secs: Some(3600),
                    last_seen_unix: Some(112),
                }),
            },
            112
        ));

        write_paid_route_store(&store_path, &store).expect("write store");
        let loaded = load_paid_route_store(&store_path).expect("load store");

        assert_eq!(
            loaded.wallet.default_mint,
            "https://mint.minibits.cash/Bitcoin"
        );
        assert_eq!(loaded.wallet.mints.len(), 1);
        assert_eq!(loaded.offers.len(), 1);
        assert_eq!(loaded.channels["channel-1"].payment.paid_msat, 42_000);
        assert_eq!(loaded.sessions["session-1"].session.usage.rx_bytes, 20);
        assert_eq!(
            loaded.sessions["session-1"]
                .session
                .quality
                .as_ref()
                .and_then(|quality| quality.jitter_ms),
            Some(7)
        );
    }

    #[test]
    fn paid_route_wallet_mints_can_be_defaulted_and_removed() {
        let mut store = PaidRouteStore::default();

        assert!(store.set_default_mint("https://mint.minibits.cash/Bitcoin"));
        assert_eq!(
            store.wallet.default_mint,
            "https://mint.minibits.cash/Bitcoin"
        );
        assert_eq!(store.wallet.mints.len(), 1);

        assert!(store.upsert_wallet_mint("https://mint.example", "Example", Some(10_000), 100));
        assert!(store.set_default_mint("https://mint.example"));
        assert_eq!(store.wallet.default_mint, "https://mint.example");

        assert!(store.remove_wallet_mint("https://mint.example"));
        assert_eq!(
            store.wallet.default_mint,
            "https://mint.minibits.cash/Bitcoin"
        );
        assert!(!store.remove_wallet_mint("https://missing.example"));
    }

    #[test]
    fn paid_route_store_updates_session_probe_results() {
        let mut store = PaidRouteStore::default();
        assert!(store.upsert_session(
            PaidRouteSession {
                session_id: "session-1".to_string(),
                lease_id: "lease-1".to_string(),
                usage: PaidRouteUsage::default(),
                payment: PaidRoutePaymentState::default(),
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: Some(PaidRouteQualityMetrics {
                    down_bps: Some(10_000),
                    ..PaidRouteQualityMetrics::default()
                }),
            },
            100
        ));

        let result = store
            .update_session_probe(UpdatePaidRouteSessionProbeRequest {
                session_id: " session-1 ".to_string(),
                realized_exit_ip: Some(" 198.51.100.42 ".to_string()),
                observed_country_code: Some(" fi ".to_string()),
                observed_asn: Some(14_593),
                quality: Some(PaidRouteQualityMetrics {
                    latency_ms: Some(42),
                    jitter_ms: Some(7),
                    ..PaidRouteQualityMetrics::default()
                }),
                now_unix: 123,
            })
            .expect("update probe");

        assert!(result.changed);
        assert_eq!(result.realized_exit_ip.as_deref(), Some("198.51.100.42"));
        assert_eq!(result.observed_country_code.as_deref(), Some("FI"));
        assert_eq!(result.observed_asn, Some(14_593));
        let quality = result.quality.expect("quality");
        assert_eq!(quality.latency_ms, Some(42));
        assert_eq!(quality.jitter_ms, Some(7));
        assert_eq!(quality.down_bps, Some(10_000));
        assert_eq!(quality.last_seen_unix, Some(123));
        assert_eq!(store.sessions["session-1"].updated_at_unix, 123);

        let unchanged = store
            .update_session_probe(UpdatePaidRouteSessionProbeRequest {
                session_id: "session-1".to_string(),
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: None,
                now_unix: 124,
            })
            .expect("empty update");
        assert!(!unchanged.changed);
        assert_eq!(store.sessions["session-1"].updated_at_unix, 123);
    }

    #[test]
    fn paid_route_store_opens_buyer_probe_session_from_offer() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let signed_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            100,
        )
        .expect("signed offer");
        let offer = signed_offer.offer().expect("offer");
        let offer_key = paid_route_offer_store_key(&offer.seller_npub, &offer.offer_id);
        let mut store = PaidRouteStore::default();
        store.upsert_wallet_mint("https://mint.minibits.cash/Bitcoin", "Minibits", None, 99);
        store
            .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 101)
            .expect("store offer");

        let result = store
            .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
                offer_selector: "internet-exit".to_string(),
                buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
                mint_url: None,
                channel_capacity_sat: Some(50),
                initial_paid_msat: 0,
                now_unix: 120,
            })
            .expect("open buyer session");

        assert!(result.changed);
        assert_eq!(result.offer_key, offer_key);
        assert_eq!(result.offer_id, "internet-exit");
        assert_eq!(result.seller_npub, offer.seller_npub);
        assert_eq!(result.mint_url, "https://mint.minibits.cash/Bitcoin");
        assert_eq!(result.channel_capacity_sat, 50);
        assert_eq!(result.expires_at_unix, 720);
        assert_eq!(store.wallet.mints[0].label, "Minibits");
        assert_eq!(
            store.quotes[&result.quote_id].quote.receiver_pubkey_hex,
            seller.public_key().to_hex()
        );
        assert_eq!(
            store.leases[&result.lease_id].status,
            PaidRouteLifecycleStatus::Probing
        );
        assert_eq!(
            store.channels[&result.channel_id].status,
            PaidRouteLifecycleStatus::Probing
        );
        assert_eq!(
            store.sessions[&result.session_id]
                .session
                .payment
                .channel_id,
            result.channel_id
        );
        assert_eq!(
            store
                .buyer_session_seller_npub(&result.session_id)
                .expect("resolve seller"),
            offer.seller_npub
        );
    }

    #[test]
    fn paid_route_store_uses_offer_spilman_receiver_pubkey_for_buyer_quote() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let receiver_pubkey_hex = format!("03{}", "22".repeat(32));
        let signed_offer = signed_paid_exit_offer_from_config_with_receiver(
            "internet-exit",
            &seller,
            &sample_config(),
            Some(&receiver_pubkey_hex),
            None,
            100,
        )
        .expect("signed offer");
        let mut store = PaidRouteStore::default();
        store.upsert_wallet_mint("https://mint.minibits.cash/Bitcoin", "Minibits", None, 99);
        store
            .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 101)
            .expect("store offer");

        let result = store
            .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
                offer_selector: "internet-exit".to_string(),
                buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
                mint_url: None,
                channel_capacity_sat: Some(50),
                initial_paid_msat: 0,
                now_unix: 120,
            })
            .expect("open buyer session");

        assert_eq!(
            store.quotes[&result.quote_id].quote.receiver_pubkey_hex,
            receiver_pubkey_hex
        );
    }

    #[test]
    fn buyer_session_seller_npub_rejects_seller_sessions() {
        let mut store = PaidRouteStore::default();
        assert!(store.upsert_channel(PaidRouteChannelRecord {
            channel_id: "channel-1".to_string(),
            offer_id: "internet-exit".to_string(),
            role: PaidRouteChannelRole::Seller,
            status: PaidRouteLifecycleStatus::Active,
            payment: PaidRoutePaymentState {
                channel_id: "channel-1".to_string(),
                ..PaidRoutePaymentState::default()
            },
            mint_url: "https://mint.example".to_string(),
            counterparty_npub:
                "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqfu2a5w".to_string(),
            created_at_unix: 100,
            expires_at_unix: 700,
            updated_at_unix: 100,
            error: String::new(),
        }));
        assert!(store.upsert_session(
            PaidRouteSession {
                session_id: "session-1".to_string(),
                lease_id: "lease-1".to_string(),
                usage: PaidRouteUsage::default(),
                payment: PaidRoutePaymentState {
                    channel_id: "channel-1".to_string(),
                    ..PaidRoutePaymentState::default()
                },
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: None,
            },
            100
        ));

        let error = store
            .buyer_session_seller_npub("session-1")
            .expect_err("reject seller channel");

        assert!(error.to_string().contains("not a buyer session"));
    }

    #[test]
    fn attach_buyer_spilman_channel_replaces_placeholder_channel_id() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, placeholder_channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);
        let real_channel_id = "spilman-real-channel-1";

        let result = store
            .attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
                session_id: session_id.clone(),
                channel_id: real_channel_id.to_string(),
                cashu_unit: "sat".to_string(),
                capacity_sat: 10,
                paid_msat: Some(1_000),
                payment: sample_spilman_payment(real_channel_id, 1),
                now_unix: 130,
            })
            .expect("attach real channel");

        assert!(result.changed);
        assert_eq!(result.previous_channel_id, placeholder_channel_id);
        assert!(!store.channels.contains_key(&placeholder_channel_id));
        assert_eq!(
            store.channels[real_channel_id].status,
            PaidRouteLifecycleStatus::Active
        );
        assert_eq!(
            store.sessions[&session_id].session.payment.channel_id,
            real_channel_id
        );
        assert_eq!(store.sessions[&session_id].session.payment.paid_msat, 1_000);
        assert_eq!(
            store.sessions[&session_id]
                .session
                .payment
                .cashu_spilman_payment
                .as_ref()
                .map(|payment| payment.channel_id.as_str()),
            Some(real_channel_id)
        );
    }

    #[test]
    fn attach_buyer_spilman_channel_rejects_overclaimed_payment_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, placeholder_channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);
        let before = store.clone();

        let error = store
            .attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
                session_id: session_id.clone(),
                channel_id: "spilman-real-channel-1".to_string(),
                cashu_unit: "sat".to_string(),
                capacity_sat: 10,
                paid_msat: Some(2_000),
                payment: sample_spilman_payment("spilman-real-channel-1", 1),
                now_unix: 130,
            })
            .expect_err("overclaimed payment should fail");

        assert!(
            error
                .to_string()
                .contains("does not match Cashu Spilman balance")
        );
        assert_eq!(store, before);
        assert!(store.channels.contains_key(&placeholder_channel_id));
    }

    #[test]
    fn seller_admissions_reflect_streaming_payment_decision() {
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = PaidRouteStore::default();
        assert!(store.upsert_lease(
            PaidRouteLease {
                lease_id: "lease-1".to_string(),
                offer_id: "internet-exit".to_string(),
                quote_id: "quote-1".to_string(),
                buyer_npub: buyer_npub.clone(),
                starts_at_unix: 100,
                expires_at_unix: 200,
            },
            PaidRouteLifecycleStatus::Active,
            100,
        ));
        assert!(store.upsert_channel(PaidRouteChannelRecord {
            channel_id: "channel-1".to_string(),
            offer_id: "internet-exit".to_string(),
            role: PaidRouteChannelRole::Seller,
            status: PaidRouteLifecycleStatus::Active,
            payment: PaidRoutePaymentState {
                mode: PaidRoutePaymentMode::CashuSpilman,
                channel_id: "channel-1".to_string(),
                capacity_sat: 10,
                paid_msat: 1_000,
                updated_at_unix: 100,
                ..PaidRoutePaymentState::default()
            },
            mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
            counterparty_npub: buyer_npub.clone(),
            created_at_unix: 100,
            expires_at_unix: 200,
            updated_at_unix: 100,
            error: String::new(),
        }));
        assert!(store.upsert_session(
            PaidRouteSession {
                session_id: "session-1".to_string(),
                lease_id: "lease-1".to_string(),
                usage: PaidRouteUsage {
                    rx_bytes: 100,
                    billable_bytes: 100,
                    ..PaidRouteUsage::default()
                },
                payment: PaidRoutePaymentState {
                    mode: PaidRoutePaymentMode::CashuSpilman,
                    channel_id: "channel-1".to_string(),
                    capacity_sat: 10,
                    paid_msat: 1_000,
                    updated_at_unix: 100,
                    ..PaidRoutePaymentState::default()
                },
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: None,
            },
            100,
        ));

        let admissions = store.seller_admissions(&config, 150);

        assert_eq!(admissions.len(), 1);
        assert_eq!(admissions[0].buyer_pubkey, buyer.public_key().to_hex());
        assert_eq!(admissions[0].buyer_npub, buyer_npub);
        assert_eq!(admissions[0].state, PaidRouteAccessState::Paid);
        assert!(admissions[0].allow_routing);
        assert_eq!(admissions[0].amount_due_msat, 1_000);
        assert_eq!(admissions[0].unpaid_msat, 0);

        {
            let record = store.sessions.get_mut("session-1").expect("session");
            record.session.usage.rx_bytes = 200;
            record.session.usage.billable_bytes = 200;
            record.updated_at_unix = 151;
        }
        let admissions = store.seller_admissions(&config, 150);

        assert_eq!(admissions[0].state, PaidRouteAccessState::Suspended);
        assert!(!admissions[0].allow_routing);
        assert_eq!(admissions[0].amount_due_msat, 2_000);
        assert_eq!(admissions[0].unpaid_msat, 1_000);

        let admissions = store.seller_admissions(&config, 201);

        assert!(!admissions[0].allow_routing);
        assert_eq!(admissions[0].state, PaidRouteAccessState::Suspended);
    }

    #[test]
    fn seller_collection_states_mark_expired_spilman_credit_due() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    129,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 129,
            })
            .expect("apply paid balance");

        let current = store.seller_collection_states(&config, 499);

        assert_eq!(current.len(), 1);
        assert!(current[0].collectable);
        assert!(current[0].manual_collect);
        assert!(!current[0].auto_collect_due);
        assert_eq!(current[0].reason, "manual");
        assert_eq!(current[0].paid_msat, 1_000);
        assert_eq!(current[0].due_at_unix, 500);

        let due = store.seller_collection_states(&config, 500);

        assert_eq!(due.len(), 1);
        assert!(due[0].collectable);
        assert!(due[0].manual_collect);
        assert!(due[0].auto_collect_due);
        assert_eq!(due[0].reason, "expired");
        assert_eq!(due[0].channel_id, "channel-1");
        assert_eq!(due[0].session_id, "seller-session-lease-1");
    }

    #[test]
    fn record_seller_usage_updates_session_and_admission_decision() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    129,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 0,
                        amount_due_msat: 0,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 129,
            })
            .expect("apply paid balance");
        let result = store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    rx_bytes: 60,
                    rx_packets: 1,
                    billable_bytes: 60,
                    billable_packets: 1,
                    ..PaidRouteUsage::default()
                },
                now_unix: 130,
            })
            .expect("record usage")
            .expect("matched seller session");

        assert!(result.changed);
        assert_eq!(result.session_id, "seller-session-lease-1");
        assert_eq!(result.usage.rx_bytes, 60);
        assert_eq!(result.usage.rx_packets, 1);
        assert_eq!(result.amount_due_msat, 600);
        assert_eq!(result.unpaid_msat, 0);
        assert!(result.allow_routing);
        assert_eq!(
            store.seller_admissions(&config, 130)[0].state,
            PaidRouteAccessState::Paid
        );

        let result = store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    tx_bytes: 50,
                    tx_packets: 1,
                    billable_bytes: 50,
                    billable_packets: 1,
                    ..PaidRouteUsage::default()
                },
                now_unix: 131,
            })
            .expect("record usage")
            .expect("matched seller session");

        assert_eq!(result.usage.rx_bytes, 60);
        assert_eq!(result.usage.tx_bytes, 50);
        assert_eq!(result.amount_due_msat, 1_100);
        assert_eq!(result.unpaid_msat, 100);
        assert!(!result.allow_routing);
        assert_eq!(result.state, PaidRouteAccessState::Suspended);
        assert_eq!(
            store.seller_admissions(&config, 131)[0].state,
            PaidRouteAccessState::Suspended
        );
    }

    #[test]
    fn seller_payment_channel_open_creates_seller_session_and_admission() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 100;
        config.channel.grace_units = 0;

        let mut store = PaidRouteStore::default();
        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    100,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 500,
                        receiver_pubkey_hex: seller.public_key().to_hex(),
                        paid_msat: 0,
                        payment: sample_spilman_payment("channel-1", 0),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 100,
            })
            .expect("apply channel open");

        assert!(result.changed);
        assert_eq!(result.payload_type, "channel_open");
        assert_eq!(result.session_id, "seller-session-lease-1");
        assert_eq!(result.state, PaidRouteAccessState::FreeProbe);
        assert!(result.allow_routing);
        assert_eq!(
            store.quotes["seller-quote-lease-1"].quote.offer_id,
            "internet-exit"
        );
        assert_eq!(
            store.leases["lease-1"].lease.buyer_npub,
            buyer.public_key().to_bech32().expect("buyer npub")
        );
        assert_eq!(
            store.channels["channel-1"].role,
            PaidRouteChannelRole::Seller
        );
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .payment
                .capacity_sat,
            10
        );

        let admissions = store.seller_admissions(&config, 101);
        assert_eq!(admissions.len(), 1);
        assert_eq!(admissions[0].buyer_pubkey, buyer.public_key().to_hex());
        assert!(admissions[0].allow_routing);
        assert_eq!(admissions[0].state, PaidRouteAccessState::FreeProbe);
    }

    #[test]
    fn seller_payment_channel_open_rejects_reused_lease_with_new_channel() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let config = sample_config();
        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        let before = store.clone();

        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    110,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 500,
                        receiver_pubkey_hex: seller.public_key().to_hex(),
                        paid_msat: 0,
                        payment: sample_spilman_payment("channel-2", 0),
                    }),
                ),
                seller_npub,
                config,
                now_unix: 110,
            })
            .expect_err("lease id must not be rebound");

        assert!(error.to_string().contains("already bound to channel"));
        assert_eq!(store, before);
    }

    #[test]
    fn seller_payment_channel_open_requires_spilman_funding_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 100;
        config.channel.grace_units = 0;
        let mut payment = sample_spilman_payment("channel-1", 0);
        payment.params = None;
        payment.funding_proofs = None;

        let mut store = PaidRouteStore::default();
        let before = store.clone();
        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    100,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 500,
                        receiver_pubkey_hex: seller.public_key().to_hex(),
                        paid_msat: 0,
                        payment,
                    }),
                ),
                seller_npub,
                config,
                now_unix: 100,
            })
            .expect_err("missing Spilman funding should fail");

        assert!(error.to_string().contains("missing funding"));
        assert_eq!(store, before);
    }

    #[test]
    fn seller_payment_with_spilman_receiver_validates_and_applies_channel_open() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 100;
        config.channel.grace_units = 0;
        let mut store = PaidRouteStore::default();
        let receiver = FakeSpilmanReceiver::new("channel-1", 0);

        let result = store
            .apply_seller_payment_with_spilman_receiver(
                ApplyPaidRouteSellerPaymentRequest {
                    envelope: seller_payment_envelope(
                        "internet-exit",
                        "lease-1",
                        &buyer_npub,
                        &seller_npub,
                        100,
                        StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                            mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                            unit: "sat".to_string(),
                            capacity: 10,
                            expires_unix: 500,
                            receiver_pubkey_hex: seller.public_key().to_hex(),
                            paid_msat: 0,
                            payment: sample_spilman_payment("channel-1", 0),
                        }),
                    ),
                    seller_npub,
                    config: config.clone(),
                    now_unix: 100,
                },
                &receiver,
                &(),
            )
            .expect("apply receiver-validated channel open");

        assert!(result.changed);
        assert_eq!(result.payload_type, "channel_open");
        assert_eq!(result.state, PaidRouteAccessState::FreeProbe);
        assert_eq!(
            store.channels["channel-1"].payment.cashu_spilman_payment,
            Some(sample_spilman_payment("channel-1", 0))
        );
        assert_eq!(receiver.validate_calls.get(), 0);
        assert_eq!(receiver.process_calls.get(), 1);
    }

    #[test]
    fn seller_payment_with_spilman_receiver_rejects_receiver_mismatch_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 100;
        config.channel.grace_units = 0;
        let mut store = PaidRouteStore::default();
        let before = store.clone();
        let receiver = FakeSpilmanReceiver::new("channel-1", 1);

        let error = store
            .apply_seller_payment_with_spilman_receiver(
                ApplyPaidRouteSellerPaymentRequest {
                    envelope: seller_payment_envelope(
                        "internet-exit",
                        "lease-1",
                        &buyer_npub,
                        &seller_npub,
                        100,
                        StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                            mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                            unit: "sat".to_string(),
                            capacity: 10,
                            expires_unix: 500,
                            receiver_pubkey_hex: seller.public_key().to_hex(),
                            paid_msat: 0,
                            payment: sample_spilman_payment("channel-1", 0),
                        }),
                    ),
                    seller_npub,
                    config,
                    now_unix: 100,
                },
                &receiver,
                &(),
            )
            .expect_err("receiver mismatch should fail");

        assert!(error.to_string().contains("receiver validated balance"));
        assert_eq!(store, before);
        assert_eq!(receiver.validate_calls.get(), 0);
        assert_eq!(receiver.process_calls.get(), 1);
    }

    #[test]
    fn seller_payment_with_spilman_receiver_accepts_lagging_due_as_partial_credit() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let mut store = PaidRouteStore::default();
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    100,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 500,
                        receiver_pubkey_hex: seller.public_key().to_hex(),
                        paid_msat: 0,
                        payment: sample_spilman_payment("channel-1", 0),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 100,
            })
            .expect("seed seller channel");
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    billable_bytes: 200,
                    ..PaidRouteUsage::default()
                },
                now_unix: 100,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");
        let receiver = FakeSpilmanReceiver::new("channel-1", 2);

        let result = store
            .apply_seller_payment_with_spilman_receiver(
                ApplyPaidRouteSellerPaymentRequest {
                    envelope: seller_payment_envelope(
                        "internet-exit",
                        "lease-1",
                        &buyer_npub,
                        &seller_npub,
                        101,
                        StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                            delivered_units: 200,
                            amount_due_msat: 1_000,
                            paid_msat: 2_000,
                            payment: sample_spilman_payment("channel-1", 2),
                        }),
                    ),
                    seller_npub,
                    config,
                    now_unix: 101,
                },
                &receiver,
                &(),
            )
            .expect("lagging reported due is accepted as partial credit");

        assert_eq!(result.amount_due_msat, 2_000);
        assert_eq!(result.paid_msat, 2_000);
        assert_eq!(result.unpaid_msat, 0);
        assert!(result.allow_routing);
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .usage
                .billable_bytes,
            200
        );
        assert_eq!(receiver.validate_calls.get(), 0);
        assert_eq!(receiver.process_calls.get(), 1);
    }

    #[test]
    fn buyer_payment_envelope_channel_open_persists_spilman_snapshot() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        let result = store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub,
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen,
                payment: sample_spilman_payment(&channel_id, 0),
                delivered_units: None,
                paid_msat: Some(0),
                now_unix: 130,
            })
            .expect("build channel open envelope");

        assert!(result.changed);
        assert_eq!(result.payload_type, "channel_open");
        assert_eq!(result.offer_id, "internet-exit");
        assert_eq!(result.delivered_units, 0);
        assert_eq!(result.paid_msat, 0);
        match result.envelope.payload {
            StreamingRoutePaymentPayload::ChannelOpen(open) => {
                assert_eq!(open.mint_url, "https://mint.minibits.cash/Bitcoin");
                assert_eq!(open.unit, "sat");
                assert_eq!(open.capacity, 10);
                assert_eq!(open.receiver_pubkey_hex, seller.public_key().to_hex());
                assert!(open.payment.has_funding());
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        assert!(
            store.sessions[&session_id]
                .session
                .payment
                .cashu_spilman_payment
                .as_ref()
                .is_some_and(CashuSpilmanPayment::has_funding)
        );
    }

    #[test]
    fn buyer_payment_envelope_balance_update_advances_usage_and_paid_amount() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        let result = store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub: buyer_npub.clone(),
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                payment: sample_spilman_payment(&channel_id, 1),
                delivered_units: Some(100),
                paid_msat: Some(1_000),
                now_unix: 140,
            })
            .expect("build balance update");

        assert!(result.changed);
        assert_eq!(result.payload_type, "balance_update");
        assert_eq!(result.state, PaidRouteAccessState::Paid);
        assert_eq!(result.delivered_units, 100);
        assert_eq!(result.amount_due_msat, 1_000);
        assert_eq!(result.unpaid_msat, 0);
        match result.envelope.payload {
            StreamingRoutePaymentPayload::BalanceUpdate(update) => {
                assert_eq!(update.delivered_units, 100);
                assert_eq!(update.amount_due_msat, 1_000);
                assert_eq!(update.paid_msat, 1_000);
                assert_eq!(update.payment.balance, 1);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        let record = &store.sessions[&session_id];
        assert_eq!(record.session.usage.billable_bytes, 100);
        assert_eq!(record.session.payment.paid_msat, 1_000);
        assert_eq!(
            record
                .session
                .payment
                .cashu_spilman_payment
                .as_ref()
                .map(|payment| payment.balance),
            Some(1)
        );

        let error = store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id,
                buyer_npub,
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                payment: sample_spilman_payment(&channel_id, 0),
                delivered_units: Some(50),
                paid_msat: Some(500),
                now_unix: 141,
            })
            .expect_err("regressing buyer update rejected");
        assert!(error.to_string().contains("regressed"));
    }

    #[test]
    fn buyer_payment_envelope_rejects_overclaimed_spilman_balance_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);
        let before = store.clone();

        let error = store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id,
                buyer_npub,
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                payment: sample_spilman_payment(&channel_id, 1),
                delivered_units: Some(100),
                paid_msat: Some(2_000),
                now_unix: 140,
            })
            .expect_err("overclaimed payment should fail");

        assert!(
            error
                .to_string()
                .contains("does not match Cashu Spilman balance")
        );
        assert_eq!(store, before);
    }

    #[test]
    fn cashu_token_lease_fallback_prepays_buyer_but_seller_requires_redemption() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut buyer_store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);
        buyer_store
            .sessions
            .get_mut(&session_id)
            .expect("buyer session")
            .session
            .usage
            .billable_bytes = 100;

        let buyer_payment = buyer_store
            .build_buyer_token_lease_envelope(BuildPaidRouteBuyerTokenLeaseEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub: buyer_npub.clone(),
                mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                cashu_unit: "sat".to_string(),
                amount: 2,
                paid_msat: Some(1_500),
                token: "cashuBdevtoken".to_string(),
                expires_at_unix: Some(500),
                now_unix: 140,
            })
            .expect("build token lease");

        assert!(buyer_payment.changed);
        assert_eq!(buyer_payment.payload_type, "cashu_token_lease");
        assert_eq!(buyer_payment.state, PaidRouteAccessState::Paid);
        assert_eq!(buyer_payment.amount_due_msat, 1_000);
        assert_eq!(buyer_payment.paid_msat, 1_500);
        assert_eq!(buyer_payment.channel_id, channel_id);
        let buyer_payment_state = &buyer_store.sessions[&session_id].session.payment;
        assert_eq!(
            buyer_payment_state.mode,
            PaidRoutePaymentMode::CashuTokenLease
        );
        assert!(buyer_payment_state.cashu_spilman_payment.is_none());
        assert!(
            buyer_payment_state
                .cashu_token_lease
                .as_ref()
                .is_some_and(|lease| lease.token == "cashuBdevtoken")
        );
        match &buyer_payment.envelope.payload {
            StreamingRoutePaymentPayload::CashuTokenLease(lease) => {
                assert_eq!(lease.amount, 2);
                assert_eq!(lease.paid_msat, 1_500);
                assert_eq!(lease.expires_unix, 500);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let mut seller_store = PaidRouteStore::default();
        let before = seller_store.clone();
        let error = seller_store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: buyer_payment.envelope.clone(),
                seller_npub,
                config: config.clone(),
                now_unix: 141,
            })
            .expect_err("seller must redeem token leases before admitting routing");

        assert!(error.to_string().contains("token redemption"));
        assert_eq!(seller_store, before);
    }

    #[test]
    fn cashu_token_lease_fallback_rejects_credit_above_token_amount() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let config = sample_config();
        let (mut store, session_id, _) = buyer_store_with_session(&seller, &buyer, &config);

        let error = store
            .build_buyer_token_lease_envelope(BuildPaidRouteBuyerTokenLeaseEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub,
                mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                cashu_unit: "sat".to_string(),
                amount: 1,
                paid_msat: Some(1_001),
                token: "cashuBdevtoken".to_string(),
                expires_at_unix: Some(500),
                now_unix: 140,
            })
            .expect_err("over-credit should fail");

        assert!(error.to_string().contains("exceeds token amount"));
        let payment = &store.sessions[&session_id].session.payment;
        assert_eq!(payment.mode, PaidRoutePaymentMode::CashuSpilman);
        assert!(payment.cashu_token_lease.is_none());
    }

    #[test]
    fn record_buyer_usage_updates_session_for_exit_seller() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub,
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                payment: sample_spilman_payment(&channel_id, 1),
                delivered_units: Some(0),
                paid_msat: Some(1_000),
                now_unix: 130,
            })
            .expect("apply paid balance");

        let result = store
            .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
                seller_pubkey: seller.public_key().to_hex(),
                usage_delta: PaidRouteUsage {
                    rx_bytes: 60,
                    rx_packets: 1,
                    billable_bytes: 60,
                    billable_packets: 1,
                    ..PaidRouteUsage::default()
                },
                now_unix: 131,
            })
            .expect("record buyer usage")
            .expect("matched buyer session");

        assert!(result.changed);
        assert_eq!(result.session_id, session_id);
        assert_eq!(result.usage.rx_bytes, 60);
        assert_eq!(result.amount_due_msat, 600);
        assert_eq!(result.unpaid_msat, 0);
        assert!(result.allow_routing);
        assert_eq!(result.state, PaidRouteAccessState::Paid);

        let result = store
            .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
                seller_pubkey: seller.public_key().to_hex(),
                usage_delta: PaidRouteUsage {
                    tx_bytes: 50,
                    tx_packets: 1,
                    billable_bytes: 50,
                    billable_packets: 1,
                    ..PaidRouteUsage::default()
                },
                now_unix: 132,
            })
            .expect("record buyer usage")
            .expect("matched buyer session");

        assert_eq!(result.usage.rx_bytes, 60);
        assert_eq!(result.usage.tx_bytes, 50);
        assert_eq!(result.amount_due_msat, 1_100);
        assert_eq!(result.unpaid_msat, 100);
        assert!(!result.allow_routing);
        assert_eq!(result.state, PaidRouteAccessState::Suspended);
    }

    #[test]
    fn buyer_payment_updates_due_reports_signable_balance_updates() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        store
            .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id: session_id.clone(),
                buyer_npub: buyer_npub.clone(),
                kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                payment: sample_spilman_payment(&channel_id, 1),
                delivered_units: Some(0),
                paid_msat: Some(1_000),
                now_unix: 130,
            })
            .expect("apply paid balance");
        store
            .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
                seller_pubkey: seller.public_key().to_hex(),
                usage_delta: PaidRouteUsage {
                    rx_bytes: 60,
                    tx_bytes: 50,
                    billable_bytes: 110,
                    ..PaidRouteUsage::default()
                },
                now_unix: 131,
            })
            .expect("record buyer usage")
            .expect("matched buyer session");

        let due = store.buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
            now_unix: 132,
            min_increment_msat: 1,
        });

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].session_id, session_id);
        assert_eq!(due[0].channel_id, channel_id);
        assert_eq!(due[0].delivered_units, 110);
        assert_eq!(due[0].amount_due_msat, 1_100);
        assert_eq!(due[0].paid_msat, 1_000);
        assert_eq!(due[0].target_paid_msat, 2_000);
        assert_eq!(due[0].payment_increment_msat, 1_000);
        assert_eq!(due[0].remaining_unpaid_msat, 0);
        assert!(!due[0].capacity_exhausted);

        let signed = store
            .build_buyer_signed_payment_envelope_for_due(
                &FakePaymentSigner,
                &buyer_npub,
                &due[0],
                133,
            )
            .expect("sign due update");

        assert_eq!(signed.due, due[0]);
        assert_eq!(signed.payment.paid_msat, 2_000);
        store = signed.store;
        assert!(
            store
                .buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
                    now_unix: 134,
                    min_increment_msat: 1,
                })
                .is_empty()
        );
    }

    #[test]
    fn buyer_payment_updates_due_uses_connection_minimum_floor() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 0;
        config.pricing.connection_minimum_msat_per_day = 86_400;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        store
            .attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
                session_id: session_id.clone(),
                channel_id: channel_id.clone(),
                cashu_unit: "sat".to_string(),
                capacity_sat: 10,
                paid_msat: Some(0),
                payment: sample_spilman_payment(&channel_id, 0),
                now_unix: 130,
            })
            .expect("attach channel");
        store
            .sessions
            .get_mut(&session_id)
            .expect("session")
            .session
            .usage
            .active_millis = 1_000;

        let due = store.buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
            now_unix: 131,
            min_increment_msat: 1,
        });

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].delivered_units, 0);
        assert_eq!(due[0].amount_due_msat, 1);
        assert_eq!(due[0].target_paid_msat, 1_000);
        assert_eq!(due[0].payment_increment_msat, 1_000);
    }

    #[test]
    fn buyer_payment_updates_due_caps_at_channel_capacity() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);
        store
            .sessions
            .get_mut(&session_id)
            .unwrap()
            .session
            .payment
            .capacity_sat = 1;
        store
            .channels
            .get_mut(&channel_id)
            .unwrap()
            .payment
            .capacity_sat = 1;
        store.sessions.get_mut(&session_id).unwrap().session.usage = PaidRouteUsage {
            rx_bytes: 250,
            billable_bytes: 250,
            ..PaidRouteUsage::default()
        };

        let due = store.buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
            now_unix: 132,
            min_increment_msat: 1,
        });

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].amount_due_msat, 2_500);
        assert_eq!(due[0].target_paid_msat, 1_000);
        assert_eq!(due[0].capacity_msat, 1_000);
        assert_eq!(due[0].remaining_unpaid_msat, 1_500);
        assert!(due[0].capacity_exhausted);
    }

    #[test]
    fn buyer_signed_payment_envelope_uses_cashu_service_signer() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;
        let (mut store, session_id, channel_id) =
            buyer_store_with_session(&seller, &buyer, &config);

        let result = store
            .build_buyer_signed_payment_envelope(
                &FakePaymentSigner,
                BuildPaidRouteBuyerSignedPaymentEnvelopeRequest {
                    session_id: session_id.clone(),
                    buyer_npub,
                    kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
                    delivered_units: Some(100),
                    paid_msat: None,
                    now_unix: 150,
                },
            )
            .expect("build signed payment envelope");

        assert!(result.changed);
        assert_eq!(result.amount_due_msat, 1);
        assert_eq!(result.paid_msat, 1_000);
        assert_eq!(result.state, PaidRouteAccessState::Paid);
        match result.envelope.payload {
            StreamingRoutePaymentPayload::BalanceUpdate(update) => {
                assert_eq!(update.paid_msat, 1_000);
                assert_eq!(update.payment.channel_id, channel_id);
                assert_eq!(update.payment.balance, 1);
                assert_eq!(
                    update.payment.signature,
                    format!("signed-{channel_id}-update")
                );
                assert!(!update.payment.has_funding());
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        assert_eq!(store.sessions[&session_id].session.payment.paid_msat, 1_000);
    }

    #[test]
    fn seller_payment_balance_update_raises_paid_amount_and_usage() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    billable_bytes: 100,
                    ..PaidRouteUsage::default()
                },
                now_unix: 110,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");
        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply balance update");

        assert!(result.changed);
        assert_eq!(result.payload_type, "balance_update");
        assert_eq!(result.state, PaidRouteAccessState::Paid);
        assert!(result.allow_routing);
        assert_eq!(result.delivered_units, 100);
        assert_eq!(result.paid_msat, 1_000);
        assert_eq!(result.amount_due_msat, 1_000);
        assert_eq!(result.unpaid_msat, 0);
        assert_eq!(store.channels["channel-1"].payment.paid_msat, 1_000);
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .usage
                .billable_bytes,
            100
        );
        assert_eq!(
            store.seller_admissions(&config, 121)[0].state,
            PaidRouteAccessState::Paid
        );
    }

    #[test]
    fn seller_payment_balance_update_does_not_import_buyer_overreported_units() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    billable_bytes: 100,
                    ..PaidRouteUsage::default()
                },
                now_unix: 110,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");

        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 200,
                        amount_due_msat: 2_000,
                        paid_msat: 2_000,
                        payment: sample_spilman_payment("channel-1", 2),
                    }),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply overreported balance update");

        assert_eq!(result.delivered_units, 100);
        assert_eq!(result.amount_due_msat, 1_000);
        assert_eq!(result.paid_msat, 2_000);
        assert_eq!(result.unpaid_msat, 0);
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .usage
                .billable_bytes,
            100
        );
    }

    #[test]
    fn seller_payment_balance_update_accepts_lagging_buyer_usage_counter() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    rx_bytes: 150,
                    billable_bytes: 150,
                    ..PaidRouteUsage::default()
                },
                now_unix: 110,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");

        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_500,
                        paid_msat: 2_000,
                        payment: sample_spilman_payment("channel-1", 2),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply lagging balance update");

        assert_eq!(result.delivered_units, 150);
        assert_eq!(result.amount_due_msat, 1_500);
        assert_eq!(result.paid_msat, 2_000);
        assert_eq!(result.unpaid_msat, 0);
        assert_eq!(result.state, PaidRouteAccessState::Paid);
        assert!(result.allow_routing);
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .usage
                .rx_bytes,
            150
        );
    }

    #[test]
    fn seller_payment_balance_update_tolerates_connection_minimum_flush_skew() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 0;
        config.pricing.connection_minimum_msat_per_day = 86_400;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    active_millis: 3_000,
                    ..PaidRouteUsage::default()
                },
                now_unix: 110,
            })
            .expect("record seller-observed active time")
            .expect("matched seller session");

        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 0,
                        amount_due_msat: 1,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply skew-tolerated balance update");

        assert_eq!(result.amount_due_msat, 3);
        assert_eq!(result.paid_msat, 1_000);
        assert!(result.allow_routing);
    }

    #[test]
    fn seller_payment_balance_update_accepts_underreported_due_without_importing_usage() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.pricing.connection_minimum_msat_per_day = 86_400;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    active_millis: 3_000,
                    billable_bytes: 150,
                    ..PaidRouteUsage::default()
                },
                now_unix: 110,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");

        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 0,
                        amount_due_msat: 1,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub,
                config,
                now_unix: 120,
            })
            .expect("lagging traffic report is accepted as partial credit");

        assert_eq!(result.amount_due_msat, 1_500);
        assert_eq!(result.paid_msat, 1_000);
        assert_eq!(result.unpaid_msat, 500);
        assert_eq!(result.state, PaidRouteAccessState::Suspended);
        assert!(!result.allow_routing);
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .usage
                .billable_bytes,
            150
        );
    }

    #[test]
    fn seller_payment_rejects_regressing_balance_update_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply first update");
        let before = store.clone();

        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    121,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 0,
                        payment: sample_spilman_payment("channel-1", 0),
                    }),
                ),
                seller_npub,
                config,
                now_unix: 121,
            })
            .expect_err("regressing update rejected");

        assert!(error.to_string().contains("regressed"));
        assert_eq!(store, before);
    }

    #[test]
    fn seller_payment_rejects_overclaimed_spilman_balance_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        let before = store.clone();
        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 2_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub,
                config,
                now_unix: 120,
            })
            .expect_err("overclaimed balance update should fail");

        assert!(error.to_string().contains("does not match"));
        assert_eq!(store, before);
    }

    #[test]
    fn seller_payment_rejects_underpaid_cooperative_close_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                buyer_pubkey: buyer.public_key().to_hex(),
                config: config.clone(),
                usage_delta: PaidRouteUsage {
                    billable_bytes: 200,
                    ..PaidRouteUsage::default()
                },
                now_unix: 120,
            })
            .expect("record seller-observed usage")
            .expect("matched seller session");
        let before = store.clone();

        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    130,
                    StreamingRoutePaymentPayload::CooperativeClose(
                        StreamingRouteCooperativeClose {
                            final_paid_msat: 1_000,
                            payment: sample_spilman_payment("channel-1", 1),
                        },
                    ),
                ),
                seller_npub,
                config,
                now_unix: 130,
            })
            .expect_err("underpaid close should fail");

        assert!(error.to_string().contains("underpays amount due"));
        assert_eq!(store, before);
    }

    #[test]
    fn seller_payment_cooperative_close_suspends_admission() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    120,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 1_000,
                        payment: sample_spilman_payment("channel-1", 1),
                    }),
                ),
                seller_npub: seller_npub.clone(),
                config: config.clone(),
                now_unix: 120,
            })
            .expect("apply first update");

        let result = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    130,
                    StreamingRoutePaymentPayload::CooperativeClose(
                        StreamingRouteCooperativeClose {
                            final_paid_msat: 1_000,
                            payment: sample_spilman_payment("channel-1", 1),
                        },
                    ),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 130,
            })
            .expect("apply close");

        assert_eq!(result.payload_type, "cooperative_close");
        assert_eq!(result.state, PaidRouteAccessState::Suspended);
        assert!(!result.allow_routing);
        assert_eq!(
            store.channels["channel-1"].status,
            PaidRouteLifecycleStatus::Closing
        );
        assert_eq!(
            store.leases["lease-1"].status,
            PaidRouteLifecycleStatus::Closing
        );
        assert_eq!(
            store.seller_admissions(&config, 131)[0].allow_routing,
            false
        );
        let collection = store.seller_collection_states(&config, 131);
        assert_eq!(collection.len(), 1);
        assert!(collection[0].collectable);
        assert!(collection[0].manual_collect);

        assert!(
            store
                .mark_seller_channel_closed("channel-1", 1_000, 132)
                .expect("settled close")
        );
        assert_eq!(
            store.channels["channel-1"].status,
            PaidRouteLifecycleStatus::Closed
        );
    }

    #[test]
    fn seller_manual_channel_close_suspends_admission() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);

        let changed = store
            .mark_seller_channel_closed("channel-1", 1_000, 130)
            .expect("mark closed");

        assert!(changed);
        assert_eq!(
            store.channels["channel-1"].status,
            PaidRouteLifecycleStatus::Closed
        );
        assert_eq!(store.channels["channel-1"].payment.paid_msat, 1_000);
        assert_eq!(
            store.leases["lease-1"].status,
            PaidRouteLifecycleStatus::Closed
        );
        assert_eq!(
            store.sessions["seller-session-lease-1"]
                .session
                .payment
                .paid_msat,
            1_000
        );
        let admissions = store.seller_admissions(&config, 131);
        assert_eq!(admissions.len(), 1);
        assert!(!admissions[0].allow_routing);
        assert!(
            !store
                .mark_seller_channel_closed("channel-1", 1_000, 131)
                .expect("idempotent mark")
        );
    }

    #[test]
    fn seller_payment_rejects_overclaimed_spilman_close_without_mutating_store() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut config = sample_config();
        config.pricing.price_msat = 1_000;
        config.pricing.per_units = 100;
        config.channel.free_probe_units = 0;
        config.channel.grace_units = 0;

        let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
        let before = store.clone();
        let error = store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    130,
                    StreamingRoutePaymentPayload::CooperativeClose(
                        StreamingRouteCooperativeClose {
                            final_paid_msat: 2_000,
                            payment: sample_spilman_payment("channel-1", 1),
                        },
                    ),
                ),
                seller_npub,
                config,
                now_unix: 130,
            })
            .expect_err("overclaimed close should fail");

        assert!(error.to_string().contains("does not match"));
        assert_eq!(store, before);
    }

    #[test]
    fn paid_route_store_rejects_incompatible_buyer_mint() {
        let seller = Keys::generate();
        let buyer = Keys::generate();
        let signed_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            100,
        )
        .expect("signed offer");
        let mut store = PaidRouteStore::default();
        store
            .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 101)
            .expect("store offer");

        let error = store
            .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
                offer_selector: "internet-exit".to_string(),
                buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
                mint_url: Some("https://other-mint.example".to_string()),
                channel_capacity_sat: None,
                initial_paid_msat: 0,
                now_unix: 120,
            })
            .expect_err("incompatible mint is rejected");

        assert!(error.to_string().contains("not accepted"));
        assert!(store.sessions.is_empty());
    }

    #[test]
    fn paid_route_store_upserts_newer_offer_and_merges_relays() {
        let seller = Keys::generate();
        let old = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            100,
        )
        .expect("old offer");
        let new = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            200,
        )
        .expect("new offer");
        let mut store = PaidRouteStore::default();

        assert!(
            store
                .upsert_signed_offer(old.clone(), vec!["wss://a.example".to_string()], 101)
                .expect("old insert")
        );
        assert!(
            store
                .upsert_signed_offer(old, vec!["wss://b.example".to_string()], 102)
                .expect("same offer relay merge")
        );
        assert!(
            store
                .upsert_signed_offer(new.clone(), vec!["wss://c.example".to_string()], 201)
                .expect("newer replace")
        );

        let key =
            paid_route_offer_store_key(&new.offer().expect("offer").seller_npub, "internet-exit");
        let record = &store.offers[&key];
        assert_eq!(record.signed_offer.event.created_at.as_secs(), 200);
        assert_eq!(record.first_seen_unix, 101);
        assert_eq!(record.last_seen_unix, 201);
        assert_eq!(record.relay_urls, vec!["wss://c.example"]);
    }

    #[test]
    fn paid_route_store_persists_offer_rating_score() {
        let seller = Keys::generate();
        let old = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            100,
        )
        .expect("old offer");
        let new = signed_paid_exit_offer_from_config(
            "internet-exit",
            &seller,
            &sample_config(),
            None,
            200,
        )
        .expect("new offer");
        let seller_npub = old.offer().expect("offer").seller_npub;
        let key = paid_route_offer_store_key(&seller_npub, "internet-exit");
        let mut store = PaidRouteStore::default();

        store
            .upsert_signed_offer(old, vec!["wss://relay.example".to_string()], 101)
            .expect("store offer");
        assert!(store.upsert_offer_rating_score(&seller_npub, 80, 120));
        assert!(!store.upsert_offer_rating_score(&seller_npub, -80, 110));
        store
            .upsert_signed_offer(new, vec!["wss://relay.example".to_string()], 201)
            .expect("replace offer");

        let record = &store.offers[&key];
        assert_eq!(record.rating_score, Some(80));
        assert_eq!(record.rating_updated_at_unix, 120);

        assert!(store.upsert_offer_rating_score(&seller_npub, -120, 220));
        let record = &store.offers[&key];
        assert_eq!(record.rating_score, Some(-100));
        assert_eq!(record.rating_updated_at_unix, 220);
    }

    #[test]
    fn best_rated_offer_key_prefers_good_then_newcomer_over_degraded() {
        let good_seller = Keys::generate();
        let newcomer_seller = Keys::generate();
        let bad_seller = Keys::generate();
        let good_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &good_seller,
            &sample_config(),
            None,
            100,
        )
        .expect("good offer");
        let newcomer_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &newcomer_seller,
            &sample_config(),
            None,
            100,
        )
        .expect("newcomer offer");
        let bad_offer = signed_paid_exit_offer_from_config(
            "internet-exit",
            &bad_seller,
            &sample_config(),
            None,
            100,
        )
        .expect("bad offer");
        let good = good_offer.offer().expect("good offer record");
        let newcomer = newcomer_offer.offer().expect("newcomer offer record");
        let bad = bad_offer.offer().expect("bad offer record");
        let good_key = paid_route_offer_store_key(&good.seller_npub, &good.offer_id);
        let newcomer_key = paid_route_offer_store_key(&newcomer.seller_npub, &newcomer.offer_id);
        let bad_key = paid_route_offer_store_key(&bad.seller_npub, &bad.offer_id);
        let mut store = PaidRouteStore::default();

        store
            .upsert_signed_offer(good_offer, vec!["wss://relay.example".to_string()], 100)
            .expect("store good");
        store
            .upsert_signed_offer(newcomer_offer, vec!["wss://relay.example".to_string()], 110)
            .expect("store newcomer");
        store
            .upsert_signed_offer(bad_offer, vec!["wss://relay.example".to_string()], 120)
            .expect("store bad");
        assert!(store.upsert_offer_rating_score(&good.seller_npub, 80, 130));
        assert!(store.upsert_offer_rating_score(&bad.seller_npub, -80, 130));

        assert_eq!(
            store.best_rated_offer_key().expect("best rated offer"),
            good_key
        );

        assert!(store.upsert_offer_rating_score(&good.seller_npub, -90, 140));
        assert_eq!(
            store.best_rated_offer_key().expect("newcomer before bad"),
            newcomer_key
        );

        assert!(store.upsert_offer_rating_score(&newcomer.seller_npub, -10, 150));
        assert_eq!(
            store.best_rated_offer_key().expect("least bad offer"),
            newcomer_key
        );
        assert_ne!(
            store.best_rated_offer_key().expect("not worse bad offer"),
            bad_key
        );
    }

    #[test]
    fn unreadable_paid_route_store_is_discarded() {
        let scratch = ScratchDir::new("unreadable");
        let store_path = scratch.path().join("paid-routes.json");
        fs::write(&store_path, "not json").expect("write junk");

        let store = load_paid_route_store(&store_path).expect("load default");

        assert_eq!(store, PaidRouteStore::default());
    }

    fn sample_config() -> PaidExitConfig {
        PaidExitConfig {
            enabled: true,
            access: PaidRouteAccessPolicy {
                upstream: crate::paid_routes::PaidExitUpstream::HostDefault,
                private_vpn_access: PaidRoutePrivateVpnAccess::Denied,
            },
            pricing: PaidRoutePricing {
                meter: PaidRouteMeter::Bytes,
                price_msat: 2500,
                per_units: 1_000_000,
                connection_minimum_msat_per_day: 0,
            },
            channel: PaidRouteChannelTerms {
                accepted_mints: vec!["https://mint.minibits.cash/Bitcoin".to_string()],
                max_channel_capacity_sat: 100,
                channel_expiry_secs: 600,
                free_probe_units: 1_048_576,
                grace_units: 262_144,
            },
            location: PaidRouteLocationHint::default(),
            ip_support: PaidRouteIpSupport::default(),
            rating_discovery: Default::default(),
        }
    }

    fn seller_store_with_open_channel(
        seller: &Keys,
        buyer: &Keys,
        config: &PaidExitConfig,
    ) -> PaidRouteStore {
        let seller_npub = seller.public_key().to_bech32().expect("seller npub");
        let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
        let mut store = PaidRouteStore::default();
        store
            .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
                envelope: seller_payment_envelope(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    100,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.minibits.cash/Bitcoin".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 500,
                        receiver_pubkey_hex: seller.public_key().to_hex(),
                        paid_msat: 0,
                        payment: sample_spilman_payment("channel-1", 0),
                    }),
                ),
                seller_npub,
                config: config.clone(),
                now_unix: 100,
            })
            .expect("apply open");
        store
    }

    fn buyer_store_with_session(
        seller: &Keys,
        buyer: &Keys,
        config: &PaidExitConfig,
    ) -> (PaidRouteStore, String, String) {
        let signed_offer =
            signed_paid_exit_offer_from_config("internet-exit", seller, config, None, 100)
                .expect("signed offer");
        let mut store = PaidRouteStore::default();
        store.upsert_wallet_mint("https://mint.minibits.cash/Bitcoin", "Minibits", None, 99);
        store
            .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 101)
            .expect("store offer");
        let result = store
            .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
                offer_selector: "internet-exit".to_string(),
                buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
                mint_url: Some("https://mint.minibits.cash/Bitcoin".to_string()),
                channel_capacity_sat: Some(10),
                initial_paid_msat: 0,
                now_unix: 120,
            })
            .expect("open buyer session");
        (store, result.session_id, result.channel_id)
    }

    fn seller_payment_envelope(
        service_id: &str,
        lease_id: &str,
        buyer_npub: &str,
        seller_npub: &str,
        sent_at_unix: u64,
        payload: StreamingRoutePaymentPayload,
    ) -> StreamingRoutePaymentEnvelope {
        StreamingRoutePaymentEnvelope::new(
            service_id,
            lease_id,
            buyer_npub,
            seller_npub,
            sent_at_unix,
            payload,
        )
    }

    fn sample_spilman_payment(channel_id: &str, balance: u64) -> CashuSpilmanPayment {
        CashuSpilmanPayment {
            channel_id: channel_id.to_string(),
            balance,
            signature: format!("signature-{channel_id}-{balance}"),
            params: Some(json!({"channel": channel_id})),
            funding_proofs: Some(json!({"proofs": []})),
        }
    }

    struct FakePaymentSigner;

    impl CashuSpilmanPaymentSigner for FakePaymentSigner {
        fn sign_cashu_spilman_payment(
            &self,
            channel_id: &str,
            balance: u64,
            include_funding: bool,
        ) -> std::result::Result<CashuSpilmanPayment, String> {
            Ok(CashuSpilmanPayment {
                channel_id: channel_id.to_string(),
                balance,
                signature: format!(
                    "signed-{channel_id}-{}",
                    if include_funding { "funding" } else { "update" }
                ),
                params: include_funding.then(|| json!({"channel": channel_id})),
                funding_proofs: include_funding.then(|| json!({"proofs": []})),
            })
        }
    }

    struct FakeSpilmanReceiver {
        channel_id: String,
        balance: u64,
        validate_calls: std::cell::Cell<u32>,
        process_calls: std::cell::Cell<u32>,
    }

    impl FakeSpilmanReceiver {
        fn new(channel_id: &str, balance: u64) -> Self {
            Self {
                channel_id: channel_id.to_string(),
                balance,
                validate_calls: std::cell::Cell::new(0),
                process_calls: std::cell::Cell::new(0),
            }
        }

        fn validation(&self) -> CashuSpilmanPaymentReceiverValidation {
            CashuSpilmanPaymentReceiverValidation {
                channel_id: self.channel_id.clone(),
                balance: self.balance,
                amount_due: 0,
                capacity: 10,
            }
        }
    }

    impl CashuSpilmanPaymentReceiver<()> for FakeSpilmanReceiver {
        fn validate_cashu_spilman_payment(
            &self,
            _payment: &CashuSpilmanPayment,
            _context: &(),
        ) -> std::result::Result<CashuSpilmanPaymentReceiverValidation, String> {
            self.validate_calls.set(self.validate_calls.get() + 1);
            Ok(self.validation())
        }

        fn process_cashu_spilman_payment(
            &self,
            _payment: &CashuSpilmanPayment,
            _context: &(),
        ) -> std::result::Result<CashuSpilmanPaymentReceiverValidation, String> {
            self.process_calls.set(self.process_calls.get() + 1);
            Ok(self.validation())
        }
    }
}
