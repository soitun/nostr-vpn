use super::*;
use nostr_vpn_core::paid_routes::PaidRouteUsage;

const PAID_EXIT_AUTO_HEALTH_TTL_SECS: u64 = 15;
const PAID_EXIT_AUTO_PROBE_TIMEOUT_SECS: u64 = 30;
const PAID_EXIT_AUTO_FAILOVER_SECS: u64 = 60;

async fn paid_exit_automatic_probe_measurement(
    app: &AppConfig,
    now_unix: u64,
) -> Result<PaidRouteProbeMeasurement> {
    let args = PaidExitProbeArgs {
        config: None,
        session: String::new(),
        ip_url: None,
        stun_servers: Vec::new(),
        no_stun: false,
        geoip_url_template: None,
        no_geoip: true,
        download_url: None,
        upload_url: None,
        bandwidth_bytes: DEFAULT_PAID_ROUTE_BANDWIDTH_BYTES,
        no_bandwidth: false,
        samples: 1,
        timeout_secs: 5,
        no_reload_daemon: true,
        json: false,
    };
    let (measurement, _, bandwidth_error) =
        paid_exit_probe_measurement(&args, app, now_unix).await?;
    if let Some(error) = bandwidth_error {
        return Err(anyhow!("paid exit free probe failed: {error}"));
    }
    if measurement.quality.down_bps.is_none() || measurement.quality.up_bps.is_none() {
        return Err(anyhow!(
            "paid exit free probe did not verify both traffic directions"
        ));
    }
    Ok(measurement)
}

#[derive(Default)]
pub(crate) struct PaidExitAutomaticBuyer {
    generation: u64,
    candidate: Option<PaidExitAutomaticCandidate>,
    rejected_offers: HashSet<String>,
    probe: Option<PaidExitAutomaticProbe>,
}

struct PaidExitAutomaticProbe {
    generation: u64,
    task: tokio::task::JoinHandle<Result<PaidRouteProbeMeasurement>>,
}

struct PaidExitAutomaticCandidate {
    selection: nostr_vpn_core::paid_route_store::PaidRouteAutomaticOfferSelection,
    seller_pubkey: String,
    session_id: String,
    selected_at: u64,
    probe_started_at: Option<u64>,
    probe_succeeded: bool,
    funding_attempted: bool,
    funded: bool,
    last_authenticated_at: Option<u64>,
    last_tx_at: Option<u64>,
    last_rx_at: Option<u64>,
    last_healthy_at: Option<u64>,
    failed: bool,
}

#[derive(Default)]
pub(crate) struct PaidExitUsageFlush {
    pub(crate) seller_admission_changed: bool,
    pub(crate) buyer_delta: PaidRouteUsage,
}

impl PaidExitAutomaticCandidate {
    fn observe_presence(&mut self, statuses: &[MeshPeerStatus], now_unix: u64) {
        let authenticated = statuses.iter().any(|status| {
            status.connected
                && normalize_nostr_pubkey(&status.pubkey).ok().as_deref()
                    == Some(self.seller_pubkey.as_str())
                && status.last_seen_at.is_some_and(|seen| {
                    now_unix.saturating_sub(seen) <= PAID_EXIT_AUTO_HEALTH_TTL_SECS
                })
        });
        if authenticated {
            self.last_authenticated_at = Some(now_unix);
        }
    }

    fn observe_usage(&mut self, delta: &PaidRouteUsage, now_unix: u64) {
        if delta.tx_bytes > 0 {
            self.last_tx_at = Some(now_unix);
        }
        if delta.rx_bytes > 0 {
            self.last_rx_at = Some(now_unix);
        }
        if self.health_evidence_fresh(now_unix) {
            self.last_healthy_at = Some(now_unix);
        }
    }

    fn health_evidence_fresh(&self, now_unix: u64) -> bool {
        self.probe_succeeded
            && [self.last_authenticated_at, self.last_tx_at, self.last_rx_at]
                .into_iter()
                .all(|observed| {
                    observed.is_some_and(|observed| {
                        now_unix.saturating_sub(observed) <= PAID_EXIT_AUTO_HEALTH_TTL_SECS
                    })
                })
    }

