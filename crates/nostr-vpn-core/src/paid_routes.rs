use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
#[cfg(feature = "paid-exit")]
use cashu_service::{
    CashuSpilmanPayment, StreamingRouteAccessState, StreamingRouteCashuTokenLease,
    StreamingRouteDecision, StreamingRouteMeter, StreamingRoutePaymentEnvelope,
    StreamingRoutePolicy,
};
use nostr_sdk::prelude::{
    Event, EventBuilder, Filter, Keys, Kind, PublicKey, Tag, Timestamp, ToBech32,
};
use serde::{Deserialize, Serialize};

/// Parameterized replaceable Nostr event for generic paid route offers.
///
/// FIPS overlay endpoint discovery already uses kind 37195 for transport
/// locator adverts. Paid route offers deliberately use a separate adjacent
/// kind so market/payment terms do not overload endpoint discovery or require
/// publishing raw transport endpoints.
pub const PAID_ROUTE_OFFER_KIND: u16 = 37_196;
pub const PAID_ROUTE_OFFER_VERSION: &str = "1";
pub const PAID_ROUTE_OFFER_APP: &str = "fips/paid-route-offer";
pub const PAID_ROUTE_PAYMENT_VERSION: &str = "1";
pub const PAID_ROUTE_PAYMENT_APP: &str = "fips/paid-route-payment";
pub const DEFAULT_FIPS_PEER_RATING_SCOPE: &str = "fips.peer";

const DEFAULT_PRICE_DENOMINATOR_UNITS: u64 = 1_000_000;
const DEFAULT_MAX_CHANNEL_CAPACITY_SAT: u64 = 1_000;
const DEFAULT_CHANNEL_EXPIRY_SECS: u64 = 86_400;
const DEFAULT_FREE_PROBE_BYTES: u64 = 1_048_576;
const DEFAULT_GRACE_BYTES: u64 = 262_144;
const MILLIS_PER_DAY: u64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteServiceKind {
    #[default]
    InternetExit,
}

impl PaidRouteServiceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InternetExit => "internet_exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaidExitUpstream {
    #[default]
    HostDefault,
    #[serde(rename = "wireguard_exit")]
    WireGuardExit,
}

impl PaidExitUpstream {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostDefault => "host_default",
            Self::WireGuardExit => "wireguard_exit",
        }
    }
}

impl FromStr for PaidExitUpstream {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "" | "host_default" | "host" | "default" | "internet" | "local" => {
                Ok(Self::HostDefault)
            }
            "wireguard_exit" | "wireguard" | "wg" | "upstream_vpn" | "vpn" => {
                Ok(Self::WireGuardExit)
            }
            _ => Err(format!("unsupported paid exit upstream '{value}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaidRoutePrivateVpnAccess {
    #[default]
    Denied,
}

impl PaidRoutePrivateVpnAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidRouteAccessPolicy {
    #[serde(default)]
    pub upstream: PaidExitUpstream,
    #[serde(default)]
    pub private_vpn_access: PaidRoutePrivateVpnAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteMeter {
    Milliseconds,
    #[default]
    Bytes,
    Packets,
}

impl PaidRouteMeter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Milliseconds => "milliseconds",
            Self::Bytes => "bytes",
            Self::Packets => "packets",
        }
    }
}

#[cfg(feature = "paid-exit")]
impl From<PaidRouteMeter> for StreamingRouteMeter {
    fn from(value: PaidRouteMeter) -> Self {
        match value {
            PaidRouteMeter::Milliseconds => Self::Milliseconds,
            PaidRouteMeter::Bytes => Self::Bytes,
            PaidRouteMeter::Packets => Self::Packets,
        }
    }
}

impl FromStr for PaidRouteMeter {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "milliseconds" | "millis" | "ms" | "time" => Ok(Self::Milliseconds),
            "bytes" | "byte" | "bandwidth" => Ok(Self::Bytes),
            "packets" | "packet" => Ok(Self::Packets),
            _ => Err(format!("unsupported paid route meter '{value}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExitNetworkClass {
    #[default]
    Unknown,
    Datacenter,
    Residential,
    Mobile,
    Satellite,
    CommunityMesh,
}

impl ExitNetworkClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Datacenter => "datacenter",
            Self::Residential => "residential",
            Self::Mobile => "mobile",
            Self::Satellite => "satellite",
            Self::CommunityMesh => "community_mesh",
        }
    }
}

