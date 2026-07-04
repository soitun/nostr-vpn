#[cfg(feature = "paid-exit")]
mod paid_exit {
    use std::collections::HashSet;
    use std::io::Write as _;
    use std::process::Stdio;

    use nostr_sdk::prelude::ToBech32;
    use nostr_vpn_core::paid_route_store::{
        OpenPaidRouteBuyerSessionRequest, PaidRouteChannelRecord, PaidRouteChannelRole,
        PaidRouteLifecycleStatus, PaidRouteSellerCollectionState, PaidRouteStore,
        PaidRouteWalletState, UpdatePaidRouteSessionProbeRequest, load_paid_route_store,
        paid_route_store_file_path, write_paid_route_store,
    };
    use nostr_vpn_core::paid_routes::{
        ExitNetworkClass, PaidExitConfig, PaidExitUpstream, PaidRouteAccessState,
        PaidRouteCountryClaim, PaidRouteMeter, PaidRouteOffer, PaidRouteQualityMetrics,
        PaidRouteRoutingDecision, paid_route_country_claim,
    };
    use serde_json::json;

    use crate::native_state::{
        NativePaidRouteChannelState, NativePaidRouteOfferState, NativePaidRouteSessionState,
        NativePaidRouteWalletMintState,
    };

    use super::*;

    include!("paid_exit_actions.rs");
    include!("paid_exit_state.rs");
    include!("paid_exit_text.rs");
    include!("paid_exit_json.rs");
}

#[cfg(not(feature = "paid-exit"))]
impl NativeAppRuntime {
    fn paid_exit_seller_state(
        &self,
        _app: Option<&AppConfig>,
        _port_mapping: Option<&PortMappingStatus>,
        _mobile: bool,
    ) -> NativePaidExitSellerState {
        NativePaidExitSellerState {
            supported: false,
            status_text: "Paid exit support was not built into this app".to_string(),
            ..NativePaidExitSellerState::default()
        }
    }

    fn paid_route_market_state(&self, _app: Option<&AppConfig>) -> NativePaidRouteMarketState {
        NativePaidRouteMarketState {
            supported: false,
            status_text: "Paid exit support was not built into this app".to_string(),
            wallet: NativePaidRouteWalletState {
                last_action: self.paid_route_wallet_last_action.clone(),
                ..NativePaidRouteWalletState::default()
            },
            last_payment_action: self.paid_route_payment_last_action.clone(),
            filter: self.paid_route_market_filter.clone(),
            ..NativePaidRouteMarketState::default()
        }
    }

    fn add_paid_route_wallet_mint(&mut self, _url: &str, _label: Option<&str>) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn remove_paid_route_wallet_mint(&mut self, _url: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn set_paid_route_default_mint(&mut self, _url: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn refresh_paid_route_wallet(&mut self, _refresh: bool) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn top_up_paid_route_wallet(
        &mut self,
        _mint_url: Option<&str>,
        _amount_sat: u64,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn receive_paid_route_wallet_token(&mut self, _token: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn send_paid_route_wallet_token(
        &mut self,
        _mint_url: Option<&str>,
        _amount_sat: u64,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn withdraw_paid_route_wallet_lightning(
        &mut self,
        _mint_url: Option<&str>,
        _invoice: &str,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn buy_paid_route_offer(
        &mut self,
        _offer_key: &str,
        _mint_url: Option<&str>,
        _channel_capacity_sat: Option<u64>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn buy_best_paid_route_offer(
        &mut self,
        _mint_url: Option<&str>,
        _channel_capacity_sat: Option<u64>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn select_paid_route_session(&mut self, _session_id: &str, _connect: bool) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn probe_paid_route_session(&mut self, _session_id: &str, _timeout_secs: u64) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_paid_route_probe(
        &mut self,
        _session_id: &str,
        _realized_exit_ip: Option<&str>,
        _observed_country_code: Option<&str>,
        _observed_asn: Option<u32>,
        _latency_ms: Option<u32>,
        _jitter_ms: Option<u32>,
        _packet_loss_ppm: Option<u32>,
        _down_bps: Option<u64>,
        _up_bps: Option<u64>,
        _uptime_secs: Option<u64>,
        _last_seen_unix: Option<u64>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn create_paid_route_payment_envelope(
        &mut self,
        _session_id: &str,
        _kind: &str,
        _payment_json: &str,
        _delivered_units: Option<u64>,
        _paid_msat: Option<u64>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn open_paid_route_channel_from_wallet(
        &mut self,
        _session_id: &str,
        _mint_url: Option<&str>,
        _paid_msat: Option<u64>,
        _max_amount_per_output: Option<u64>,
        _keyset_id: Option<&str>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn sign_paid_route_payment_envelope_from_wallet(
        &mut self,
        _session_id: &str,
        _kind: &str,
        _delivered_units: Option<u64>,
        _paid_msat: Option<u64>,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn close_paid_route_channel_from_wallet(
        &mut self,
        _session_id: &str,
        _publish: bool,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn apply_paid_route_payment_envelope(&mut self, _envelope_json: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn send_paid_route_payment_envelope(&mut self, _envelope_json: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn stream_paid_route_payments(
        &mut self,
        _publish: bool,
        _min_increment_msat: u64,
        _limit: u64,
    ) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn receive_paid_route_payments(&mut self, _duration_secs: u64) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn collect_paid_exit_channel(&mut self, _channel_id: &str) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn collect_due_paid_exit_channels(&mut self) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn publish_paid_exit_offer(&mut self) -> Result<()> {
        Err(paid_exit_not_built_error())
    }

    fn discover_paid_route_offers(&mut self, _duration_secs: u64) -> Result<()> {
        Err(paid_exit_not_built_error())
    }
}

#[cfg(not(feature = "paid-exit"))]
fn paid_exit_not_built_error() -> anyhow::Error {
    anyhow!("paid exit support was not built into this app")
}