    fn should_failover(&self, now_unix: u64) -> bool {
        if self.failed {
            return true;
        }
        if !self.probe_succeeded
            && now_unix.saturating_sub(self.probe_started_at.unwrap_or(self.selected_at))
                >= PAID_EXIT_AUTO_PROBE_TIMEOUT_SECS
        {
            return true;
        }
        self.funded
            && self.last_healthy_at.is_some_and(|healthy| {
                now_unix.saturating_sub(healthy) >= PAID_EXIT_AUTO_FAILOVER_SECS
            })
    }
}

impl PaidExitAutomaticBuyer {
    pub(crate) fn enabled(app: &AppConfig) -> bool {
        app.internet_source == nostr_vpn_core::config::InternetSource::PaidAutomatic
    }

    pub(crate) fn payments_allowed(&self, app: &AppConfig, now_unix: u64) -> bool {
        if !Self::enabled(app) {
            return true;
        }
        self.candidate
            .as_ref()
            .is_some_and(|candidate| candidate.funded && candidate.health_evidence_fresh(now_unix))
    }

    pub(crate) fn cancel_if_disabled(&mut self, app: &AppConfig) {
        if Self::enabled(app) {
            return;
        }
        if let Some(probe) = self.probe.take() {
            probe.task.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        self.candidate = None;
        self.rejected_offers.clear();
    }

    fn selection(
        &self,
        store: &PaidRouteStore,
        now_unix: u64,
    ) -> Result<nostr_vpn_core::paid_route_store::PaidRouteAutomaticOfferSelection> {
        let mut candidates = store.clone();
        for offer in &self.rejected_offers {
            candidates.offers.remove(offer);
        }
        candidates.select_automatic_offer(now_unix)
    }

    fn start_candidate(
        &mut self,
        selection: nostr_vpn_core::paid_route_store::PaidRouteAutomaticOfferSelection,
        seller_pubkey: String,
        session_id: String,
        funded: bool,
        now_unix: u64,
    ) {
        self.generation = self.generation.wrapping_add(1);
        self.candidate = Some(PaidExitAutomaticCandidate {
            selection,
            seller_pubkey,
            session_id,
            selected_at: now_unix,
            probe_started_at: None,
            probe_succeeded: false,
            funding_attempted: funded,
            funded,
            last_authenticated_at: None,
            last_tx_at: None,
            last_rx_at: None,
            last_healthy_at: None,
            failed: false,
        });
    }

    fn cancel_candidate(&mut self, reject: bool) {
        if let Some(probe) = self.probe.take() {
            probe.task.abort();
        }
        if reject && let Some(candidate) = self.candidate.as_ref() {
            self.rejected_offers
                .insert(candidate.selection.offer_key.clone());
        }
        self.generation = self.generation.wrapping_add(1);
        self.candidate = None;
    }
}

pub(crate) fn reconcile_automatic_paid_exit_selection(
    automatic: &mut PaidExitAutomaticBuyer,
    app: &mut AppConfig,
    config_path: &Path,
    now_unix: u64,
) -> Result<bool> {
    automatic.cancel_if_disabled(app);
    if !PaidExitAutomaticBuyer::enabled(app) {
        return Ok(false);
    }

    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let selection = match automatic.selection(&store, now_unix) {
        Ok(selection) => selection,
        Err(_) => {
            if let Some(candidate) = automatic.candidate.as_mut() {
                candidate.failed = true;
            }
            return Ok(false);
        }
    };
    if let Some(candidate) = automatic.candidate.as_mut() {
        if candidate.selection != selection {
            candidate.failed = true;
        }
        return Ok(false);
    }

    if let Some((seller_npub, seller_pubkey, session_id, funded)) =
        recover_automatic_paid_exit_session(&store, &selection, now_unix)
    {
        let route_changed =
            app.public_paid_exit_node_pubkey_hex().as_deref() != Some(seller_pubkey.as_str());
        app.select_public_paid_exit_node(&seller_npub)?;
        if !PaidExitAutomaticBuyer::enabled(app) {
            return Err(anyhow!(
                "automatic paid exit recovery changed internet mode"
            ));
        }
        if route_changed {
            app.save(config_path)?;
        }
        automatic.start_candidate(selection, seller_pubkey, session_id, funded, now_unix);
        return Ok(route_changed);
    }

    let buyer_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode automatic paid exit buyer npub")?;
    let session = store.open_buyer_session(OpenPaidRouteBuyerSessionRequest {
        offer_selector: selection.offer_key.clone(),
        buyer_npub,
        mint_url: Some(selection.mint_url.clone()),
        channel_capacity_sat: Some(selection.channel_capacity_sat),
        initial_paid_msat: 0,
        now_unix,
    })?;
    let seller_pubkey = normalize_nostr_pubkey(&session.seller_npub)
        .context("invalid automatically selected paid exit seller")?;
    if !PaidExitAutomaticBuyer::enabled(app) {
        return Ok(false);
    }
    app.select_public_paid_exit_node(&session.seller_npub)?;
    if !PaidExitAutomaticBuyer::enabled(app) {
        return Err(anyhow!(
            "automatic paid exit selection changed internet mode"
        ));
    }
    if session.changed {
        write_paid_route_store(&store_path, &store)?;
    }
    app.save(config_path)?;
    automatic.start_candidate(
        selection,
        seller_pubkey,
        session.session_id,
        false,
        now_unix,
    );
    Ok(true)
}

fn recover_automatic_paid_exit_session(
    store: &PaidRouteStore,
    selection: &nostr_vpn_core::paid_route_store::PaidRouteAutomaticOfferSelection,
    now_unix: u64,
) -> Option<(String, String, String, bool)> {
    let offer = &store.offers.get(&selection.offer_key)?.offer;
    let seller_pubkey = normalize_nostr_pubkey(&offer.seller_npub).ok()?;
    store
        .sessions
        .values()
        .filter_map(|session| {
            let channel = store.channels.get(&session.session.payment.channel_id)?;
            let lease = store.leases.get(&session.session.lease_id)?;
            (channel.role == PaidRouteChannelRole::Buyer
                && channel.offer_id == offer.offer_id
                && channel.counterparty_npub == offer.seller_npub
                && channel.expires_at_unix > now_unix
                && lease.lease.expires_at_unix > now_unix
                && matches!(
                    lease.status,
                    PaidRouteLifecycleStatus::Opening
                        | PaidRouteLifecycleStatus::Probing
                        | PaidRouteLifecycleStatus::Active
                        | PaidRouteLifecycleStatus::Paused
                )
                && matches!(
                    channel.status,
                    PaidRouteLifecycleStatus::Opening
                        | PaidRouteLifecycleStatus::Probing
                        | PaidRouteLifecycleStatus::Active
                        | PaidRouteLifecycleStatus::Paused
                ))
            .then_some((
                session.updated_at_unix,
                session.session.session_id.clone(),
                session.session.payment.cashu_spilman_payment.is_some(),
            ))
        })
        .max_by_key(|candidate| candidate.0)
        .map(|(_, session_id, funded)| {
            (offer.seller_npub.clone(), seller_pubkey, session_id, funded)
        })
}

pub(crate) async fn update_automatic_paid_exit(
    automatic: &mut PaidExitAutomaticBuyer,
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &mut AppConfig,
    config_path: &Path,
    buyer_delta: &PaidRouteUsage,
    now_unix: u64,
) -> Result<bool> {
    automatic.cancel_if_disabled(app);
    if !PaidExitAutomaticBuyer::enabled(app) {
        return Ok(false);
    }

    if let Some(candidate) = automatic.candidate.as_mut() {
        candidate.observe_presence(&runtime.peer_statuses(), now_unix);
        candidate.observe_usage(buyer_delta, now_unix);
    }

    if automatic.probe.is_none()
        && automatic.candidate.as_ref().is_some_and(|candidate| {
            candidate.probe_started_at.is_none()
                && candidate.last_authenticated_at.is_some_and(|observed| {
                    now_unix.saturating_sub(observed) <= PAID_EXIT_AUTO_HEALTH_TTL_SECS
                })
        })
    {
        let probe_app = app.clone();
        if let Some(candidate) = automatic.candidate.as_mut() {
            candidate.probe_started_at = Some(now_unix);
            candidate.last_tx_at = None;
            candidate.last_rx_at = None;
        }
        automatic.probe = Some(PaidExitAutomaticProbe {
            generation: automatic.generation,
            task: tokio::spawn(async move {
                paid_exit_automatic_probe_measurement(&probe_app, now_unix).await
            }),
        });
    }

    if automatic
        .probe
        .as_ref()
        .is_some_and(|probe| probe.task.is_finished())
    {
        let probe = automatic.probe.take().expect("finished probe exists");
        let result = probe
            .task
            .await
            .map_err(|error| anyhow!("automatic paid exit probe task failed: {error}"))?;
        if probe.generation == automatic.generation {
            match result {
                Ok(measurement) => {
                    let session_id = automatic
                        .candidate
                        .as_ref()
                        .map(|candidate| candidate.session_id.clone())
                        .ok_or_else(|| anyhow!("automatic paid exit probe lost its candidate"))?;
                    record_automatic_paid_exit_probe(
                        config_path,
                        &session_id,
                        measurement,
                        now_unix,
                    )?;
                    if let Some(candidate) = automatic.candidate.as_mut() {
                        candidate.probe_succeeded = true;
                        if candidate.health_evidence_fresh(now_unix) {
                            candidate.last_healthy_at = Some(now_unix);
                        }
                    }
                }
                Err(error) => {
                    eprintln!("paid-exit: automatic free probe failed: {error}");
                    if let Some(candidate) = automatic.candidate.as_mut() {
                        candidate.failed = true;
                    }
                }
            }
        }
    }

    let fund = automatic.candidate.as_ref().is_some_and(|candidate| {
        !candidate.failed
            && !candidate.funding_attempted
            && candidate.health_evidence_fresh(now_unix)
    });
    if fund {
        let session_id = automatic
            .candidate
            .as_ref()
            .map(|candidate| candidate.session_id.clone())
            .expect("funding candidate exists");
        if let Some(candidate) = automatic.candidate.as_mut() {
            candidate.funding_attempted = true;
        }
        match fund_automatic_paid_exit(app, config_path, &session_id, now_unix).await {
            Ok(envelope) => {
                if let Some(candidate) = automatic.candidate.as_mut() {
                    candidate.funded = true;
                    candidate.last_healthy_at = Some(now_unix);
                }
                if let Err(error) = queue_paid_exit_payment(app, config_path, &envelope) {
                    eprintln!("paid-exit: automatic channel-open queue failed: {error}");
                    if let Some(candidate) = automatic.candidate.as_mut() {
                        candidate.failed = true;
                    }
                }
            }
            Err(error) => {
                eprintln!("paid-exit: automatic funding failed: {error}");
                if let Some(candidate) = automatic.candidate.as_mut() {
                    candidate.failed = true;
                }
            }
        }
    }

    if automatic
        .candidate
        .as_ref()
        .is_some_and(|candidate| candidate.should_failover(now_unix))
    {
        finalize_automatic_paid_exit(automatic, runtime, app, config_path, now_unix).await?;
        if !PaidExitAutomaticBuyer::enabled(app) {
            return Ok(false);
        }
        app.set_internet_source(nostr_vpn_core::config::InternetSource::PaidAutomatic);
        app.save(config_path)?;
        automatic.cancel_candidate(true);
        return Ok(true);
    }

    Ok(false)
}

fn record_automatic_paid_exit_probe(
    config_path: &Path,
    session_id: &str,
    measurement: PaidRouteProbeMeasurement,
    now_unix: u64,
) -> Result<()> {
    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let result = store.update_session_probe(UpdatePaidRouteSessionProbeRequest {
        session_id: session_id.to_string(),
        realized_exit_ip: measurement.realized_exit_ip,
        observed_country_code: measurement.observed_country_code,
        observed_asn: measurement.observed_asn,
        quality: Some(measurement.quality),
        now_unix,
    })?;
    if result.changed {
        write_paid_route_store(&store_path, &store)?;
    }
    Ok(())
}

async fn fund_automatic_paid_exit(
    app: &AppConfig,
    config_path: &Path,
    session_id: &str,
    now_unix: u64,
) -> Result<StreamingRoutePaymentEnvelope> {
    if !PaidExitAutomaticBuyer::enabled(app) {
        return Err(anyhow!(
            "automatic paid exit funding cancelled by internet mode"
        ));
    }
    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let session = store
        .sessions
        .get(session_id)
        .cloned()
        .ok_or_else(|| anyhow!("automatic paid exit session {session_id} does not exist"))?;
    let lease = store
        .leases
        .get(&session.session.lease_id)
        .cloned()
        .ok_or_else(|| anyhow!("automatic paid exit session has no lease"))?;
    let channel = store
        .channels
        .get(&session.session.payment.channel_id)
        .cloned()
        .ok_or_else(|| anyhow!("automatic paid exit session has no channel"))?;
    let quote = store
        .quotes
        .get(&lease.lease.quote_id)
        .cloned()
        .ok_or_else(|| anyhow!("automatic paid exit session has no quote"))?;
    let opened = open_streaming_route_cashu_spilman_channel_from_wallet(
        &paid_exit_wallet_data_dir(config_path),
        StreamingRouteOpenCashuSpilmanChannelFromWalletRequest {
            mint_url: channel.mint_url,
            receiver_pubkey_hex: quote.quote.receiver_pubkey_hex,
            capacity_sat: session.session.payment.capacity_sat,
            expiry_unix: channel.expires_at_unix,
            max_amount_per_output: 0,
            unit: "sat".to_string(),
            opening_paid_msat: 0,
            keyset_id: None,
            keyset_info_json: None,
        },
    )
    .await?;
    store.attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
        session_id: session_id.to_string(),
        channel_id: opened.channel.channel_id.clone(),
        cashu_unit: opened.channel.unit.clone(),
        capacity_sat: opened.channel.capacity_sat,
        paid_msat: Some(opened.channel.opening_paid_msat),
        payment: opened.channel.payment.clone(),
        now_unix,
    })?;
    let buyer_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode automatic paid exit buyer npub")?;
    let payment =
        store.build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
            session_id: session_id.to_string(),
            buyer_npub,
            kind: BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen,
            payment: opened.channel.payment,
            delivered_units: None,
            paid_msat: Some(opened.channel.opening_paid_msat),
            now_unix,
        })?;
    write_paid_route_store(&store_path, &store)?;
    Ok(payment.envelope)
}