impl FromStr for ExitNetworkClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "" | "unknown" => Ok(Self::Unknown),
            "datacenter" | "data_center" | "dc" | "hosting" => Ok(Self::Datacenter),
            "residential" | "home" => Ok(Self::Residential),
            "mobile" | "cellular" | "lte" | "5g" => Ok(Self::Mobile),
            "satellite" | "starlink" => Ok(Self::Satellite),
            "community_mesh" | "mesh" | "community" => Ok(Self::CommunityMesh),
            _ => Err(format!("unsupported exit network class '{value}'")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteIpSupport {
    #[serde(default = "default_true")]
    pub ipv4: bool,
    #[serde(default)]
    pub ipv6: bool,
}

impl Default for PaidRouteIpSupport {
    fn default() -> Self {
        Self {
            ipv4: true,
            ipv6: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidRouteLocationHint {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub country_code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn: Option<u32>,
    #[serde(default, skip_serializing_if = "ExitNetworkClass::is_unknown")]
    pub network_class: ExitNetworkClass,
}

impl ExitNetworkClass {
    fn is_unknown(value: &Self) -> bool {
        *value == Self::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRoutePricing {
    #[serde(default)]
    pub meter: PaidRouteMeter,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub price_msat: u64,
    #[serde(default = "default_price_denominator_units")]
    pub per_units: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub connection_minimum_msat_per_day: u64,
}

impl Default for PaidRoutePricing {
    fn default() -> Self {
        Self {
            meter: PaidRouteMeter::Bytes,
            price_msat: 0,
            per_units: DEFAULT_PRICE_DENOMINATOR_UNITS,
            connection_minimum_msat_per_day: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteChannelTerms {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_mints: Vec<String>,
    #[serde(default = "default_max_channel_capacity_sat")]
    pub max_channel_capacity_sat: u64,
    #[serde(default = "default_channel_expiry_secs")]
    pub channel_expiry_secs: u64,
    #[serde(default = "default_free_probe_bytes")]
    pub free_probe_units: u64,
    #[serde(default = "default_grace_bytes")]
    pub grace_units: u64,
}

impl Default for PaidRouteChannelTerms {
    fn default() -> Self {
        Self {
            accepted_mints: Vec::new(),
            max_channel_capacity_sat: DEFAULT_MAX_CHANNEL_CAPACITY_SAT,
            channel_expiry_secs: DEFAULT_CHANNEL_EXPIRY_SECS,
            free_probe_units: DEFAULT_FREE_PROBE_BYTES,
            grace_units: DEFAULT_GRACE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidExitRatingDiscoveryConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<String>,
    #[serde(
        default = "default_fips_peer_rating_scope",
        skip_serializing_if = "fips_peer_rating_scope_is_default"
    )]
    pub scope: String,
}

impl Default for PaidExitRatingDiscoveryConfig {
    fn default() -> Self {
        Self {
            file: String::new(),
            relays: Vec::new(),
            scope: default_fips_peer_rating_scope(),
        }
    }
}

impl PaidExitRatingDiscoveryConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn normalize(&mut self) {
        self.file = self.file.trim().to_string();
        self.relays = normalize_string_list(&self.relays);
        self.scope = self.scope.trim().to_string();
        if self.scope.is_empty() {
            self.scope = default_fips_peer_rating_scope();
        }
    }

    pub fn configured(&self) -> bool {
        !self.file.trim().is_empty() || !self.relays.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidExitConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default)]
    pub access: PaidRouteAccessPolicy,
    #[serde(default)]
    pub pricing: PaidRoutePricing,
    #[serde(default)]
    pub channel: PaidRouteChannelTerms,
    #[serde(default)]
    pub location: PaidRouteLocationHint,
    #[serde(default)]
    pub ip_support: PaidRouteIpSupport,
    #[serde(
        default,
        skip_serializing_if = "PaidExitRatingDiscoveryConfig::is_default"
    )]
    pub rating_discovery: PaidExitRatingDiscoveryConfig,
}

impl PaidExitConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn normalize(&mut self) {
        self.access.private_vpn_access = PaidRoutePrivateVpnAccess::Denied;
        self.pricing.per_units = self.pricing.per_units.max(1);
        self.channel.max_channel_capacity_sat = self.channel.max_channel_capacity_sat.max(1);
        self.channel.channel_expiry_secs = self.channel.channel_expiry_secs.max(1);
        self.channel.accepted_mints = normalize_string_list(&self.channel.accepted_mints);
        self.location.country_code = normalize_country_code(&self.location.country_code);
        self.location.region = self.location.region.trim().to_string();
        self.rating_discovery.normalize();
    }

    #[cfg(feature = "paid-exit")]
    pub fn streaming_policy(&self) -> StreamingRoutePolicy {
        StreamingRoutePolicy {
            meter: self.pricing.meter.into(),
            price_msat: self.pricing.price_msat,
            per_units: self.pricing.per_units.max(1),
            max_channel_capacity_sat: self.channel.max_channel_capacity_sat.max(1),
            channel_expiry_secs: self.channel.channel_expiry_secs.max(1),
            free_probe_units: self.channel.free_probe_units,
            grace_units: self.channel.grace_units,
        }
    }

    pub fn amount_due_msat(&self, usage: &PaidRouteUsage) -> u64 {
        paid_route_amount_due_msat_for_usage(
            usage,
            self.pricing.meter,
            self.channel.free_probe_units,
            self.pricing.price_msat,
            self.pricing.per_units,
            self.pricing.connection_minimum_msat_per_day,
        )
    }

    pub fn amount_due_msat_with_connection_minimum_skew(
        &self,
        usage: &PaidRouteUsage,
        active_millis_skew: u64,
    ) -> u64 {
        paid_route_amount_due_msat_for_usage_with_connection_minimum_skew(
            usage,
            self.pricing.meter,
            self.channel.free_probe_units,
            self.pricing.price_msat,
            self.pricing.per_units,
            self.pricing.connection_minimum_msat_per_day,
            active_millis_skew,
        )
    }

    pub fn routing_decision(
        &self,
        usage: &PaidRouteUsage,
        paid_msat: u64,
    ) -> PaidRouteRoutingDecision {
        let delivered_units = usage.billable_units_for_meter(self.pricing.meter);
        let amount_due_msat = self.amount_due_msat(usage);
        let mut enforced_usage = usage.clone();
        set_billable_units_for_meter(
            &mut enforced_usage,
            self.pricing.meter,
            delivered_units.saturating_sub(self.channel.grace_units),
        );
        let enforced_amount_due_msat = self.amount_due_msat(&enforced_usage);
        let unpaid_msat = amount_due_msat.saturating_sub(paid_msat);
        let enforced_unpaid_msat = enforced_amount_due_msat.saturating_sub(paid_msat);
        let state = if amount_due_msat == 0 {
            PaidRouteAccessState::FreeProbe
        } else if unpaid_msat == 0 {
            PaidRouteAccessState::Paid
        } else if enforced_unpaid_msat == 0 {
            PaidRouteAccessState::Grace
        } else {
            PaidRouteAccessState::Suspended
        };

        PaidRouteRoutingDecision {
            state,
            allow_routing: state != PaidRouteAccessState::Suspended,
            delivered_units,
            paid_msat,
            amount_due_msat,
            enforced_amount_due_msat,
            unpaid_msat,
            free_probe_remaining_units: self
                .channel
                .free_probe_units
                .saturating_sub(delivered_units),
            grace_remaining_units: paid_route_grace_remaining_units(
                delivered_units,
                self.channel.free_probe_units,
                self.channel.grace_units,
                paid_msat,
                self.pricing.price_msat,
                self.pricing.per_units,
            ),
        }
    }

    pub fn can_continue_routing(&self, usage: &PaidRouteUsage, paid_msat: u64) -> bool {
        self.routing_decision(usage, paid_msat).allow_routing
    }

    pub fn from_paid_route_offer(offer: &PaidRouteOffer) -> Self {
        Self {
            enabled: true,
            access: offer.access.clone(),
            pricing: offer.pricing.clone(),
            channel: offer.channel.clone(),
            location: offer.location.clone(),
            ip_support: offer.ip_support.clone(),
            rating_discovery: PaidExitRatingDiscoveryConfig::default(),
        }
    }
}

fn default_fips_peer_rating_scope() -> String {
    DEFAULT_FIPS_PEER_RATING_SCOPE.to_string()
}

fn fips_peer_rating_scope_is_default(value: &str) -> bool {
    value == DEFAULT_FIPS_PEER_RATING_SCOPE
}

fn paid_route_amount_due_msat(
    delivered_units: u64,
    free_probe_units: u64,
    price_msat: u64,
    per_units: u64,
) -> u64 {
    let billable_units = delivered_units.saturating_sub(free_probe_units);
    paid_route_price_for_units(billable_units, price_msat, per_units)
}

fn paid_route_amount_due_msat_for_usage(
    usage: &PaidRouteUsage,
    meter: PaidRouteMeter,
    free_probe_units: u64,
    price_msat: u64,
    per_units: u64,
    connection_minimum_msat_per_day: u64,
) -> u64 {
    let traffic_due = paid_route_amount_due_msat(
        usage.billable_units_for_meter(meter),
        free_probe_units,
        price_msat,
        per_units,
    );
    let connection_due = paid_route_connection_minimum_due_msat(
        usage.active_millis,
        connection_minimum_msat_per_day,
    );
    traffic_due.max(connection_due)
}

fn paid_route_amount_due_msat_for_usage_with_connection_minimum_skew(
    usage: &PaidRouteUsage,
    meter: PaidRouteMeter,
    free_probe_units: u64,
    price_msat: u64,
    per_units: u64,
    connection_minimum_msat_per_day: u64,
    active_millis_skew: u64,
) -> u64 {
    let traffic_due = paid_route_amount_due_msat(
        usage.billable_units_for_meter(meter),
        free_probe_units,
        price_msat,
        per_units,
    );
    let connection_due = paid_route_connection_minimum_due_msat(
        usage.active_millis.saturating_sub(active_millis_skew),
        connection_minimum_msat_per_day,
    );
    traffic_due.max(connection_due)
}

fn paid_route_connection_minimum_due_msat(
    active_millis: u64,
    connection_minimum_msat_per_day: u64,
) -> u64 {
    if active_millis == 0 || connection_minimum_msat_per_day == 0 {
        return 0;
    }
    active_millis
        .saturating_mul(connection_minimum_msat_per_day)
        .saturating_div(MILLIS_PER_DAY)
}

fn paid_route_price_for_units(units: u64, price_msat: u64, per_units: u64) -> u64 {
    if units == 0 || price_msat == 0 {
        return 0;
    }
    let numerator = u128::from(units).saturating_mul(u128::from(price_msat));
    let denominator = u128::from(per_units.max(1));
    let due = numerator
        .saturating_add(denominator.saturating_sub(1))
        .saturating_div(denominator);
    due.min(u128::from(u64::MAX)) as u64
}

fn paid_route_grace_remaining_units(
    delivered_units: u64,
    free_probe_units: u64,
    grace_units: u64,
    paid_msat: u64,
    price_msat: u64,
    per_units: u64,
) -> u64 {
    let billable_units = delivered_units.saturating_sub(free_probe_units);
    if billable_units == 0 || grace_units == 0 {
        return 0;
    }
    let paid_units = if price_msat == 0 {
        billable_units
    } else {
        let units = u128::from(paid_msat)
            .saturating_mul(u128::from(per_units.max(1)))
            .saturating_div(u128::from(price_msat));
        units.min(u128::from(u64::MAX)) as u64
    };
    paid_units
        .saturating_add(grace_units)
        .saturating_sub(billable_units)
        .min(grace_units)
}

fn set_billable_units_for_meter(
    usage: &mut PaidRouteUsage,
    meter: PaidRouteMeter,
    delivered_units: u64,
) {
    match meter {
        PaidRouteMeter::Milliseconds => usage.active_millis = delivered_units,
        PaidRouteMeter::Bytes => usage.billable_bytes = delivered_units,
        PaidRouteMeter::Packets => usage.billable_packets = delivered_units,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidRouteUsage {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub active_millis: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tx_bytes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rx_bytes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tx_packets: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rx_packets: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub billable_bytes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub billable_packets: u64,
}

impl PaidRouteUsage {
    pub fn units_for_meter(&self, meter: PaidRouteMeter) -> u64 {
        match meter {
            PaidRouteMeter::Milliseconds => self.active_millis,
            PaidRouteMeter::Bytes => self.tx_bytes.saturating_add(self.rx_bytes),
            PaidRouteMeter::Packets => self.tx_packets.saturating_add(self.rx_packets),
        }
    }

    pub fn billable_units_for_meter(&self, meter: PaidRouteMeter) -> u64 {
        match meter {
            PaidRouteMeter::Milliseconds => self.active_millis,
            PaidRouteMeter::Bytes => self.billable_bytes,
            PaidRouteMeter::Packets => self.billable_packets,
        }
    }

    pub fn add_assign(&mut self, delta: &Self) {
        self.active_millis = self.active_millis.saturating_add(delta.active_millis);
        self.tx_bytes = self.tx_bytes.saturating_add(delta.tx_bytes);
        self.rx_bytes = self.rx_bytes.saturating_add(delta.rx_bytes);
        self.tx_packets = self.tx_packets.saturating_add(delta.tx_packets);
        self.rx_packets = self.rx_packets.saturating_add(delta.rx_packets);
        self.billable_bytes = self.billable_bytes.saturating_add(delta.billable_bytes);
        self.billable_packets = self.billable_packets.saturating_add(delta.billable_packets);
    }

    pub fn is_empty(&self) -> bool {
        self.active_millis == 0
            && self.tx_bytes == 0
            && self.rx_bytes == 0
            && self.tx_packets == 0
            && self.rx_packets == 0
            && self.billable_bytes == 0
            && self.billable_packets == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidRouteQualityMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_loss_ppm: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_unix: Option<u64>,
}

impl PaidRouteQualityMetrics {
    pub fn is_empty(&self) -> bool {
        self.latency_ms.is_none()
            && self.jitter_ms.is_none()
            && self.packet_loss_ppm.is_none()
            && self.down_bps.is_none()
            && self.up_bps.is_none()
            && self.uptime_secs.is_none()
            && self.last_seen_unix.is_none()
    }

    pub fn merge_patch(&mut self, patch: PaidRouteQualityMetrics) {
        if patch.latency_ms.is_some() {
            self.latency_ms = patch.latency_ms;
        }
        if patch.jitter_ms.is_some() {
            self.jitter_ms = patch.jitter_ms;
        }
        if patch.packet_loss_ppm.is_some() {
            self.packet_loss_ppm = patch.packet_loss_ppm;
        }
        if patch.down_bps.is_some() {
            self.down_bps = patch.down_bps;
        }
        if patch.up_bps.is_some() {
            self.up_bps = patch.up_bps;
        }
        if patch.uptime_secs.is_some() {
            self.uptime_secs = patch.uptime_secs;
        }
        if patch.last_seen_unix.is_some() {
            self.last_seen_unix = patch.last_seen_unix;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaidRoutePaymentMode {
    #[default]
    CashuSpilman,
    CashuTokenLease,
}

impl PaidRoutePaymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CashuSpilman => "cashu_spilman",
            Self::CashuTokenLease => "cashu_token_lease",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteAccessState {
    FreeProbe,
    Paid,
    Grace,
    Suspended,
}

impl PaidRouteAccessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FreeProbe => "free_probe",
            Self::Paid => "paid",
            Self::Grace => "grace",
            Self::Suspended => "suspended",
        }
    }
}

#[cfg(feature = "paid-exit")]
impl From<StreamingRouteAccessState> for PaidRouteAccessState {
    fn from(value: StreamingRouteAccessState) -> Self {
        match value {
            StreamingRouteAccessState::FreeProbe => Self::FreeProbe,
            StreamingRouteAccessState::Paid => Self::Paid,
            StreamingRouteAccessState::Grace => Self::Grace,
            StreamingRouteAccessState::Suspended => Self::Suspended,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteRoutingDecision {
    pub state: PaidRouteAccessState,
    pub allow_routing: bool,
    pub delivered_units: u64,
    pub paid_msat: u64,
    pub amount_due_msat: u64,
    pub enforced_amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub free_probe_remaining_units: u64,
    pub grace_remaining_units: u64,
}

#[cfg(feature = "paid-exit")]
impl From<StreamingRouteDecision> for PaidRouteRoutingDecision {
    fn from(value: StreamingRouteDecision) -> Self {
        Self {
            state: value.state.into(),
            allow_routing: value.allow_routing,
            delivered_units: value.delivered_units,
            paid_msat: value.paid_msat,
            amount_due_msat: value.amount_due_msat,
            enforced_amount_due_msat: value.enforced_amount_due_msat,
            unpaid_msat: value.unpaid_msat,
            free_probe_remaining_units: value.free_probe_remaining_units,
            grace_remaining_units: value.grace_remaining_units,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaidRouteCountryClaimStatus {
    NoClaim,
    Unknown,
    Match,
    Mismatch,
}

impl PaidRouteCountryClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoClaim => "no_claim",
            Self::Unknown => "unknown",
            Self::Match => "match",
            Self::Mismatch => "mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteCountryClaim {
    pub claimed_country_code: String,
    pub observed_country_code: String,
    pub status: PaidRouteCountryClaimStatus,
}

impl PaidRouteCountryClaim {
    pub fn matches_claim(&self) -> Option<bool> {
        match self.status {
            PaidRouteCountryClaimStatus::Match => Some(true),
            PaidRouteCountryClaimStatus::Mismatch => Some(false),
            PaidRouteCountryClaimStatus::NoClaim | PaidRouteCountryClaimStatus::Unknown => None,
        }
    }
}

pub fn paid_route_country_claim(
    claimed_country_code: impl AsRef<str>,
    observed_country_code: Option<&str>,
) -> PaidRouteCountryClaim {
    let claimed_country_code = normalize_country_code(claimed_country_code.as_ref());
    let observed_country_code = observed_country_code
        .map(normalize_country_code)
        .unwrap_or_default();
    let status = if claimed_country_code.is_empty() {
        PaidRouteCountryClaimStatus::NoClaim
    } else if observed_country_code.is_empty() {
        PaidRouteCountryClaimStatus::Unknown
    } else if claimed_country_code == observed_country_code {
        PaidRouteCountryClaimStatus::Match
    } else {
        PaidRouteCountryClaimStatus::Mismatch
    };
    PaidRouteCountryClaim {
        claimed_country_code,
        observed_country_code,
        status,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteOffer {
    pub offer_id: String,
    pub seller_npub: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receiver_pubkey_hex: String,
    #[serde(default)]
    pub service: PaidRouteServiceKind,
    #[serde(default)]
    pub access: PaidRouteAccessPolicy,
    #[serde(default)]
    pub pricing: PaidRoutePricing,
    #[serde(default)]
    pub channel: PaidRouteChannelTerms,
    #[serde(default)]
    pub location: PaidRouteLocationHint,
    #[serde(default)]
    pub ip_support: PaidRouteIpSupport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<PaidRouteQualityMetrics>,
}

impl PaidRouteOffer {
    pub fn from_paid_exit_config(
        offer_id: impl Into<String>,
        seller_npub: impl Into<String>,
        config: &PaidExitConfig,
        quality: Option<PaidRouteQualityMetrics>,
    ) -> Self {
        Self::from_paid_exit_config_with_receiver(offer_id, seller_npub, config, None, quality)
    }

    pub fn from_paid_exit_config_with_receiver(
        offer_id: impl Into<String>,
        seller_npub: impl Into<String>,
        config: &PaidExitConfig,
        receiver_pubkey_hex: Option<&str>,
        quality: Option<PaidRouteQualityMetrics>,
    ) -> Self {
        let mut config = config.clone();
        config.normalize();
        Self {
            offer_id: offer_id.into(),
            seller_npub: seller_npub.into(),
            receiver_pubkey_hex: receiver_pubkey_hex
                .map(normalize_receiver_pubkey_hex_lossy)
                .unwrap_or_default(),
            service: PaidRouteServiceKind::InternetExit,
            access: config.access.clone(),
            pricing: config.pricing.clone(),
            channel: config.channel.clone(),
            location: config.location.clone(),
            ip_support: config.ip_support.clone(),
            quality,
        }
    }
}

pub fn signed_paid_exit_offer_from_config(
    offer_id: impl Into<String>,
    keys: &Keys,
    config: &PaidExitConfig,
    quality: Option<PaidRouteQualityMetrics>,
    signed_at: u64,
) -> Result<SignedPaidRouteOffer> {
    signed_paid_exit_offer_from_config_with_receiver(
        offer_id, keys, config, None, quality, signed_at,
    )
}

pub fn signed_paid_exit_offer_from_config_with_receiver(
    offer_id: impl Into<String>,
    keys: &Keys,
    config: &PaidExitConfig,
    receiver_pubkey_hex: Option<&str>,
    quality: Option<PaidRouteQualityMetrics>,
    signed_at: u64,
) -> Result<SignedPaidRouteOffer> {
    if !config.enabled {
        return Err(anyhow!("paid exit selling is disabled"));
    }
    let mut normalized_config = config.clone();
    normalized_config.normalize();
    if (normalized_config.pricing.price_msat > 0
        || normalized_config.pricing.connection_minimum_msat_per_day > 0)
        && normalized_config.channel.accepted_mints.is_empty()
    {
        return Err(anyhow!(
            "paid exit offers with non-zero pricing require at least one accepted Cashu mint"
        ));
    }

    let receiver_pubkey_hex = receiver_pubkey_hex
        .map(normalize_receiver_pubkey_hex)
        .transpose()?;
    let offer = PaidRouteOffer::from_paid_exit_config_with_receiver(
        offer_id,
        public_key_npub(&keys.public_key())?,
        &normalized_config,
        receiver_pubkey_hex.as_deref(),
        quality,
    );
    SignedPaidRouteOffer::sign(offer, keys, signed_at)
}

pub fn paid_route_offer_filter(limit: usize, since_unix: Option<u64>) -> Filter {
    let mut filter = Filter::new().kind(Kind::Custom(PAID_ROUTE_OFFER_KIND));
    if limit > 0 {
        filter = filter.limit(limit);
    }
    if let Some(since_unix) = since_unix {
        filter = filter.since(Timestamp::from(since_unix));
    }
    filter
}

pub fn paid_route_payment_filter(
    recipient: PublicKey,
    limit: usize,
    since_unix: Option<u64>,
) -> Filter {
    let mut filter = Filter::new().kind(Kind::GiftWrap).pubkey(recipient);
    if limit > 0 {
        filter = filter.limit(limit);
    }
    if let Some(since_unix) = since_unix {
        filter = filter.since(Timestamp::from(since_unix));
    }
    filter
}

#[cfg(feature = "paid-exit")]
pub async fn gift_wrap_paid_route_payment(
    envelope: &StreamingRoutePaymentEnvelope,
    keys: &Keys,
) -> Result<Event> {
    let buyer_npub = normalize_npub(&envelope.buyer, "buyer")?;
    let local_npub = public_key_npub(&keys.public_key())?;
    if buyer_npub != local_npub {
        return Err(anyhow!(
            "paid route payment buyer does not match local signer"
        ));
    }

    let seller = PublicKey::parse(&envelope.seller)
        .map_err(|error| anyhow!("invalid paid route payment seller npub: {error}"))?;
    let content =
        serde_json::to_string(envelope).context("failed to encode paid route payment envelope")?;
    EventBuilder::private_msg(
        keys,
        seller,
        content,
        paid_route_payment_rumor_tags(envelope)?,
    )
    .await
    .map_err(|error| anyhow!("failed to gift-wrap paid route payment: {error}"))
}

#[cfg(feature = "paid-exit")]
pub async fn unwrap_paid_route_payment(
    event: &Event,
    keys: &Keys,
) -> Result<StreamingRoutePaymentEnvelope> {
    if event.kind != Kind::GiftWrap {
        return Err(anyhow!("paid route payment event is not a gift wrap"));
    }

    let unwrapped = nostr_sdk::prelude::nip59::extract_rumor(keys, event)
        .await
        .map_err(|error| anyhow!("failed to unwrap paid route payment: {error}"))?;
    if unwrapped.rumor.kind != Kind::PrivateDirectMessage {
        return Err(anyhow!("paid route payment rumor is not a private message"));
    }
    validate_paid_route_payment_rumor_tags(unwrapped.rumor.tags.as_slice())?;

    let envelope: StreamingRoutePaymentEnvelope = serde_json::from_str(&unwrapped.rumor.content)
        .context("failed to decode paid route payment envelope")?;
    let sender_npub = public_key_npub(&unwrapped.sender)?;
    let buyer_npub = normalize_npub(&envelope.buyer, "buyer")?;
    if sender_npub != buyer_npub {
        return Err(anyhow!(
            "paid route payment buyer does not match gift-wrap sender"
        ));
    }

    let local_npub = public_key_npub(&keys.public_key())?;
    let seller_npub = normalize_npub(&envelope.seller, "seller")?;
    if seller_npub != local_npub {
        return Err(anyhow!(
            "paid route payment seller does not match local recipient"
        ));
    }

    Ok(envelope)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPaidRouteOffer {
    pub event: Event,
}

impl SignedPaidRouteOffer {
    pub fn sign(offer: PaidRouteOffer, keys: &Keys, signed_at: u64) -> Result<Self> {
        validate_paid_route_offer(&offer)?;
        let content = serde_json::to_string(&offer).context("failed to encode paid route offer")?;
        let event = EventBuilder::new(Kind::Custom(PAID_ROUTE_OFFER_KIND), content)
            .tags(paid_route_offer_tags(&offer)?)
            .custom_created_at(Timestamp::from(signed_at))
            .sign_with_keys(keys)
            .map_err(|error| anyhow!("failed to sign paid route offer: {error}"))?;
        let signed = Self { event };
        signed.verify()?;
        Ok(signed)
    }

    pub fn from_event(event: Event) -> Result<Self> {
        let signed = Self { event };
        signed.verify()?;
        Ok(signed)
    }

    pub fn verify(&self) -> Result<()> {
        if u16::from(self.event.kind) != PAID_ROUTE_OFFER_KIND {
            return Err(anyhow!(
                "unexpected paid route offer event kind {}",
                u16::from(self.event.kind)
            ));
        }
        self.event
            .verify()
            .map_err(|error| anyhow!("invalid paid route offer signature: {error}"))?;
        let offer = self.offer()?;
        validate_paid_route_offer(&offer)?;
        let signer_npub = public_key_npub(&self.event.pubkey)?;
        let offer_seller_npub = normalize_npub(&offer.seller_npub, "seller")?;
        if signer_npub != offer_seller_npub {
            return Err(anyhow!(
                "paid route offer seller does not match event signer"
            ));
        }
        validate_paid_route_offer_tags(self.event.tags.as_slice(), &offer)?;
        Ok(())
    }

    pub fn offer(&self) -> Result<PaidRouteOffer> {
        serde_json::from_str(&self.event.content).context("failed to decode paid route offer")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteQuote {
    pub quote_id: String,
    pub offer_id: String,
    #[serde(default)]
    pub payment_mode: PaidRoutePaymentMode,
    pub channel_capacity_sat: u64,
    pub expires_at_unix: u64,
    pub receiver_pubkey_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteLease {
    pub lease_id: String,
    pub offer_id: String,
    pub quote_id: String,
    pub buyer_npub: String,
    pub starts_at_unix: u64,
    pub expires_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaidRoutePaymentState {
    #[serde(default)]
    pub mode: PaidRoutePaymentMode,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cashu_unit: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub capacity_sat: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paid_msat: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated_at_unix: u64,
    #[cfg(feature = "paid-exit")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cashu_spilman_payment: Option<CashuSpilmanPayment>,
    #[cfg(feature = "paid-exit")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cashu_token_lease: Option<StreamingRouteCashuTokenLease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaidRouteSession {
    pub session_id: String,
    pub lease_id: String,
    #[serde(default)]
    pub usage: PaidRouteUsage,
    #[serde(default)]
    pub payment: PaidRoutePaymentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_exit_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_country_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_asn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<PaidRouteQualityMetrics>,
}

impl PaidRouteSession {
    pub fn routing_decision(&self, config: &PaidExitConfig) -> PaidRouteRoutingDecision {
        config.routing_decision(&self.usage, self.payment.paid_msat)
    }

    pub fn can_continue_routing(&self, config: &PaidExitConfig) -> bool {
        self.routing_decision(config).allow_routing
    }
}

fn default_true() -> bool {
    true
}

fn default_price_denominator_units() -> u64 {
    DEFAULT_PRICE_DENOMINATOR_UNITS
}

fn default_max_channel_capacity_sat() -> u64 {
    DEFAULT_MAX_CHANNEL_CAPACITY_SAT
}

fn default_channel_expiry_secs() -> u64 {
    DEFAULT_CHANNEL_EXPIRY_SECS
}

fn default_free_probe_bytes() -> u64 {
    DEFAULT_FREE_PROBE_BYTES
}

fn default_grace_bytes() -> u64 {
    DEFAULT_GRACE_BYTES
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn normalize_country_code(value: &str) -> String {
    let value = value.trim();
    if value.len() == 2 && value.chars().all(|ch| ch.is_ascii_alphabetic()) {
        value.to_ascii_uppercase()
    } else {
        String::new()
    }
}

fn normalize_enum_value(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .flat_map(|value| value.split([',', '\n', '\r', '\t']))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn paid_route_offer_tags(offer: &PaidRouteOffer) -> Result<Vec<Tag>> {
    let mut tags = vec![
        Tag::identifier(offer.offer_id.trim().to_string()),
        paid_route_tag(&["app", PAID_ROUTE_OFFER_APP])?,
        paid_route_tag(&["v", PAID_ROUTE_OFFER_VERSION])?,
        paid_route_tag(&["service", offer.service.as_str()])?,
        paid_route_tag(&["payment", PaidRoutePaymentMode::CashuSpilman.as_str()])?,
        paid_route_tag(&["meter", offer.pricing.meter.as_str()])?,
        paid_route_owned_tag(vec![
            "price_msat".to_string(),
            offer.pricing.price_msat.to_string(),
        ])?,
        paid_route_owned_tag(vec![
            "per_units".to_string(),
            offer.pricing.per_units.to_string(),
        ])?,
        paid_route_owned_tag(vec![
            "connection_minimum_msat_per_day".to_string(),
            offer.pricing.connection_minimum_msat_per_day.to_string(),
        ])?,
        paid_route_owned_tag(vec![
            "max_channel_capacity_sat".to_string(),
            offer.channel.max_channel_capacity_sat.to_string(),
        ])?,
        paid_route_owned_tag(vec![
            "channel_expiry_secs".to_string(),
            offer.channel.channel_expiry_secs.to_string(),
        ])?,
        paid_route_number_tag("free_probe_units", offer.channel.free_probe_units)?,
        paid_route_number_tag("grace_units", offer.channel.grace_units)?,
        paid_route_tag(&["upstream", offer.access.upstream.as_str()])?,
        paid_route_tag(&[
            "private_vpn_access",
            offer.access.private_vpn_access.as_str(),
        ])?,
    ];

    if !offer.receiver_pubkey_hex.trim().is_empty() {
        tags.push(paid_route_owned_tag(vec![
            "receiver_pubkey".to_string(),
            normalize_receiver_pubkey_hex(&offer.receiver_pubkey_hex)?,
        ])?);
    }

    for mint in normalize_string_list(&offer.channel.accepted_mints) {
        tags.push(paid_route_owned_tag(vec!["mint".to_string(), mint])?);
    }
    if !offer.location.country_code.trim().is_empty() {
        tags.push(paid_route_tag(&[
            "country",
            offer.location.country_code.trim(),
        ])?);
    }
    if !offer.location.region.trim().is_empty() {
        tags.push(paid_route_tag(&["region", offer.location.region.trim()])?);
    }
    if let Some(asn) = offer.location.asn {
        tags.push(paid_route_owned_tag(vec![
            "asn".to_string(),
            asn.to_string(),
        ])?);
    }
    if offer.location.network_class != ExitNetworkClass::Unknown {
        tags.push(paid_route_tag(&[
            "network_class",
            offer.location.network_class.as_str(),
        ])?);
    }
    if offer.ip_support.ipv4 {
        tags.push(paid_route_tag(&["ip", "ipv4"])?);
    }
    if offer.ip_support.ipv6 {
        tags.push(paid_route_tag(&["ip", "ipv6"])?);
    }
    if let Some(quality) = &offer.quality {
        if let Some(latency_ms) = quality.latency_ms {
            tags.push(paid_route_number_tag("latency_ms", latency_ms)?);
        }
        if let Some(jitter_ms) = quality.jitter_ms {
            tags.push(paid_route_number_tag("jitter_ms", jitter_ms)?);
        }
        if let Some(packet_loss_ppm) = quality.packet_loss_ppm {
            tags.push(paid_route_number_tag("packet_loss_ppm", packet_loss_ppm)?);
        }
        if let Some(down_bps) = quality.down_bps {
            tags.push(paid_route_number_tag("down_bps", down_bps)?);
        }
        if let Some(up_bps) = quality.up_bps {
            tags.push(paid_route_number_tag("up_bps", up_bps)?);
        }
        if let Some(uptime_secs) = quality.uptime_secs {
            tags.push(paid_route_number_tag("uptime_secs", uptime_secs)?);
        }
        if let Some(last_seen_unix) = quality.last_seen_unix {
            tags.push(paid_route_number_tag("last_seen_unix", last_seen_unix)?);
        }
    }

    Ok(tags)
}

fn paid_route_number_tag(name: &str, value: impl ToString) -> Result<Tag> {
    paid_route_owned_tag(vec![name.to_string(), value.to_string()])
}

fn paid_route_tag(parts: &[&str]) -> Result<Tag> {
    Tag::parse(parts.iter().copied())
        .map_err(|error| anyhow!("failed to build paid route tag: {error}"))
}

fn paid_route_owned_tag(parts: Vec<String>) -> Result<Tag> {
    Tag::parse(parts).map_err(|error| anyhow!("failed to build paid route tag: {error}"))
}

#[cfg(feature = "paid-exit")]
fn paid_route_payment_rumor_tags(envelope: &StreamingRoutePaymentEnvelope) -> Result<Vec<Tag>> {
    Ok(vec![
        paid_route_tag(&["app", PAID_ROUTE_PAYMENT_APP])?,
        paid_route_tag(&["v", PAID_ROUTE_PAYMENT_VERSION])?,
        paid_route_tag(&["service", envelope.service_id.as_str()])?,
        paid_route_tag(&["lease", envelope.lease_id.as_str()])?,
        paid_route_tag(&["channel", envelope.channel_id()])?,
    ])
}

#[cfg(feature = "paid-exit")]
fn validate_paid_route_payment_rumor_tags(tags: &[Tag]) -> Result<()> {
    let mut app_ok = false;
    let mut version_ok = false;
    for tag in tags {
        let parts = tag.as_slice();
        let Some(kind) = parts.first().map(String::as_str) else {
            continue;
        };
        match kind {
            "app" => {
                app_ok |= parts
                    .get(1)
                    .is_some_and(|value| value == PAID_ROUTE_PAYMENT_APP)
            }
            "v" => {
                version_ok |= parts
                    .get(1)
                    .is_some_and(|value| value == PAID_ROUTE_PAYMENT_VERSION)
            }
            _ => {}
        }
    }

    if !app_ok {
        return Err(anyhow!("paid route payment rumor is missing app tag"));
    }
    if !version_ok {
        return Err(anyhow!("paid route payment rumor is missing version tag"));
    }
    Ok(())
}

fn validate_paid_route_offer(offer: &PaidRouteOffer) -> Result<()> {
    if offer.offer_id.trim().is_empty() {
        return Err(anyhow!("paid route offer id is empty"));
    }
    let _ = normalize_npub(&offer.seller_npub, "seller")?;
    if offer.access.private_vpn_access != PaidRoutePrivateVpnAccess::Denied {
        return Err(anyhow!(
            "paid public exit offers must deny private VPN access"
        ));
    }
    if offer.service != PaidRouteServiceKind::InternetExit {
        return Err(anyhow!("unsupported paid route service"));
    }
    if offer.pricing.per_units == 0 {
        return Err(anyhow!(
            "paid route price denominator must be greater than zero"
        ));
    }
    if offer.channel.max_channel_capacity_sat == 0 {
        return Err(anyhow!(
            "paid route max channel capacity must be greater than zero"
        ));
    }
    if offer.channel.channel_expiry_secs == 0 {
        return Err(anyhow!(
            "paid route channel expiry must be greater than zero"
        ));
    }
    Ok(())
}

fn validate_paid_route_offer_tags(tags: &[Tag], offer: &PaidRouteOffer) -> Result<()> {
    let mut app_ok = false;
    let mut version_ok = false;
    let mut id_ok = false;
    let mut service_ok = false;
    let mut payment_ok = false;
    let mut meter_ok = false;
    let mut upstream_ok = false;
    let mut access_ok = false;
    let mut receiver_ok = offer.receiver_pubkey_hex.trim().is_empty();
    let offer_receiver_pubkey_hex = if offer.receiver_pubkey_hex.trim().is_empty() {
        String::new()
    } else {
        normalize_receiver_pubkey_hex(&offer.receiver_pubkey_hex)?
    };

    for tag in tags {
        let parts = tag.as_slice();
        let Some(kind) = parts.first().map(String::as_str) else {
            continue;
        };
        match kind {
            "d" => id_ok |= parts.get(1).is_some_and(|value| value == &offer.offer_id),
            "app" => {
                app_ok |= parts
                    .get(1)
                    .is_some_and(|value| value == PAID_ROUTE_OFFER_APP)
            }
            "v" => {
                version_ok |= parts
                    .get(1)
                    .is_some_and(|value| value == PAID_ROUTE_OFFER_VERSION)
            }
            "service" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!("paid route offer event has empty service tag"));
                };
                if value != offer.service.as_str() {
                    return Err(anyhow!(
                        "paid route offer event service tag does not match content"
                    ));
                }
                service_ok = true;
            }
            "payment" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!("paid route offer event has empty payment tag"));
                };
                if value != PaidRoutePaymentMode::CashuSpilman.as_str() {
                    return Err(anyhow!(
                        "paid route offer event payment tag does not match default mode"
                    ));
                }
                payment_ok = true;
            }
            "meter" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!("paid route offer event has empty meter tag"));
                };
                if value != offer.pricing.meter.as_str() {
                    return Err(anyhow!(
                        "paid route offer event meter tag does not match content"
                    ));
                }
                meter_ok = true;
            }
            "upstream" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!("paid route offer event has empty upstream tag"));
                };
                if value != offer.access.upstream.as_str() {
                    return Err(anyhow!(
                        "paid route offer event upstream tag does not match content"
                    ));
                }
                upstream_ok = true;
            }
            "private_vpn_access" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!(
                        "paid route offer event has empty private access tag"
                    ));
                };
                if value != "denied" {
                    return Err(anyhow!(
                        "paid route offer event advertises private VPN access"
                    ));
                }
                access_ok = true;
            }
            "receiver_pubkey" => {
                let Some(value) = parts.get(1) else {
                    return Err(anyhow!(
                        "paid route offer event has empty receiver pubkey tag"
                    ));
                };
                let tag_receiver_pubkey_hex = normalize_receiver_pubkey_hex(value)?;
                if tag_receiver_pubkey_hex != offer_receiver_pubkey_hex {
                    return Err(anyhow!(
                        "paid route offer event receiver pubkey tag does not match content"
                    ));
                }
                receiver_ok = true;
            }
            _ => {}
        }
    }

    if !id_ok {
        return Err(anyhow!("paid route offer event is missing matching d tag"));
    }
    if !app_ok {
        return Err(anyhow!("paid route offer event is missing app tag"));
    }
    if !version_ok {
        return Err(anyhow!("paid route offer event is missing version tag"));
    }
    if !service_ok {
        return Err(anyhow!("paid route offer event is missing service tag"));
    }
    if !payment_ok {
        return Err(anyhow!("paid route offer event is missing payment tag"));
    }
    if !meter_ok {
        return Err(anyhow!("paid route offer event is missing meter tag"));
    }
    if !upstream_ok {
        return Err(anyhow!("paid route offer event is missing upstream tag"));
    }
    if !access_ok {
        return Err(anyhow!(
            "paid route offer event is missing private access denial tag"
        ));
    }
    if !receiver_ok {
        return Err(anyhow!(
            "paid route offer event is missing matching receiver pubkey tag"
        ));
    }
    Ok(())
}

fn normalize_receiver_pubkey_hex(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(format!("02{}", hex.to_ascii_lowercase()));
    }
    if hex.len() == 66
        && matches!(&hex[..2], "02" | "03")
        && hex.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Ok(hex.to_ascii_lowercase());
    }
    Err(anyhow!("invalid paid route receiver pubkey"))
}

fn normalize_receiver_pubkey_hex_lossy(value: &str) -> String {
    normalize_receiver_pubkey_hex(value).unwrap_or_default()
}

fn normalize_npub(value: &str, role: &str) -> Result<String> {
    let public_key = PublicKey::parse(value.trim())
        .map_err(|error| anyhow!("invalid paid route {role} npub: {error}"))?;
    public_key
        .to_bech32()
        .context("failed to encode paid route npub")
}

fn public_key_npub(public_key: &PublicKey) -> Result<String> {
    public_key
        .to_bech32()
        .context("failed to encode paid route event signer npub")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paid_exit_config_normalizes_operator_hints() {
        let mut config = PaidExitConfig {
            enabled: true,
            access: PaidRouteAccessPolicy {
                upstream: PaidExitUpstream::WireGuardExit,
                private_vpn_access: PaidRoutePrivateVpnAccess::Denied,
            },
            pricing: PaidRoutePricing {
                meter: PaidRouteMeter::Bytes,
                price_msat: 25,
                per_units: 0,
                connection_minimum_msat_per_day: 0,
            },
            channel: PaidRouteChannelTerms {
                accepted_mints: vec![
                    " https://mint.example ".to_string(),
                    "https://mint.example".to_string(),
                    "https://mint2.example, https://mint3.example".to_string(),
                ],
                max_channel_capacity_sat: 0,
                channel_expiry_secs: 0,
                free_probe_units: 100,
                grace_units: 20,
            },
            location: PaidRouteLocationHint {
                country_code: "fi".to_string(),
                region: " Uusimaa ".to_string(),
                asn: Some(12_345),
                network_class: ExitNetworkClass::Residential,
            },
            ip_support: PaidRouteIpSupport::default(),
            rating_discovery: PaidExitRatingDiscoveryConfig {
                file: " ratings.json ".to_string(),
                relays: vec![
                    " wss://ratings-b.example ".to_string(),
                    "wss://ratings-a.example,wss://ratings-b.example".to_string(),
                ],
                scope: " ".to_string(),
            },
        };

        config.normalize();

        assert_eq!(config.pricing.per_units, 1);
        assert_eq!(config.channel.max_channel_capacity_sat, 1);
        assert_eq!(config.channel.channel_expiry_secs, 1);
        assert_eq!(
            config.channel.accepted_mints,
            vec![
                "https://mint.example",
                "https://mint2.example",
                "https://mint3.example"
            ]
        );
        assert_eq!(config.location.country_code, "FI");
        assert_eq!(config.location.region, "Uusimaa");
        assert_eq!(config.rating_discovery.file, "ratings.json");
        assert_eq!(
            config.rating_discovery.relays,
            vec!["wss://ratings-a.example", "wss://ratings-b.example"]
        );
        assert_eq!(
            config.rating_discovery.scope,
            DEFAULT_FIPS_PEER_RATING_SCOPE
        );
    }

    #[test]
    fn paid_route_channel_default_expires_next_day() {
        assert_eq!(PaidRouteChannelTerms::default().channel_expiry_secs, 86_400);
    }

    #[test]
    fn country_claim_status_compares_claimed_and_observed_exit_country() {
        let no_claim = paid_route_country_claim("", Some("FI"));
        assert_eq!(no_claim.status, PaidRouteCountryClaimStatus::NoClaim);
        assert_eq!(no_claim.matches_claim(), None);

        let unknown = paid_route_country_claim("fi", None);
        assert_eq!(unknown.claimed_country_code, "FI");
        assert_eq!(unknown.status, PaidRouteCountryClaimStatus::Unknown);
        assert_eq!(unknown.matches_claim(), None);

        let matched = paid_route_country_claim("fi", Some(" FI "));
        assert_eq!(matched.status, PaidRouteCountryClaimStatus::Match);
        assert_eq!(matched.matches_claim(), Some(true));

        let mismatch = paid_route_country_claim("FI", Some("DE"));
        assert_eq!(mismatch.status, PaidRouteCountryClaimStatus::Mismatch);
        assert_eq!(mismatch.matches_claim(), Some(false));
    }

    #[test]
    fn route_usage_accounting_uses_cashu_service_spilman_policy() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 25,
                per_units: 10,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 100,
                grace_units: 20,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };

        let usage = PaidRouteUsage {
            rx_bytes: 90,
            tx_bytes: 40,
            billable_bytes: 130,
            ..PaidRouteUsage::default()
        };

        assert_eq!(config.amount_due_msat(&usage), 75);
        assert!(config.can_continue_routing(&usage, 25));
        assert!(!config.can_continue_routing(&usage, 24));
    }

    #[test]
    fn route_pricing_prorates_fractional_units_before_rounding() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 25,
                per_units: 10,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 0,
                grace_units: 0,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };

        assert_eq!(config.amount_due_msat(&usage_bytes(1)), 3);
        assert_eq!(config.amount_due_msat(&usage_bytes(10)), 25);
        assert_eq!(config.amount_due_msat(&usage_bytes(11)), 28);

        let grace = config.routing_decision(&usage_bytes(11), 25);
        assert_eq!(grace.amount_due_msat, 28);
        assert_eq!(grace.unpaid_msat, 3);
    }

    #[test]
    fn connection_minimum_is_prorated_and_acts_as_floor() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 100,
                per_units: 10,
                connection_minimum_msat_per_day: 86_400,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 0,
                grace_units: 0,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };

        let idle = PaidRouteUsage {
            active_millis: 1_000,
            ..PaidRouteUsage::default()
        };
        assert_eq!(config.amount_due_msat(&idle), 1);

        let below_floor = PaidRouteUsage {
            active_millis: 1_000,
            billable_bytes: 1,
            ..PaidRouteUsage::default()
        };
        assert_eq!(config.amount_due_msat(&below_floor), 10);

        let above_floor = PaidRouteUsage {
            active_millis: 1_000,
            billable_bytes: 20,
            ..PaidRouteUsage::default()
        };
        assert_eq!(config.amount_due_msat(&above_floor), 200);
    }

    #[test]
    fn connection_minimum_due_can_tolerate_active_time_skew_without_discounting_traffic() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 100,
                per_units: 10,
                connection_minimum_msat_per_day: 86_400,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 0,
                grace_units: 0,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };
        let usage = PaidRouteUsage {
            active_millis: 2_000,
            billable_bytes: 2,
            ..PaidRouteUsage::default()
        };

        assert_eq!(config.amount_due_msat(&usage), 20);
        assert_eq!(
            config.amount_due_msat_with_connection_minimum_skew(&usage, 1_000),
            20
        );

        let idle = PaidRouteUsage {
            active_millis: 2_000,
            ..PaidRouteUsage::default()
        };
        assert_eq!(config.amount_due_msat(&idle), 2);
        assert_eq!(
            config.amount_due_msat_with_connection_minimum_skew(&idle, 1_000),
            1
        );
    }

    #[test]
    fn connection_minimum_participates_in_routing_decision() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 0,
                connection_minimum_msat_per_day: 86_400,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 1_000,
                grace_units: 1_000,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };
        let usage = PaidRouteUsage {
            active_millis: 1_000,
            ..PaidRouteUsage::default()
        };

        let paid = config.routing_decision(&usage, 1);
        assert_eq!(paid.state, PaidRouteAccessState::Paid);
        assert!(paid.allow_routing);
        assert_eq!(paid.amount_due_msat, 1);

        let suspended = config.routing_decision(&usage, 0);
        assert_eq!(suspended.state, PaidRouteAccessState::Suspended);
        assert!(!suspended.allow_routing);
        assert_eq!(suspended.unpaid_msat, 1);
    }

    #[test]
    fn route_decision_reports_free_paid_grace_and_suspended_states() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                price_msat: 25,
                per_units: 10,
                ..PaidRoutePricing::default()
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 100,
                grace_units: 20,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };

        let free = config.routing_decision(&usage_bytes(100), 0);
        assert_eq!(free.state, PaidRouteAccessState::FreeProbe);
        assert!(free.allow_routing);
        assert_eq!(free.amount_due_msat, 0);
        assert_eq!(free.free_probe_remaining_units, 0);

        let paid = config.routing_decision(&usage_bytes(130), 75);
        assert_eq!(paid.state, PaidRouteAccessState::Paid);
        assert!(paid.allow_routing);
        assert_eq!(paid.unpaid_msat, 0);

        let grace = config.routing_decision(&usage_bytes(130), 25);
        assert_eq!(grace.state, PaidRouteAccessState::Grace);
        assert!(grace.allow_routing);
        assert_eq!(grace.amount_due_msat, 75);
        assert_eq!(grace.enforced_amount_due_msat, 25);
        assert_eq!(grace.unpaid_msat, 50);

        let suspended = config.routing_decision(&usage_bytes(130), 24);
        assert_eq!(suspended.state, PaidRouteAccessState::Suspended);
        assert!(!suspended.allow_routing);
        assert_eq!(suspended.unpaid_msat, 51);
    }

    #[test]
    fn session_routing_decision_uses_configured_meter() {
        let config = PaidExitConfig {
            enabled: true,
            pricing: PaidRoutePricing {
                meter: PaidRouteMeter::Packets,
                price_msat: 100,
                per_units: 1,
                connection_minimum_msat_per_day: 0,
            },
            channel: PaidRouteChannelTerms {
                free_probe_units: 0,
                grace_units: 0,
                ..PaidRouteChannelTerms::default()
            },
            ..PaidExitConfig::default()
        };

        let session = PaidRouteSession {
            session_id: "session-1".to_string(),
            lease_id: "lease-1".to_string(),
            usage: PaidRouteUsage {
                rx_bytes: 2_000_000,
                tx_bytes: 1_000_000,
                rx_packets: 2,
                tx_packets: 3,
                billable_packets: 5,
                ..PaidRouteUsage::default()
            },
            payment: PaidRoutePaymentState {
                paid_msat: 500,
                ..PaidRoutePaymentState::default()
            },
            realized_exit_ip: None,
            observed_country_code: None,
            observed_asn: None,
            quality: None,
        };

        let decision = session.routing_decision(&config);

        assert_eq!(decision.state, PaidRouteAccessState::Paid);
        assert_eq!(decision.delivered_units, 5);
        assert_eq!(decision.amount_due_msat, 500);
        assert!(session.can_continue_routing(&config));
    }

    #[test]
    fn offer_json_does_not_publish_raw_exit_ip() {
        let offer = PaidRouteOffer {
            offer_id: "offer-1".to_string(),
            seller_npub: "npub1seller".to_string(),
            receiver_pubkey_hex: String::new(),
            service: PaidRouteServiceKind::InternetExit,
            access: PaidRouteAccessPolicy {
                upstream: PaidExitUpstream::WireGuardExit,
                private_vpn_access: PaidRoutePrivateVpnAccess::Denied,
            },
            pricing: PaidRoutePricing::default(),
            channel: PaidRouteChannelTerms::default(),
            location: PaidRouteLocationHint {
                country_code: "FI".to_string(),
                network_class: ExitNetworkClass::Satellite,
                ..PaidRouteLocationHint::default()
            },
            ip_support: PaidRouteIpSupport::default(),
            quality: None,
        };

        let json = serde_json::to_string(&offer).expect("serialize offer");
        assert!(json.contains("country_code"));
        assert!(json.contains("satellite"));
        assert!(!json.contains("public_ip"));
        assert!(!json.contains("publicIp"));
        assert!(!json.contains("realized_exit_ip"));
        assert!(json.contains("wireguard_exit"));
        assert!(json.contains("denied"));
        assert!(!json.contains("private_routes"));
    }

    #[test]
    fn enum_parsers_accept_user_friendly_spellings() {
        assert_eq!(
            "community-mesh".parse::<ExitNetworkClass>(),
            Ok(ExitNetworkClass::CommunityMesh)
        );
        assert_eq!(
            "ms".parse::<PaidRouteMeter>(),
            Ok(PaidRouteMeter::Milliseconds)
        );
        assert_eq!(
            "wg".parse::<PaidExitUpstream>(),
            Ok(PaidExitUpstream::WireGuardExit)
        );
    }

    #[test]
    fn signed_offer_event_roundtrips_without_raw_exit_endpoint() {
        let seller = Keys::generate();
        let offer = sample_paid_exit_offer(&seller);

        let signed =
            SignedPaidRouteOffer::sign(offer.clone(), &seller, 123).expect("sign paid route offer");

        assert_eq!(u16::from(signed.event.kind), PAID_ROUTE_OFFER_KIND);
        assert_eq!(signed.event.created_at.as_secs(), 123);
        assert_eq!(signed.offer().expect("decode offer"), offer);
        SignedPaidRouteOffer::from_event(signed.event.clone()).expect("verify signed offer");

        let tags = signed
            .event
            .tags
            .iter()
            .map(Tag::as_slice)
            .collect::<Vec<_>>();
        assert!(tags.contains(&vec!["d".to_string(), "paid-exit-fi".to_string()].as_slice()));
        assert!(
            tags.contains(&vec!["app".to_string(), PAID_ROUTE_OFFER_APP.to_string()].as_slice())
        );
        assert!(
            tags.contains(&vec!["v".to_string(), PAID_ROUTE_OFFER_VERSION.to_string()].as_slice())
        );
        assert!(
            tags.contains(&vec!["service".to_string(), "internet_exit".to_string()].as_slice())
        );
        assert!(
            tags.contains(&vec!["payment".to_string(), "cashu_spilman".to_string()].as_slice())
        );
        assert!(tags.contains(&vec!["meter".to_string(), "bytes".to_string()].as_slice()));
        assert!(tags.contains(&vec!["price_msat".to_string(), "2500".to_string()].as_slice()));
        assert!(tags.contains(&vec!["per_units".to_string(), "1000000".to_string()].as_slice()));
        assert!(
            tags.contains(
                &vec![
                    "connection_minimum_msat_per_day".to_string(),
                    "86400".to_string()
                ]
                .as_slice()
            )
        );
        assert!(
            tags.contains(
                &vec!["max_channel_capacity_sat".to_string(), "100".to_string()].as_slice()
            )
        );
        assert!(
            tags.contains(&vec!["channel_expiry_secs".to_string(), "600".to_string()].as_slice())
        );
        assert!(
            tags.contains(&vec!["free_probe_units".to_string(), "1048576".to_string()].as_slice())
        );
        assert!(tags.contains(&vec!["grace_units".to_string(), "262144".to_string()].as_slice()));
        assert!(
            tags.contains(&vec!["upstream".to_string(), "wireguard_exit".to_string()].as_slice())
        );
        assert!(
            tags.contains(&vec!["private_vpn_access".to_string(), "denied".to_string()].as_slice())
        );
        assert!(tags.contains(&vec!["country".to_string(), "FI".to_string()].as_slice()));
        assert!(
            tags.contains(&vec!["network_class".to_string(), "satellite".to_string()].as_slice())
        );
        assert!(tags.contains(&vec!["ip".to_string(), "ipv4".to_string()].as_slice()));
        assert!(
            tags.contains(
                &vec![
                    "mint".to_string(),
                    "https://mint.minibits.cash/Bitcoin".to_string()
                ]
                .as_slice()
            )
        );
        assert!(tags.contains(&vec!["latency_ms".to_string(), "42".to_string()].as_slice()));
        assert!(tags.contains(&vec!["jitter_ms".to_string(), "7".to_string()].as_slice()));
        assert!(tags.contains(&vec!["packet_loss_ppm".to_string(), "500".to_string()].as_slice()));
        assert!(tags.contains(&vec!["down_bps".to_string(), "25000000".to_string()].as_slice()));
        assert!(tags.contains(&vec!["up_bps".to_string(), "5000000".to_string()].as_slice()));
        assert!(tags.contains(&vec!["uptime_secs".to_string(), "3600".to_string()].as_slice()));
        assert!(tags.contains(&vec!["last_seen_unix".to_string(), "123".to_string()].as_slice()));

        let content = &signed.event.content;
        assert!(!content.contains("public_ip"));
        assert!(!content.contains("publicIp"));
        assert!(!content.contains("realized_exit_ip"));
        assert!(!content.contains("203.0.113."));
        assert!(!content.contains("private_routes"));
    }

    #[test]
    fn signed_offer_event_includes_spilman_receiver_pubkey_when_present() {
        let seller = Keys::generate();
        let receiver_pubkey_hex = format!("03{}", "11".repeat(32));
        let signed = signed_paid_exit_offer_from_config_with_receiver(
            "paid-exit-fi",
            &seller,
            &sample_paid_exit_config(),
            Some(&receiver_pubkey_hex),
            None,
            123,
        )
        .expect("sign paid route offer with receiver key");
        let offer = signed.offer().expect("decode offer");

        assert_eq!(offer.receiver_pubkey_hex, receiver_pubkey_hex);
        assert!(signed.event.tags.iter().any(|tag| {
            tag.as_slice()
                == ["receiver_pubkey".to_string(), receiver_pubkey_hex.clone()].as_slice()
        }));
        SignedPaidRouteOffer::from_event(signed.event).expect("verify receiver-key offer");
    }

    #[test]
    fn signed_offer_rejects_seller_that_does_not_match_signer() {
        let seller = Keys::generate();
        let signer = Keys::generate();
        let offer = sample_paid_exit_offer(&seller);
        let event = EventBuilder::new(
            Kind::Custom(PAID_ROUTE_OFFER_KIND),
            serde_json::to_string(&offer).expect("encode offer"),
        )
        .tags(paid_route_offer_tags(&offer).expect("offer tags"))
        .custom_created_at(Timestamp::from(123))
        .sign_with_keys(&signer)
        .expect("sign offer with mismatched key");

        let error = SignedPaidRouteOffer::from_event(event).expect_err("seller mismatch rejected");

        assert!(error.to_string().contains("seller"));
    }

    #[test]
    fn signed_offer_rejects_private_vpn_access_tag_claims() {
        let seller = Keys::generate();
        let offer = sample_paid_exit_offer(&seller);
        let mut tags = paid_route_offer_tags(&offer).expect("offer tags");
        let private_access_index = tags
            .iter()
            .position(|tag| {
                tag.as_slice()
                    .first()
                    .is_some_and(|kind| kind == "private_vpn_access")
            })
            .expect("private access tag");
        tags[private_access_index] =
            paid_route_tag(&["private_vpn_access", "allowed"]).expect("bad access tag");
        let event = EventBuilder::new(
            Kind::Custom(PAID_ROUTE_OFFER_KIND),
            serde_json::to_string(&offer).expect("encode offer"),
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(123))
        .sign_with_keys(&seller)
        .expect("sign tampered offer");

        let error = SignedPaidRouteOffer::from_event(event).expect_err("access claim rejected");

        assert!(error.to_string().contains("private VPN access"));
    }

    #[test]
    fn signed_offer_builder_requires_enabled_paid_exit_with_mint_for_nonzero_price() {
        let seller = Keys::generate();
        let mut config = sample_paid_exit_config();
        config.enabled = false;

        let error = signed_paid_exit_offer_from_config("paid-exit-fi", &seller, &config, None, 123)
            .expect_err("disabled seller rejected");
        assert!(error.to_string().contains("disabled"));

        config.enabled = true;
        config.channel.accepted_mints.clear();
        let error = signed_paid_exit_offer_from_config("paid-exit-fi", &seller, &config, None, 123)
            .expect_err("priced offer without mint rejected");
        assert!(error.to_string().contains("mint"));

        config.pricing.price_msat = 0;
        config.pricing.connection_minimum_msat_per_day = 0;
        signed_paid_exit_offer_from_config("paid-exit-fi", &seller, &config, None, 123)
            .expect("free dev offer can omit mints");
    }

    #[test]
    fn paid_route_offer_filter_targets_offer_kind() {
        let filter = paid_route_offer_filter(25, Some(100));
        let json = serde_json::to_value(&filter).expect("filter json");

        assert_eq!(json["kinds"], serde_json::json!([PAID_ROUTE_OFFER_KIND]));
        assert_eq!(json["limit"], 25);
        assert_eq!(json["since"], 100);
    }

    #[test]
    fn paid_route_payment_filter_targets_recipient_gift_wraps() {
        let seller = Keys::generate();
        let filter = paid_route_payment_filter(seller.public_key(), 25, Some(100));
        let json = serde_json::to_value(&filter).expect("filter json");

        assert_eq!(
            json["kinds"],
            serde_json::json!([u16::from(Kind::GiftWrap)])
        );
        assert_eq!(
            json["#p"],
            serde_json::json!([seller.public_key().to_hex()])
        );
        assert_eq!(json["limit"], 25);
        assert_eq!(json["since"], 100);
    }

    #[cfg(feature = "paid-exit")]
    #[tokio::test]
    async fn paid_route_payment_gift_wrap_roundtrips_envelope() {
        let buyer = Keys::generate();
        let seller = Keys::generate();
        let envelope = sample_paid_route_payment_envelope(&buyer, &seller);

        let event = gift_wrap_paid_route_payment(&envelope, &buyer)
            .await
            .expect("gift-wrap payment");

        assert_eq!(event.kind, Kind::GiftWrap);
        assert!(!event.content.contains("lease-1"));
        assert!(!event.content.contains("channel-1"));

        let unwrapped = unwrap_paid_route_payment(&event, &seller)
            .await
            .expect("unwrap payment");

        assert_eq!(unwrapped, envelope);
    }

    #[cfg(feature = "paid-exit")]
    #[tokio::test]
    async fn paid_route_payment_gift_wrap_rejects_wrong_sender() {
        let buyer = Keys::generate();
        let seller = Keys::generate();
        let envelope = sample_paid_route_payment_envelope(&buyer, &seller);

        let error = gift_wrap_paid_route_payment(&envelope, &seller)
            .await
            .expect_err("seller cannot send buyer payment envelope");

        assert!(error.to_string().contains("buyer"));
    }

    fn sample_paid_exit_offer(seller: &Keys) -> PaidRouteOffer {
        let config = sample_paid_exit_config();
        PaidRouteOffer::from_paid_exit_config(
            "paid-exit-fi",
            seller.public_key().to_bech32().expect("seller npub"),
            &config,
            Some(PaidRouteQualityMetrics {
                latency_ms: Some(42),
                jitter_ms: Some(7),
                packet_loss_ppm: Some(500),
                down_bps: Some(25_000_000),
                up_bps: Some(5_000_000),
                uptime_secs: Some(3600),
                last_seen_unix: Some(123),
            }),
        )
    }

    fn sample_paid_exit_config() -> PaidExitConfig {
        PaidExitConfig {
            enabled: true,
            access: PaidRouteAccessPolicy {
                upstream: PaidExitUpstream::WireGuardExit,
                private_vpn_access: PaidRoutePrivateVpnAccess::Denied,
            },
            pricing: PaidRoutePricing {
                meter: PaidRouteMeter::Bytes,
                price_msat: 2500,
                per_units: 1_000_000,
                connection_minimum_msat_per_day: 86_400,
            },
            channel: PaidRouteChannelTerms {
                accepted_mints: vec!["https://mint.minibits.cash/Bitcoin".to_string()],
                max_channel_capacity_sat: 100,
                channel_expiry_secs: 600,
                free_probe_units: 1_048_576,
                grace_units: 262_144,
            },
            location: PaidRouteLocationHint {
                country_code: "FI".to_string(),
                region: "Uusimaa".to_string(),
                asn: Some(14593),
                network_class: ExitNetworkClass::Satellite,
            },
            ip_support: PaidRouteIpSupport {
                ipv4: true,
                ipv6: false,
            },
            rating_discovery: PaidExitRatingDiscoveryConfig::default(),
        }
    }

    #[cfg(feature = "paid-exit")]
    fn sample_paid_route_payment_envelope(
        buyer: &Keys,
        seller: &Keys,
    ) -> StreamingRoutePaymentEnvelope {
        StreamingRoutePaymentEnvelope::new(
            "internet-exit",
            "lease-1",
            buyer.public_key().to_bech32().expect("buyer npub"),
            seller.public_key().to_bech32().expect("seller npub"),
            123,
            cashu_service::StreamingRoutePaymentPayload::BalanceUpdate(
                cashu_service::StreamingRouteBalanceUpdate {
                    delivered_units: 2048,
                    amount_due_msat: 500,
                    paid_msat: 1000,
                    payment: CashuSpilmanPayment {
                        channel_id: "channel-1".to_string(),
                        balance: 1,
                        signature: "sig".to_string(),
                        params: None,
                        funding_proofs: None,
                    },
                },
            ),
        )
    }

    fn usage_bytes(bytes: u64) -> PaidRouteUsage {
        PaidRouteUsage {
            rx_bytes: bytes,
            billable_bytes: bytes,
            ..PaidRouteUsage::default()
        }
    }
}