pub(crate) async fn finalize_automatic_paid_exit(
    automatic: &PaidExitAutomaticBuyer,
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    now_unix: u64,
) -> Result<()> {
    let Some(candidate) = automatic.candidate.as_ref() else {
        return Ok(());
    };
    drain_paid_exit_buyer_usage(runtime, config_path, &candidate.seller_pubkey, now_unix)?;
    if candidate.funded {
        let wallet_data_dir = paid_exit_wallet_data_dir(config_path);
        let signer =
            FileSpilmanPaymentSigner::load(&wallet_data_dir).map_err(|error| anyhow!("{error}"))?;
        let store_path = paid_route_store_file_path(config_path);
        let mut store = load_paid_route_store(&store_path)?;
        let result = paid_exit_settle_with_signer(PaidExitSettleRequest {
            app,
            config_path,
            store: &mut store,
            signer: &signer,
            session_id: &candidate.session_id,
            dry_run: false,
            wallet_data_dir: &wallet_data_dir,
            now_unix,
        })?;
        if result.persisted && result.payment.changed {
            write_paid_route_store(&store_path, &store)?;
        }
        let flushed = flush_paid_exit_payment_outbox(runtime, config_path).await;
        if flushed.errors > 0 {
            eprintln!(
                "paid-exit: automatic seller finalization queued with {} send error(s)",
                flushed.errors
            );
        }
    }
    Ok(())
}

fn drain_paid_exit_buyer_usage(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    config_path: &Path,
    seller_pubkey: &str,
    now_unix: u64,
) -> Result<PaidRouteUsage> {
    let delta = runtime.drain_paid_route_usage(seller_pubkey)?;
    if delta.is_empty() {
        return Ok(delta);
    }
    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let changed = store
        .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
            seller_pubkey: seller_pubkey.to_string(),
            usage_delta: delta.clone(),
            now_unix,
        })?
        .is_some_and(|result| result.changed);
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }
    Ok(delta)
}

#[cfg(test)]
#[path = "automatic/tests.rs"]
mod tests;
