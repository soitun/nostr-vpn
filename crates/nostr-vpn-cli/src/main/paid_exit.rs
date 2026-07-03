#[derive(Debug, Args)]
struct PaidExitArgs {
    #[command(subcommand)]
    command: PaidExitCommand,
}

#[derive(Debug, Subcommand)]
enum PaidExitCommand {
    /// Show paid-exit seller config, wallet, offers, channels, and sessions.
    Status(PaidExitStatusArgs),
    /// Enable this machine as a paid-exit seller, refresh its offer, and optionally publish it.
    Run(PaidExitRunArgs),
    /// Build/sign the local paid-exit offer, and optionally publish it.
    Offer(PaidExitOfferArgs),
    /// Import a signed paid-exit offer event from JSON.
    #[command(name = "import-offer")]
    ImportOffer(PaidExitImportOfferArgs),
    /// Discover and verify paid-exit offers from Nostr relays.
    Discover(PaidExitDiscoverArgs),
    /// Open a local buyer session for a discovered paid-exit offer.
    Buy(PaidExitBuyArgs),
    /// Select an existing buyer session as the active public exit route.
    Use(PaidExitUseArgs),
    /// Measure the realized public exit IP and quality for a paid-exit session.
    Probe(PaidExitProbeArgs),
    /// Record realized exit IP and quality measurements for a paid-exit session.
    #[command(name = "record-probe")]
    RecordProbe(PaidExitRecordProbeArgs),
    /// Create a buyer Cashu streaming payment envelope for a paid-exit session.
    #[command(name = "create-payment")]
    CreatePayment(PaidExitCreatePaymentArgs),
    /// Create a fallback fixed Cashu-token lease payment envelope.
    #[command(name = "create-token-lease")]
    CreateTokenLease(PaidExitCreateTokenLeaseArgs),
    /// Sign due buyer Cashu streaming balance updates from the local wallet.
    #[command(name = "stream-payments")]
    StreamPayments(PaidExitStreamPaymentsArgs),
    /// Settle and close a buyer Cashu streaming channel.
    Settle(PaidExitSettleArgs),
    /// Apply an incoming buyer Cashu streaming payment envelope as a seller.
    #[command(name = "apply-payment")]
    ApplyPayment(PaidExitApplyPaymentArgs),
    /// Publish a buyer payment envelope privately to the seller over Nostr.
    #[command(name = "send-payment")]
    SendPayment(PaidExitSendPaymentArgs),
    /// Receive private buyer payment envelopes from Nostr and apply them locally.
    #[command(name = "receive-payments")]
    ReceivePayments(PaidExitReceivePaymentsArgs),
    /// Close a seller Cashu streaming channel and stop routing it.
    Collect(PaidExitCollectArgs),
    /// Close all expired seller Cashu streaming channels with pending credit.
    #[command(name = "collect-due")]
    CollectDue(PaidExitCollectDueArgs),
    /// Manage local paid-route wallet mint metadata.
    Wallet(PaidExitWalletArgs),
}

#[derive(Debug, Args)]
struct PaidExitStatusArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitRunArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Stable Nostr d-tag for this seller's paid-exit offer.
    #[arg(long)]
    offer_id: Option<String>,
    /// Override configured Nostr relays. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// Publish the refreshed offer to Nostr relays.
    #[arg(long)]
    publish: bool,
    /// Do not ask a running daemon to reload after saving seller config.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    upstream: Option<String>,
    #[arg(long)]
    meter: Option<String>,
    #[arg(long)]
    price_msat: Option<u64>,
    /// Price unit. Byte meters accept values like "1 MB" or "1 GB".
    #[arg(long, value_name = "UNITS")]
    per_units: Option<String>,
    /// Replace accepted Cashu mints with a comma-separated list. Empty clears them.
    #[arg(long)]
    accepted_mints: Option<String>,
    /// Add an accepted Cashu mint. Can be supplied more than once.
    #[arg(long = "accepted-mint")]
    accepted_mint: Vec<String>,
    #[arg(long)]
    country_code: Option<String>,
    #[arg(long)]
    region: Option<String>,
    #[arg(long)]
    asn: Option<u32>,
    #[arg(long)]
    network_class: Option<String>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    ipv4: Option<bool>,
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    ipv6: Option<bool>,
    #[arg(long)]
    max_channel_capacity_sat: Option<u64>,
    #[arg(long)]
    channel_expiry_secs: Option<u64>,
    /// Free traffic before payment. Byte meters accept values like "1 MB".
    #[arg(long, value_name = "UNITS")]
    free_probe_units: Option<String>,
    /// Extra unpaid traffic allowed after payment runs behind.
    #[arg(long, value_name = "UNITS")]
    grace_units: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitOfferArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Stable Nostr d-tag for this seller's paid-exit offer.
    #[arg(long)]
    offer_id: Option<String>,
    /// Override configured Nostr relays. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// Publish the signed offer to Nostr relays.
    #[arg(long)]
    publish: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitImportOfferArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Signed Nostr paid-route offer event JSON. Omit with --event-stdin or --event-file.
    #[arg(long, conflicts_with_all = ["event_stdin", "event_file"])]
    event: Option<String>,
    /// Read signed offer event JSON from stdin.
    #[arg(long, conflicts_with = "event_file")]
    event_stdin: bool,
    /// File containing signed offer event JSON.
    #[arg(long, conflicts_with = "event_stdin")]
    event_file: Option<PathBuf>,
    /// Relay URL metadata to store with the imported offer.
    #[arg(long = "relay")]
    relays: Vec<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitDiscoverArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override configured Nostr relays. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    #[arg(long, default_value_t = 5)]
    duration_secs: u64,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// Ignore offer events older than this many seconds.
    #[arg(long, default_value_t = 86_400)]
    since_secs: u64,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitBuyArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Store key, offer id, or seller npub of the paid-exit offer to buy.
    offer: String,
    /// Cashu mint URL to use. Defaults to a compatible wallet/default mint.
    #[arg(long)]
    mint: Option<String>,
    #[arg(long)]
    channel_capacity_sat: Option<u64>,
    #[arg(long, default_value_t = 0)]
    initial_paid_msat: u64,
    /// Keep the selected VPN exit unchanged after opening the paid session.
    #[arg(long)]
    no_select_exit_node: bool,
    /// Do not ask a running daemon to reload after selecting the paid exit.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitUseArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer paid-route session id.
    session: String,
    /// Do not ask a running daemon to reload after selecting the paid exit.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitProbeArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer or seller paid-route session id.
    session: String,
    /// Public-IP endpoint. Plain IP or JSON with ip/query/origin is accepted.
    #[arg(long)]
    ip_url: Option<String>,
    /// STUN server for realized public-IP probing. Defaults to configured NAT STUN servers.
    #[arg(long = "stun-server")]
    stun_servers: Vec<String>,
    /// Skip STUN realized public-IP probing and use HTTPS only.
    #[arg(long)]
    no_stun: bool,
    /// GeoIP endpoint template. Use {ip}; otherwise the IP is appended.
    #[arg(long)]
    geoip_url_template: Option<String>,
    /// Skip GeoIP country/ASN lookup.
    #[arg(long)]
    no_geoip: bool,
    /// Download endpoint for rough bandwidth measurement. Use {bytes} or a bytes query is appended.
    #[arg(long)]
    download_url: Option<String>,
    /// Upload endpoint for rough bandwidth measurement.
    #[arg(long)]
    upload_url: Option<String>,
    /// Bytes to transfer for each rough bandwidth direction.
    #[arg(long, default_value_t = DEFAULT_PAID_ROUTE_BANDWIDTH_BYTES)]
    bandwidth_bytes: u64,
    /// Skip rough bandwidth measurement.
    #[arg(long)]
    no_bandwidth: bool,
    /// Number of public-IP samples used for latency/jitter/loss.
    #[arg(long, default_value_t = 3)]
    samples: u8,
    #[arg(long, default_value_t = 5)]
    timeout_secs: u64,
    /// Do not ask a running daemon to reload after saving the probe result.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitRecordProbeArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer or seller paid-route session id.
    session: String,
    /// Realized public exit IP observed through this route.
    #[arg(long)]
    realized_exit_ip: Option<String>,
    /// Country code observed from realized exit IP geolocation.
    #[arg(long)]
    observed_country_code: Option<String>,
    /// ASN observed from realized exit IP geolocation.
    #[arg(long)]
    observed_asn: Option<u32>,
    #[arg(long)]
    latency_ms: Option<u32>,
    #[arg(long)]
    jitter_ms: Option<u32>,
    /// Packet loss in parts per million.
    #[arg(long)]
    packet_loss_ppm: Option<u32>,
    #[arg(long)]
    down_bps: Option<u64>,
    #[arg(long)]
    up_bps: Option<u64>,
    #[arg(long)]
    uptime_secs: Option<u64>,
    #[arg(long)]
    last_seen_unix: Option<u64>,
    /// Do not ask a running daemon to reload after saving the probe result.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PaidExitCreatePaymentKind {
    ChannelOpen,
    BalanceUpdate,
    CooperativeClose,
}

impl From<PaidExitCreatePaymentKind> for BuildPaidRouteBuyerPaymentEnvelopeKind {
    fn from(value: PaidExitCreatePaymentKind) -> Self {
        match value {
            PaidExitCreatePaymentKind::ChannelOpen => Self::ChannelOpen,
            PaidExitCreatePaymentKind::BalanceUpdate => Self::BalanceUpdate,
            PaidExitCreatePaymentKind::CooperativeClose => Self::CooperativeClose,
        }
    }
}

#[derive(Debug, Args)]
struct PaidExitCreatePaymentArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer session id.
    session: String,
    /// Payment envelope kind to create.
    #[arg(long, value_enum, default_value_t = PaidExitCreatePaymentKind::BalanceUpdate)]
    kind: PaidExitCreatePaymentKind,
    /// JSON CashuSpilmanPayment snapshot. Omit with --payment-stdin.
    #[arg(long)]
    payment: Option<String>,
    /// Read JSON CashuSpilmanPayment from stdin.
    #[arg(long, conflicts_with = "payment")]
    payment_stdin: bool,
    /// Fund/open the session's Cashu Spilman channel from the local wallet.
    #[arg(long, conflicts_with_all = ["payment", "payment_stdin", "sign_from_wallet"])]
    open_from_wallet: bool,
    /// Sign the payment from the local wallet's persisted Spilman client channel.
    #[arg(long, conflicts_with_all = ["payment", "payment_stdin", "open_from_wallet"])]
    sign_from_wallet: bool,
    /// Override the wallet mint used for --open-from-wallet.
    #[arg(long)]
    mint: Option<String>,
    /// Specific mint keyset id to use for --open-from-wallet.
    #[arg(long)]
    keyset_id: Option<String>,
    /// KeysetInfo JSON for --open-from-wallet. If omitted, fetch from mint.
    #[arg(long, conflicts_with = "keyset_info_file")]
    keyset_info: Option<String>,
    /// File containing KeysetInfo JSON for --open-from-wallet.
    #[arg(long, conflicts_with = "keyset_info")]
    keyset_info_file: Option<PathBuf>,
    /// Maximum value per Cashu output when opening a Spilman channel.
    #[arg(long, default_value_t = 64)]
    max_amount_per_output: u64,
    /// Delivered route units to report. Defaults to current session usage.
    #[arg(long)]
    delivered_units: Option<u64>,
    /// Paid amount in millisats. Defaults from the payment balance and Cashu unit.
    #[arg(long)]
    paid_msat: Option<u64>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitCreateTokenLeaseArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer session id.
    session: String,
    /// Cashu token to pay the fixed lease. Omit with --token-stdin.
    #[arg(long, required_unless_present = "token_stdin")]
    token: Option<String>,
    /// Read Cashu token from stdin.
    #[arg(long, conflicts_with = "token")]
    token_stdin: bool,
    /// Cashu mint URL for the token. Defaults to the session mint.
    #[arg(long)]
    mint: Option<String>,
    /// Cashu token unit.
    #[arg(long, default_value = "sat")]
    unit: String,
    /// Token amount in the selected Cashu unit.
    #[arg(long, alias = "amount-sat")]
    amount: u64,
    /// Optional route credit in millisats. Defaults from the token amount and unit.
    #[arg(long)]
    paid_msat: Option<u64>,
    /// Override token lease expiry. Defaults to the selected session expiry.
    #[arg(long)]
    expires_at_unix: Option<u64>,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitStreamPaymentsArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override configured Nostr relays when --publish is set. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// Publish signed balance updates privately to sellers over Nostr.
    #[arg(long)]
    publish: bool,
    /// Only sign updates at least this many millisats above the last signed balance.
    #[arg(long, default_value_t = 1)]
    min_increment_msat: u64,
    /// Maximum number of due buyer sessions to update. Zero means no limit.
    #[arg(long, default_value_t = 0)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitSettleArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Buyer session id.
    session: String,
    /// Override configured Nostr relays when publishing. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// Do not publish the close envelope; print it for manual sending.
    #[arg(long)]
    no_publish: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitApplyPaymentArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// JSON StreamingRoutePaymentEnvelope. Omit with --envelope-stdin.
    #[arg(long, required_unless_present = "envelope_stdin")]
    envelope: Option<String>,
    /// Read JSON StreamingRoutePaymentEnvelope from stdin.
    #[arg(long, conflicts_with = "envelope")]
    envelope_stdin: bool,
    /// Do not ask a running daemon to reload after applying a payment.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitSendPaymentArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override configured Nostr relays. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    /// JSON StreamingRoutePaymentEnvelope. Omit with --envelope-stdin.
    #[arg(long, required_unless_present = "envelope_stdin")]
    envelope: Option<String>,
    /// Read JSON StreamingRoutePaymentEnvelope from stdin.
    #[arg(long, conflicts_with = "envelope")]
    envelope_stdin: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitReceivePaymentsArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override configured Nostr relays. Can be supplied more than once.
    #[arg(long = "relay")]
    relays: Vec<String>,
    #[arg(long, default_value_t = 30)]
    duration_secs: u64,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Ignore payment events older than this many seconds. Defaults to no
    /// since filter because NIP-59 gift wraps intentionally randomize
    /// created_at into the past.
    #[arg(long, default_value_t = 0)]
    since_secs: u64,
    /// Do not ask a running daemon to reload after applying payments.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitCollectArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Seller channel id to close.
    channel: String,
    /// Do not ask a running daemon to reload after closing the channel.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitCollectDueArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Maximum number of due seller channels to close. Zero means no limit.
    #[arg(long, default_value_t = 0)]
    limit: usize,
    /// Do not ask a running daemon to reload after closing due channels.
    #[arg(long)]
    no_reload_daemon: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PaidExitWalletArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    command: PaidExitWalletCommand,
}

#[derive(Debug, Subcommand)]
enum PaidExitWalletCommand {
    /// Show configured wallet mints and local Cashu wallet balances.
    Show(PaidExitWalletShowArgs),
    /// Create a Lightning invoice to top up the Cashu wallet.
    #[command(name = "topup", alias = "top-up")]
    Topup(PaidExitWalletTopupArgs),
    /// Receive/import a Cashu token into the wallet.
    #[command(alias = "import")]
    Receive(PaidExitWalletReceiveArgs),
    /// Send/export a Cashu token from the wallet.
    #[command(alias = "export")]
    Send(PaidExitWalletSendArgs),
    /// Pay a Lightning invoice from the Cashu wallet.
    Withdraw(PaidExitWalletWithdrawArgs),
    /// Add or update an accepted Cashu mint.
    AddMint(PaidExitWalletAddMintArgs),
    /// Remove a Cashu mint.
    RemoveMint(PaidExitWalletMintUrlArgs),
    /// Set the default Cashu mint.
    SetDefault(PaidExitWalletMintUrlArgs),
}

#[derive(Debug, Args)]
struct PaidExitWalletShowArgs {
    /// Refresh pending mint quotes and incomplete sends before showing balances.
    #[arg(long)]
    refresh: bool,
    /// Include recent wallet activity.
    #[arg(long)]
    activity: bool,
}

#[derive(Debug, Args)]
struct PaidExitWalletTopupArgs {
    /// Amount to mint, in sats.
    amount_sat: u64,
    /// Cashu mint URL. Defaults to the wallet default mint, then Minibits.
    #[arg(long)]
    mint: Option<String>,
}

#[derive(Debug, Args)]
struct PaidExitWalletReceiveArgs {
    /// Cashu token. Omit with --token-stdin to read from stdin.
    token: Option<String>,
    #[arg(long)]
    token_stdin: bool,
}

#[derive(Debug, Args)]
struct PaidExitWalletSendArgs {
    /// Amount to send, in sats.
    amount_sat: u64,
    /// Cashu mint URL. Defaults to the wallet default mint, then Minibits.
    #[arg(long)]
    mint: Option<String>,
}

#[derive(Debug, Args)]
struct PaidExitWalletWithdrawArgs {
    /// BOLT11 invoice to pay.
    invoice: String,
    /// Cashu mint URL. Defaults to the wallet default mint, then Minibits.
    #[arg(long)]
    mint: Option<String>,
}

#[derive(Debug, Args)]
struct PaidExitWalletAddMintArgs {
    /// Cashu mint URL.
    url: String,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    balance_msat: Option<u64>,
    #[arg(long)]
    make_default: bool,
}

#[derive(Debug, Args)]
struct PaidExitWalletMintUrlArgs {
    /// Cashu mint URL.
    url: String,
}

fn paid_exit_status_json(app: &AppConfig) -> serde_json::Value {
    let config = &app.paid_exit;
    json!({
        "enabled": config.enabled,
        "upstream": config.access.upstream.as_str(),
        "private_vpn_access": config.access.private_vpn_access.as_str(),
        "meter": config.pricing.meter.as_str(),
        "price_msat": config.pricing.price_msat,
        "price_text": paid_exit_price_text(
            config.pricing.price_msat,
            config.pricing.per_units,
            config.pricing.meter,
        ),
        "per_units": config.pricing.per_units,
        "per_units_text": paid_exit_meter_unit_text(config.pricing.per_units, config.pricing.meter),
        "accepted_mints": &config.channel.accepted_mints,
        "max_channel_capacity_sat": config.channel.max_channel_capacity_sat,
        "channel_expiry_secs": config.channel.channel_expiry_secs,
        "channel_expiry_text": paid_exit_duration_text(config.channel.channel_expiry_secs),
        "settlement_text": paid_exit_settlement_text(config.channel.channel_expiry_secs),
        "free_probe_units": config.channel.free_probe_units,
        "free_probe_text": paid_exit_traffic_unit_text(
            config.channel.free_probe_units,
            config.pricing.meter
        ),
        "grace_units": config.channel.grace_units,
        "grace_text": paid_exit_traffic_unit_text(config.channel.grace_units, config.pricing.meter),
        "country_code": &config.location.country_code,
        "region": &config.location.region,
        "asn": config.location.asn,
        "network_class": config.location.network_class.as_str(),
        "ipv4": config.ip_support.ipv4,
        "ipv6": config.ip_support.ipv6,
    })
}

fn print_paid_exit_status(app: &AppConfig) {
    let config = &app.paid_exit;
    println!(
        "paid_exit: {}",
        if config.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );

    if !config.enabled && config.channel.accepted_mints.is_empty() && config.pricing.price_msat == 0
    {
        return;
    }

    println!(
        "paid_exit_price: {}",
        paid_exit_price_text(
            config.pricing.price_msat,
            config.pricing.per_units,
            config.pricing.meter,
        )
    );
    println!(
        "paid_exit_access: upstream={} private_vpn_access={}",
        config.access.upstream.as_str(),
        config.access.private_vpn_access.as_str()
    );
    println!(
        "paid_exit_channel: max={} expiry={}s free_probe={} grace={}",
        paid_exit_sat_text(config.channel.max_channel_capacity_sat),
        config.channel.channel_expiry_secs,
        paid_exit_traffic_unit_text(config.channel.free_probe_units, config.pricing.meter),
        paid_exit_traffic_unit_text(config.channel.grace_units, config.pricing.meter)
    );
    println!(
        "paid_exit_settlement: {}",
        paid_exit_settlement_text(config.channel.channel_expiry_secs)
    );
    if !config.channel.accepted_mints.is_empty() {
        println!(
            "paid_exit_accepted_mints: {}",
            config.channel.accepted_mints.join(", ")
        );
    }
    println!(
        "paid_exit_location: country={} region={} class={} asn={}",
        display_or_none(&config.location.country_code),
        display_or_none(&config.location.region),
        config.location.network_class.as_str(),
        config
            .location
            .asn
            .map(|asn| asn.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "paid_exit_ip_support: ipv4={} ipv6={}",
        config.ip_support.ipv4, config.ip_support.ipv6
    );
}

fn paid_exit_price_text(price_msat: u64, per_units: u64, meter: PaidRouteMeter) -> String {
    format!(
        "{} / {}",
        paid_exit_msat_text(price_msat),
        paid_exit_meter_unit_text(per_units, meter)
    )
}

fn paid_exit_meter_unit_text(per_units: u64, meter: PaidRouteMeter) -> String {
    match meter {
        PaidRouteMeter::Bytes => paid_exit_decimal_bytes_text(per_units),
        PaidRouteMeter::Milliseconds => format!("{per_units} ms"),
        PaidRouteMeter::Packets => {
            if per_units == 1 {
                "1 packet".to_string()
            } else {
                format!("{per_units} packets")
            }
        }
    }
}

fn paid_exit_traffic_unit_text(units: u64, meter: PaidRouteMeter) -> String {
    match meter {
        PaidRouteMeter::Bytes => paid_exit_binary_bytes_text(units),
        _ => paid_exit_meter_unit_text(units, meter),
    }
}

fn paid_exit_settlement_text(channel_expiry_secs: u64) -> String {
    format!(
        "Channels end after {} or when you manually collect",
        paid_exit_duration_text(channel_expiry_secs)
    )
}

fn paid_exit_duration_text(seconds: u64) -> String {
    match seconds {
        0..=59 => paid_exit_plural_text(seconds.max(1), "sec"),
        60..=3_599 => paid_exit_plural_text((seconds / 60).max(1), "min"),
        3_600..=86_399 => {
            let hours = seconds / 3_600;
            let minutes = (seconds % 3_600) / 60;
            if minutes == 0 {
                paid_exit_plural_text(hours, "hour")
            } else {
                format!(
                    "{} {}",
                    paid_exit_plural_text(hours, "hour"),
                    paid_exit_plural_text(minutes, "min")
                )
            }
        }
        _ => {
            let days = seconds / 86_400;
            let hours = (seconds % 86_400) / 3_600;
            if hours == 0 {
                paid_exit_plural_text(days, "day")
            } else {
                format!(
                    "{} {}",
                    paid_exit_plural_text(days, "day"),
                    paid_exit_plural_text(hours, "hour")
                )
            }
        }
    }
}

fn paid_exit_plural_text(value: u64, unit: &str) -> String {
    if value == 1 || matches!(unit, "sec" | "min") {
        format!("{value} {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

fn paid_exit_parse_pricing_units_arg(
    value: &str,
    meter: PaidRouteMeter,
    flag: &str,
) -> Result<u64> {
    paid_exit_parse_units_arg(value, meter, 1_000.0, flag)
}

fn paid_exit_parse_traffic_units_arg(
    value: &str,
    meter: PaidRouteMeter,
    flag: &str,
) -> Result<u64> {
    paid_exit_parse_units_arg(value, meter, 1_024.0, flag)
}

fn paid_exit_parse_units_arg(
    value: &str,
    meter: PaidRouteMeter,
    byte_scale: f64,
    flag: &str,
) -> Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("{flag} cannot be empty"));
    }
    if let Ok(units) = trimmed.parse::<u64>() {
        return Ok(units);
    }
    if meter != PaidRouteMeter::Bytes {
        return Err(anyhow!(
            "{flag} must be a whole number when --meter is {}",
            meter.as_str()
        ));
    }
    paid_exit_parse_byte_units_text(trimmed, byte_scale, flag)
}

fn paid_exit_parse_byte_units_text(value: &str, scale: f64, flag: &str) -> Result<u64> {
    let normalized = value.trim().to_lowercase();
    let mut characters = normalized.chars().peekable();
    let mut number_text = String::new();
    while let Some(character) = characters.peek().copied() {
        if character.is_ascii_digit() || character == '.' {
            number_text.push(character);
            characters.next();
        } else if character == ',' || character == '_' {
            characters.next();
        } else {
            break;
        }
    }
    while matches!(characters.peek(), Some(character) if character.is_whitespace()) {
        characters.next();
    }
    let unit_text = characters
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if unit_text
        .chars()
        .any(|character| character.is_ascii_digit() || matches!(character, '.' | ',' | '_'))
    {
        return Err(anyhow!("{flag} has invalid byte unit '{unit_text}'"));
    }
    let amount = number_text
        .parse::<f64>()
        .map_err(|_| anyhow!("{flag} has invalid byte amount '{value}'"))?;
    if !amount.is_finite() || amount < 0.0 {
        return Err(anyhow!("{flag} has invalid byte amount '{value}'"));
    }
    let multiplier = match unit_text.as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" | "kib" => scale,
        "m" | "mb" | "mib" => scale.powi(2),
        "g" | "gb" | "gib" => scale.powi(3),
        "t" | "tb" | "tib" => scale.powi(4),
        _ => return Err(anyhow!("{flag} has unsupported byte unit '{unit_text}'")),
    };
    let units = (amount * multiplier).round();
    if !units.is_finite() || units < 0.0 || units > u64::MAX as f64 {
        return Err(anyhow!("{flag} byte amount is out of range"));
    }
    Ok(units as u64)
}

fn paid_exit_msat_text(msat: u64) -> String {
    if msat == 0 {
        return "0 sat".to_string();
    }
    let whole = msat / 1_000;
    let remainder = msat % 1_000;
    if remainder == 0 {
        format!("{whole} sat")
    } else {
        format!("{whole}.{remainder:03} sat")
    }
}

fn paid_exit_sat_text(sat: u64) -> String {
    format!("{sat} sat")
}

fn paid_exit_usage_text(bytes: u64, packets: u64, delivered_units: u64) -> String {
    if bytes > 0 {
        format!("{} used", paid_exit_binary_bytes_text(bytes))
    } else if packets > 0 {
        match packets {
            1 => "1 packet".to_string(),
            count => format!("{count} packets"),
        }
    } else {
        match delivered_units {
            1 => "1 unit".to_string(),
            count => format!("{count} units"),
        }
    }
}

fn paid_exit_binary_bytes_text(bytes: u64) -> String {
    paid_exit_scaled_bytes_text(bytes, 1_024.0)
}

fn paid_exit_decimal_bytes_text(bytes: u64) -> String {
    paid_exit_scaled_bytes_text(bytes, 1_000.0)
}

fn paid_exit_scaled_bytes_text(bytes: u64, threshold: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut index = 0usize;
    while value >= threshold && index < units.len() - 1 {
        value /= threshold;
        index += 1;
    }
    if index == 0 {
        format!("{bytes} B")
    } else if (value - value.round()).abs() < 0.05 {
        format!("{value:.0} {}", units[index])
    } else {
        format!("{value:.1} {}", units[index])
    }
}

fn paid_exit_status_snapshot_json(
    app: &AppConfig,
    store_path: &Path,
    store: &PaidRouteStore,
) -> serde_json::Value {
    let now_unix = unix_timestamp();
    let offers = store
        .offers
        .iter()
        .map(|(key, record)| {
            json!({
                "key": key,
                "offer": record.offer,
                "event_id": record.signed_offer.event.id.to_string(),
                "relays": record.relay_urls,
                "first_seen_unix": record.first_seen_unix,
                "last_seen_unix": record.last_seen_unix,
            })
        })
        .collect::<Vec<_>>();
    let channels = store
        .channels
        .values()
        .map(paid_exit_channel_status_json)
        .collect::<Vec<_>>();
    let sessions = store
        .sessions
        .values()
        .map(|record| paid_exit_session_status_json(record, store, &app.paid_exit, now_unix))
        .collect::<Vec<_>>();
    let seller_admissions = store.seller_admissions(&app.paid_exit, now_unix);
    let seller_collection = store.seller_collection_states(&app.paid_exit, now_unix);
    let pending_buyer_credit_msat =
        paid_exit_seller_pending_buyer_credit_msat(&app.paid_exit, store);
    let auto_collect_due_msat = seller_collection
        .iter()
        .filter(|state| state.auto_collect_due)
        .map(|state| state.paid_msat)
        .fold(0_u64, u64::saturating_add);

    json!({
        "config": paid_exit_status_json(app),
        "store_path": store_path.display().to_string(),
        "wallet": store.wallet,
        "seller_accounting": {
            "pending_buyer_credit_msat": pending_buyer_credit_msat,
            "pending_buyer_credit_text": paid_exit_msat_text(pending_buyer_credit_msat),
            "pending_buyer_credit_help_text": paid_exit_pending_buyer_credit_help_text(pending_buyer_credit_msat),
            "collectable_channel_count": seller_collection.iter().filter(|state| state.collectable).count(),
            "auto_collect_due_count": seller_collection.iter().filter(|state| state.auto_collect_due).count(),
            "auto_collect_due_msat": auto_collect_due_msat,
            "auto_collect_due_text": paid_exit_msat_text(auto_collect_due_msat),
        },
        "counts": {
            "offers": store.offers.len(),
            "quotes": store.quotes.len(),
            "leases": store.leases.len(),
            "channels": store.channels.len(),
            "sessions": store.sessions.len(),
        },
        "offers": offers,
        "channels": channels,
        "sessions": sessions,
        "seller_admissions": seller_admissions,
        "seller_collection": seller_collection.iter().map(paid_exit_seller_collection_status_json).collect::<Vec<_>>(),
    })
}

fn paid_exit_channel_status_json(channel: &PaidRouteChannelRecord) -> serde_json::Value {
    json!({
        "channel_id": channel.channel_id,
        "offer_id": channel.offer_id,
        "role": paid_route_channel_role_text(channel.role),
        "status": paid_route_lifecycle_status_text(channel.status),
        "payment": {
            "mode": channel.payment.mode.clone().as_str(),
            "channel_id": channel.payment.channel_id,
            "cashu_unit": channel.payment.cashu_unit,
            "capacity_sat": channel.payment.capacity_sat,
            "paid_msat": channel.payment.paid_msat,
            "updated_at_unix": channel.payment.updated_at_unix,
            "cashu_spilman": paid_exit_spilman_payment_status_json(
                channel.payment.cashu_spilman_payment.as_ref()
            ),
            "cashu_token_lease": paid_exit_token_lease_status_json(
                channel.payment.cashu_token_lease.as_ref()
            ),
        },
        "mint_url": channel.mint_url,
        "counterparty_npub": channel.counterparty_npub,
        "created_at_unix": channel.created_at_unix,
        "updated_at_unix": channel.updated_at_unix,
        "expires_at_unix": channel.expires_at_unix,
        "error": channel.error,
    })
}

fn paid_exit_spilman_payment_status_json(
    payment: Option<&CashuSpilmanPayment>,
) -> serde_json::Value {
    match payment {
        Some(payment) => json!({
            "channel_id": payment.channel_id,
            "balance": payment.balance,
            "has_signature": !payment.signature.trim().is_empty(),
            "has_funding": payment.has_funding(),
        }),
        None => serde_json::Value::Null,
    }
}

fn paid_exit_spilman_receiver_mode(processing_available: bool) -> &'static str {
    if processing_available {
        "processing"
    } else {
        "claim_only"
    }
}

fn paid_exit_token_lease_status_json(
    token_lease: Option<&StreamingRouteCashuTokenLease>,
) -> serde_json::Value {
    match token_lease {
        Some(token_lease) => json!({
            "channel_id": token_lease.channel_id,
            "mint_url": token_lease.mint_url,
            "unit": token_lease.unit,
            "amount": token_lease.amount,
            "paid_msat": token_lease.paid_msat,
            "expires_unix": token_lease.expires_unix,
            "has_token": !token_lease.token.trim().is_empty(),
        }),
        None => serde_json::Value::Null,
    }
}

fn paid_exit_session_status_json(
    record: &PaidRouteSessionRecord,
    store: &PaidRouteStore,
    seller_config: &PaidExitConfig,
    now_unix: u64,
) -> serde_json::Value {
    let session = &record.session;
    let session_config = paid_exit_session_config(store, record);
    let country_claim = paid_route_country_claim(
        session_config
            .as_ref()
            .map(|config| config.location.country_code.as_str())
            .unwrap_or_default(),
        session.observed_country_code.as_deref(),
    );
    let decision = session_config.map(|config| {
        let decision = session.routing_decision(&config);
        json!({
            "state": decision.state.as_str(),
            "allow_routing": decision.allow_routing,
            "shared_internet": paid_exit_shared_internet_text(&decision, config.pricing.meter),
            "delivered_units": decision.delivered_units,
            "paid_msat": decision.paid_msat,
            "amount_due_msat": decision.amount_due_msat,
            "enforced_amount_due_msat": decision.enforced_amount_due_msat,
            "unpaid_msat": decision.unpaid_msat,
            "free_probe_remaining_units": decision.free_probe_remaining_units,
            "grace_remaining_units": decision.grace_remaining_units,
        })
    });
    let collection =
        store.seller_collection_state_for_session(seller_config, now_unix, &session.session_id);

    json!({
        "session_id": session.session_id,
        "lease_id": session.lease_id,
        "channel_id": session.payment.channel_id,
        "created_at_unix": record.created_at_unix,
        "updated_at_unix": record.updated_at_unix,
        "usage": session.usage,
        "payment": {
            "mode": session.payment.mode.clone().as_str(),
            "channel_id": session.payment.channel_id,
            "cashu_unit": session.payment.cashu_unit,
            "capacity_sat": session.payment.capacity_sat,
            "paid_msat": session.payment.paid_msat,
            "updated_at_unix": session.payment.updated_at_unix,
            "cashu_spilman": paid_exit_spilman_payment_status_json(
                session.payment.cashu_spilman_payment.as_ref()
            ),
            "cashu_token_lease": paid_exit_token_lease_status_json(
                session.payment.cashu_token_lease.as_ref()
            ),
        },
        "routing": decision,
        "collection": paid_exit_session_collection_status_json(collection.as_ref()),
        "realized_exit_ip": session.realized_exit_ip,
        "observed_country_code": session.observed_country_code,
        "observed_asn": session.observed_asn,
        "claimed_country_code": country_claim.claimed_country_code,
        "country_claim": {
            "claimed_country_code": country_claim.claimed_country_code,
            "observed_country_code": country_claim.observed_country_code,
            "status": country_claim.status.as_str(),
            "matches": country_claim.matches_claim(),
        },
        "quality": session.quality,
    })
}

fn paid_exit_session_collection_status_json(
    state: Option<&PaidRouteSellerCollectionState>,
) -> serde_json::Value {
    match state {
        Some(state) => paid_exit_seller_collection_status_json(state),
        None => serde_json::Value::Null,
    }
}

fn paid_exit_seller_collection_status_json(
    state: &PaidRouteSellerCollectionState,
) -> serde_json::Value {
    json!({
        "buyer_npub": state.buyer_npub,
        "session_id": state.session_id,
        "lease_id": state.lease_id,
        "channel_id": state.channel_id,
        "collectable": state.collectable,
        "manual_collect": state.manual_collect,
        "auto_collect_due": state.auto_collect_due,
        "reason": state.reason,
        "paid_msat": state.paid_msat,
        "paid_text": paid_exit_msat_text(state.paid_msat),
        "expires_at_unix": state.expires_at_unix,
        "due_at_unix": state.due_at_unix,
        "updated_at_unix": state.updated_at_unix,
    })
}

fn print_paid_exit_status_snapshot(app: &AppConfig, store_path: &Path, store: &PaidRouteStore) {
    let now_unix = unix_timestamp();
    print_paid_exit_status(app);
    println!("paid_exit_store: {}", store_path.display());
    println!(
        "paid_exit_store_counts: offers={} quotes={} leases={} channels={} sessions={}",
        store.offers.len(),
        store.quotes.len(),
        store.leases.len(),
        store.channels.len(),
        store.sessions.len()
    );
    print_paid_exit_wallet(store);
    let pending_buyer_credit_msat =
        paid_exit_seller_pending_buyer_credit_msat(&app.paid_exit, store);
    let seller_collection = store.seller_collection_states(&app.paid_exit, now_unix);
    let auto_collect_due_msat = seller_collection
        .iter()
        .filter(|state| state.auto_collect_due)
        .map(|state| state.paid_msat)
        .fold(0_u64, u64::saturating_add);
    if app.paid_exit.enabled || pending_buyer_credit_msat > 0 {
        let help = paid_exit_pending_buyer_credit_help_text(pending_buyer_credit_msat);
        if help.is_empty() {
            println!(
                "paid_exit_pending_buyer_credit: {}",
                paid_exit_msat_text(pending_buyer_credit_msat)
            );
        } else {
            println!(
                "paid_exit_pending_buyer_credit: {} ({help})",
                paid_exit_msat_text(pending_buyer_credit_msat)
            );
        }
        if auto_collect_due_msat > 0 {
            println!(
                "paid_exit_collect_due: {} across {} channel(s)",
                paid_exit_msat_text(auto_collect_due_msat),
                seller_collection
                    .iter()
                    .filter(|state| state.auto_collect_due)
                    .count()
            );
        }
    }

    if !store.offers.is_empty() {
        println!("paid_exit_offers:");
        for (key, record) in &store.offers {
            let offer = &record.offer;
            println!(
                "  {key} price={} country={} class={} upstream={} last_seen={}",
                paid_exit_price_text(
                    offer.pricing.price_msat,
                    offer.pricing.per_units,
                    offer.pricing.meter,
                ),
                display_or_none(&offer.location.country_code),
                offer.location.network_class.as_str(),
                offer.access.upstream.as_str(),
                record.last_seen_unix
            );
        }
    }

    if !store.channels.is_empty() {
        println!("paid_exit_channels:");
        for channel in store.channels.values() {
            println!(
                "  {} role={} status={} mode={} paid={} capacity={} counterparty={} mint={} expires_at={}{}",
                channel.channel_id,
                paid_route_channel_role_text(channel.role),
                paid_route_lifecycle_status_text(channel.status),
                channel.payment.mode.clone().as_str(),
                paid_exit_msat_text(channel.payment.paid_msat),
                paid_exit_sat_text(channel.payment.capacity_sat),
                display_or_none(&channel.counterparty_npub),
                display_or_none(&channel.mint_url),
                channel.expires_at_unix,
                paid_exit_error_suffix(&channel.error),
            );
        }
    }

    if !store.sessions.is_empty() {
        println!("paid_exit_sessions:");
        for record in store.sessions.values() {
            let session = &record.session;
            let session_config = paid_exit_session_config(store, record);
            let country_claim = paid_route_country_claim(
                session_config
                    .as_ref()
                    .map(|config| config.location.country_code.as_str())
                    .unwrap_or_default(),
                session.observed_country_code.as_deref(),
            );
            let decision = session_config
                .as_ref()
                .map(|config| session.routing_decision(config));
            let collection = store.seller_collection_state_for_session(
                &app.paid_exit,
                now_unix,
                &session.session_id,
            );
            let (state, allow, shared_internet, due, unpaid, delivered) = decision.as_ref().map_or(
                (
                    "unknown",
                    false,
                    "off: no matching offer".to_string(),
                    0,
                    0,
                    session.usage.units_for_meter(PaidRouteMeter::Bytes),
                ),
                |decision| {
                    (
                        decision.state.as_str(),
                        decision.allow_routing,
                        paid_exit_shared_internet_text(
                            decision,
                            session_config
                                .as_ref()
                                .map(|config| config.pricing.meter)
                                .unwrap_or(PaidRouteMeter::Bytes),
                        ),
                        decision.amount_due_msat,
                        decision.unpaid_msat,
                        decision.delivered_units,
                    )
                },
            );
            let bytes = session.usage.units_for_meter(PaidRouteMeter::Bytes);
            let packets = session.usage.units_for_meter(PaidRouteMeter::Packets);
            println!(
                "  {} shared_internet=\"{}\" state={} allow={} collection={} mode={} paid={} due={} unpaid={} usage={} exit_ip={} country={} claimed_country={} country_claim={} quality={}",
                session.session_id,
                shared_internet,
                state,
                allow,
                paid_exit_collection_state_text(collection.as_ref()),
                session.payment.mode.clone().as_str(),
                paid_exit_msat_text(session.payment.paid_msat),
                paid_exit_msat_text(due),
                paid_exit_msat_text(unpaid),
                paid_exit_usage_text(bytes, packets, delivered),
                display_or_none(session.realized_exit_ip.as_deref().unwrap_or_default()),
                display_or_none(session.observed_country_code.as_deref().unwrap_or_default()),
                display_or_none(&country_claim.claimed_country_code),
                country_claim.status.as_str(),
                paid_exit_quality_text(session.quality.as_ref()),
            );
        }
    }

    let seller_admissions = store.seller_admissions(&app.paid_exit, unix_timestamp());
    if !seller_admissions.is_empty() {
        println!("paid_exit_seller_admissions:");
        for admission in seller_admissions {
            println!(
                "  buyer={} session={} shared_internet=\"{}\" state={} allow={} paid={} due={} unpaid={} expires_at={}",
                admission.buyer_npub,
                admission.session_id,
                paid_exit_shared_internet_state_text(
                    admission.allow_routing,
                    admission.state.as_str(),
                    admission.unpaid_msat,
                ),
                admission.state.as_str(),
                admission.allow_routing,
                paid_exit_msat_text(admission.paid_msat),
                paid_exit_msat_text(admission.amount_due_msat),
                paid_exit_msat_text(admission.unpaid_msat),
                admission.expires_at_unix,
            );
        }
    }
}

fn paid_exit_session_config(
    store: &PaidRouteStore,
    record: &PaidRouteSessionRecord,
) -> Option<PaidExitConfig> {
    let session = &record.session;
    let lease = store.leases.get(&session.lease_id)?;
    let channel = store.channels.get(&session.payment.channel_id);
    let offer = store
        .offers
        .values()
        .find(|candidate| {
            candidate.offer.offer_id == lease.lease.offer_id
                && channel.is_none_or(|channel| {
                    channel.counterparty_npub.is_empty()
                        || channel.counterparty_npub == candidate.offer.seller_npub
                })
        })
        .or_else(|| {
            store
                .offers
                .values()
                .find(|candidate| candidate.offer.offer_id == lease.lease.offer_id)
        })?;
    Some(PaidExitConfig::from_paid_route_offer(&offer.offer))
}

fn paid_route_channel_role_text(role: PaidRouteChannelRole) -> &'static str {
    match role {
        PaidRouteChannelRole::Buyer => "buyer",
        PaidRouteChannelRole::Seller => "seller",
    }
}

fn paid_route_lifecycle_status_text(status: PaidRouteLifecycleStatus) -> &'static str {
    match status {
        PaidRouteLifecycleStatus::Opening => "opening",
        PaidRouteLifecycleStatus::Probing => "probing",
        PaidRouteLifecycleStatus::Active => "active",
        PaidRouteLifecycleStatus::Paused => "paused",
        PaidRouteLifecycleStatus::Closed => "closed",
        PaidRouteLifecycleStatus::Expired => "expired",
        PaidRouteLifecycleStatus::Failed => "failed",
    }
}

fn paid_route_lifecycle_is_current(status: PaidRouteLifecycleStatus) -> bool {
    matches!(
        status,
        PaidRouteLifecycleStatus::Opening
            | PaidRouteLifecycleStatus::Probing
            | PaidRouteLifecycleStatus::Active
            | PaidRouteLifecycleStatus::Paused
    )
}

fn paid_exit_seller_pending_buyer_credit_msat(
    config: &PaidExitConfig,
    store: &PaidRouteStore,
) -> u64 {
    if !config.enabled {
        return 0;
    }
    let seller_channel_ids = store
        .channels
        .values()
        .filter(|channel| {
            channel.role == PaidRouteChannelRole::Seller
                && paid_route_lifecycle_is_current(channel.status)
        })
        .map(|channel| channel.channel_id.as_str())
        .collect::<HashSet<_>>();
    store
        .sessions
        .values()
        .filter(|record| seller_channel_ids.contains(record.session.payment.channel_id.as_str()))
        .map(|record| record.session.payment.paid_msat)
        .fold(0_u64, u64::saturating_add)
}

fn paid_exit_pending_buyer_credit_help_text(pending_buyer_credit_msat: u64) -> &'static str {
    if pending_buyer_credit_msat > 0 {
        "collect to move it into wallet"
    } else {
        ""
    }
}

fn paid_exit_collection_state_text(state: Option<&PaidRouteSellerCollectionState>) -> String {
    let Some(state) = state else {
        return "none".to_string();
    };
    if state.auto_collect_due {
        format!("due: {}", paid_exit_msat_text(state.paid_msat))
    } else if state.manual_collect {
        format!("manual: {}", paid_exit_msat_text(state.paid_msat))
    } else {
        "none".to_string()
    }
}

fn paid_exit_error_suffix(error: &str) -> String {
    let error = error.trim();
    if error.is_empty() {
        String::new()
    } else {
        format!(" error={error}")
    }
}

fn paid_exit_quality_text(quality: Option<&PaidRouteQualityMetrics>) -> String {
    let Some(quality) = quality else {
        return "none".to_string();
    };
    let mut parts = Vec::new();
    if let Some(latency_ms) = quality.latency_ms {
        parts.push(format!("latency={latency_ms}ms"));
    }
    if let Some(jitter_ms) = quality.jitter_ms {
        parts.push(format!("jitter={jitter_ms}ms"));
    }
    if let Some(packet_loss_ppm) = quality.packet_loss_ppm {
        parts.push(format!(
            "loss={}",
            paid_exit_packet_loss_text(packet_loss_ppm)
        ));
    }
    if let Some(down_bps) = quality.down_bps {
        parts.push(format!("down={}", paid_exit_bitrate_text(down_bps)));
    }
    if let Some(up_bps) = quality.up_bps {
        parts.push(format!("up={}", paid_exit_bitrate_text(up_bps)));
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(",")
    }
}

pub(crate) fn paid_exit_shared_internet_text(
    decision: &PaidRouteRoutingDecision,
    meter: PaidRouteMeter,
) -> String {
    let prefix = if decision.allow_routing { "on" } else { "off" };
    match decision.state.as_str() {
        "free_probe" => {
            if decision.free_probe_remaining_units > 0 {
                format!(
                    "{prefix}: free test, {} left",
                    paid_exit_traffic_unit_text(decision.free_probe_remaining_units, meter)
                )
            } else {
                format!("{prefix}: free test")
            }
        }
        "paid" => format!("{prefix}: paid"),
        "grace" => {
            let mut text = if decision.grace_remaining_units > 0 {
                format!(
                    "{prefix}: grace, {} left",
                    paid_exit_traffic_unit_text(decision.grace_remaining_units, meter)
                )
            } else {
                format!("{prefix}: grace")
            };
            if decision.unpaid_msat > 0 {
                text.push_str(&format!(
                    ", {} behind",
                    paid_exit_msat_text(decision.unpaid_msat)
                ));
            }
            text
        }
        _ => paid_exit_shared_internet_state_text(
            decision.allow_routing,
            decision.state.as_str(),
            decision.unpaid_msat,
        ),
    }
}

fn paid_exit_shared_internet_state_text(
    allow_routing: bool,
    state: &str,
    unpaid_msat: u64,
) -> String {
    let prefix = if allow_routing { "on" } else { "off" };
    if state == "suspended" && unpaid_msat > 0 {
        format!(
            "{prefix}: payment needed, {} behind",
            paid_exit_msat_text(unpaid_msat)
        )
    } else if state == "suspended" {
        format!("{prefix}: payment needed")
    } else if state.trim().is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {state}")
    }
}

fn paid_exit_packet_loss_text(packet_loss_ppm: u32) -> String {
    let percent = f64::from(packet_loss_ppm) / 10_000.0;
    if percent == 0.0 {
        "0%".to_string()
    } else if percent < 0.1 {
        format!("{percent:.2}%")
    } else if percent < 10.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

fn paid_exit_bitrate_text(bps: u64) -> String {
    let units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
    let mut value = bps as f64;
    let mut index = 0usize;
    while value >= 1_000.0 && index < units.len() - 1 {
        value /= 1_000.0;
        index += 1;
    }
    if index == 0 {
        format!("{bps} bps")
    } else if value.fract().abs() < 0.05 {
        format!("{value:.0} {}", units[index])
    } else {
        format!("{value:.1} {}", units[index])
    }
}

pub(crate) fn paid_exit_offer_summary_line(
    offer: &PaidRouteOffer,
    event_id: impl std::fmt::Display,
) -> String {
    format!(
        "  {} seller={} price={} country={} class={} upstream={} channel=max={} expiry={}s free_probe={} grace={} mints={} quality={} event={}",
        offer.offer_id,
        offer.seller_npub,
        paid_exit_price_text(
            offer.pricing.price_msat,
            offer.pricing.per_units,
            offer.pricing.meter,
        ),
        display_or_none(&offer.location.country_code),
        offer.location.network_class.as_str(),
        offer.access.upstream.as_str(),
        paid_exit_sat_text(offer.channel.max_channel_capacity_sat),
        offer.channel.channel_expiry_secs,
        paid_exit_traffic_unit_text(offer.channel.free_probe_units, offer.pricing.meter),
        paid_exit_traffic_unit_text(offer.channel.grace_units, offer.pricing.meter),
        paid_exit_mints_text(&offer.channel.accepted_mints),
        paid_exit_quality_text(offer.quality.as_ref()),
        event_id,
    )
}

fn paid_exit_mints_text(mints: &[String]) -> String {
    if mints.is_empty() {
        "none".to_string()
    } else {
        mints.join(",")
    }
}

fn display_or_none(value: &str) -> &str {
    if value.trim().is_empty() {
        "none"
    } else {
        value
    }
}

async fn run_paid_exit_command(args: PaidExitArgs) -> Result<()> {
    match args.command {
        PaidExitCommand::Status(args) => paid_exit_status_command(args),
        PaidExitCommand::Run(args) => paid_exit_run_command(args).await,
        PaidExitCommand::Offer(args) => paid_exit_offer_command(args).await,
        PaidExitCommand::ImportOffer(args) => paid_exit_import_offer_command(args),
        PaidExitCommand::Discover(args) => paid_exit_discover_command(args).await,
        PaidExitCommand::Buy(args) => paid_exit_buy_command(args),
        PaidExitCommand::Use(args) => paid_exit_use_command(args),
        PaidExitCommand::Probe(args) => paid_exit_probe_command(args).await,
        PaidExitCommand::RecordProbe(args) => paid_exit_record_probe_command(args),
        PaidExitCommand::CreatePayment(args) => paid_exit_create_payment_command(args).await,
        PaidExitCommand::CreateTokenLease(args) => paid_exit_create_token_lease_command(args),
        PaidExitCommand::StreamPayments(args) => paid_exit_stream_payments_command(args).await,
        PaidExitCommand::Settle(args) => paid_exit_settle_command(args).await,
        PaidExitCommand::ApplyPayment(args) => paid_exit_apply_payment_command(args).await,
        PaidExitCommand::SendPayment(args) => paid_exit_send_payment_command(args).await,
        PaidExitCommand::ReceivePayments(args) => paid_exit_receive_payments_command(args).await,
        PaidExitCommand::Collect(args) => paid_exit_collect_command(args).await,
        PaidExitCommand::CollectDue(args) => paid_exit_collect_due_command(args).await,
        PaidExitCommand::Wallet(args) => paid_exit_wallet_command(args).await,
    }
}

fn paid_exit_status_command(args: PaidExitStatusArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let store_path = paid_route_store_file_path(&config_path);
    let store = load_paid_route_store(&store_path)?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_status_snapshot_json(
                &app,
                &store_path,
                &store
            ))?
        );
    } else {
        print_paid_exit_status_snapshot(&app, &store_path, &store);
    }

    Ok(())
}

struct PaidExitRunResult {
    config_path: PathBuf,
    store_path: PathBuf,
    offer: PaidRouteOffer,
    event_id: String,
    relays: Vec<String>,
    stored: bool,
    publish: Option<serde_json::Value>,
    daemon_reload_attempted: bool,
    status: serde_json::Value,
}

async fn paid_exit_run_command(args: PaidExitRunArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_run_once(args).await?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_run_result_json(&result))?
        );
    } else {
        print_paid_exit_run_result(&result);
    }

    Ok(())
}

async fn paid_exit_run_once(args: PaidExitRunArgs) -> Result<PaidExitRunResult> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;
    apply_paid_exit_run_settings(&mut app, &args)?;
    app.ensure_defaults();
    enable_wireguard_exit_upstream_for_paid_exit(&mut app);
    ensure_paid_exit_advertisable(&app)?;
    app.save(&config_path)?;

    let keys = app.nostr_keys()?;
    let relays = paid_exit_relay_urls(&app, &args.relays);
    let offer_id = args.offer_id.unwrap_or_else(default_paid_exit_offer_id);
    let receiver_pubkey_hex = paid_exit_spilman_receiver_pubkey_hex(&config_path, &app.paid_exit)?;
    let signed = signed_paid_exit_offer_from_config_with_receiver(
        offer_id,
        &keys,
        &app.paid_exit,
        receiver_pubkey_hex.as_deref(),
        Some(local_paid_exit_quality_hint()),
        unix_timestamp(),
    )?;
    let offer = signed.offer()?;
    let store_path = paid_route_store_file_path(&config_path);
    let stored =
        persist_paid_exit_offer_snapshot(&store_path, &signed, &relays, &offer, unix_timestamp())?;

    let daemon_reload_attempted = !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    let publish = if args.publish {
        Some(publish_paid_exit_offer_to_relays(&app, &signed, &relays).await?)
    } else {
        None
    };
    let store = load_paid_route_store(&store_path)?;
    let status = paid_exit_status_snapshot_json(&app, &store_path, &store);

    Ok(PaidExitRunResult {
        config_path,
        store_path,
        offer,
        event_id: signed.event.id.to_string(),
        relays,
        stored,
        publish,
        daemon_reload_attempted,
        status,
    })
}

fn apply_paid_exit_run_settings(app: &mut AppConfig, args: &PaidExitRunArgs) -> Result<()> {
    app.paid_exit.enabled = true;
    if let Some(value) = args.upstream.as_deref() {
        app.paid_exit.access.upstream = value
            .parse::<PaidExitUpstream>()
            .map_err(|error| anyhow!(error))?;
    }
    if let Some(value) = args.meter.as_deref() {
        app.paid_exit.pricing.meter = value
            .parse::<PaidRouteMeter>()
            .map_err(|error| anyhow!(error))?;
    }
    if let Some(value) = args.price_msat {
        app.paid_exit.pricing.price_msat = value;
    }
    if let Some(value) = args.per_units.as_deref() {
        app.paid_exit.pricing.per_units =
            paid_exit_parse_pricing_units_arg(value, app.paid_exit.pricing.meter, "--per-units")?;
    }
    if let Some(mints) = paid_exit_run_accepted_mints(args)? {
        app.paid_exit.channel.accepted_mints = mints;
    }
    if let Some(value) = args.country_code.as_deref() {
        app.paid_exit.location.country_code = value.to_string();
    }
    if let Some(value) = args.region.as_deref() {
        app.paid_exit.location.region = value.to_string();
    }
    if let Some(value) = args.asn {
        app.paid_exit.location.asn = Some(value);
    }
    if let Some(value) = args.network_class.as_deref() {
        app.paid_exit.location.network_class = value
            .parse::<ExitNetworkClass>()
            .map_err(|error| anyhow!(error))?;
    }
    if let Some(value) = args.ipv4 {
        app.paid_exit.ip_support.ipv4 = value;
    }
    if let Some(value) = args.ipv6 {
        app.paid_exit.ip_support.ipv6 = value;
    }
    if let Some(value) = args.max_channel_capacity_sat {
        app.paid_exit.channel.max_channel_capacity_sat = value;
    }
    if let Some(value) = args.channel_expiry_secs {
        app.paid_exit.channel.channel_expiry_secs = value;
    }
    if let Some(value) = args.free_probe_units.as_deref() {
        app.paid_exit.channel.free_probe_units = paid_exit_parse_traffic_units_arg(
            value,
            app.paid_exit.pricing.meter,
            "--free-probe-units",
        )?;
    }
    if let Some(value) = args.grace_units.as_deref() {
        app.paid_exit.channel.grace_units =
            paid_exit_parse_traffic_units_arg(value, app.paid_exit.pricing.meter, "--grace-units")?;
    }
    app.paid_exit.normalize();
    Ok(())
}

fn enable_wireguard_exit_upstream_for_paid_exit(app: &mut AppConfig) {
    if app.paid_exit.access.upstream == PaidExitUpstream::WireGuardExit
        && app.wireguard_exit.configured()
    {
        app.wireguard_exit.enabled = true;
    }
}

fn paid_exit_run_accepted_mints(args: &PaidExitRunArgs) -> Result<Option<Vec<String>>> {
    if args.accepted_mints.is_none() && args.accepted_mint.is_empty() {
        return Ok(None);
    }

    let mut values = Vec::new();
    if let Some(csv) = args.accepted_mints.as_deref() {
        values.extend(parse_csv_arg(csv));
    }
    values.extend(args.accepted_mint.iter().cloned());

    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        normalized.push(normalize_mint_url(value)?);
    }
    normalized.sort();
    normalized.dedup();
    Ok(Some(normalized))
}

fn paid_exit_spilman_receiver_pubkey_hex(
    config_path: &Path,
    paid_exit: &PaidExitConfig,
) -> Result<Option<String>> {
    let mut paid_exit = paid_exit.clone();
    paid_exit.normalize();
    if paid_exit.channel.accepted_mints.is_empty() {
        return Ok(None);
    }
    let key = load_or_create_cashu_spilman_receiver_key(&paid_exit_wallet_data_dir(config_path))
        .map_err(|error| anyhow!("{error}"))?;
    Ok(Some(key.public_key_hex))
}

fn paid_exit_spilman_receiver_config(
    paid_exit: &PaidExitConfig,
) -> Option<FileSpilmanPaymentReceiverConfig> {
    let mut paid_exit = paid_exit.clone();
    paid_exit.normalize();
    if paid_exit.channel.accepted_mints.is_empty() {
        return None;
    }
    Some(FileSpilmanPaymentReceiverConfig {
        accepted_mints: paid_exit.channel.accepted_mints,
        units: vec!["sat".to_string()],
        min_capacity: 1,
        max_amount_per_output: 0,
        min_expiry_seconds: 0,
    })
}

async fn try_load_paid_exit_spilman_receiver(
    config_path: &Path,
    paid_exit: &PaidExitConfig,
) -> (Option<FileSpilmanPaymentReceiver>, Option<String>) {
    let Some(receiver_config) = paid_exit_spilman_receiver_config(paid_exit) else {
        return (None, Some("no accepted Cashu mints configured".to_string()));
    };
    match FileSpilmanPaymentReceiver::load_with_keyset_refresh(
        &paid_exit_wallet_data_dir(config_path),
        receiver_config,
    )
    .await
    {
        Ok(receiver) => (Some(receiver), None),
        Err(error) => (None, Some(error)),
    }
}

fn apply_paid_route_seller_payment(
    store: &mut PaidRouteStore,
    request: ApplyPaidRouteSellerPaymentRequest,
    receiver: Option<&FileSpilmanPaymentReceiver>,
) -> Result<nostr_vpn_core::paid_route_store::ApplyPaidRouteSellerPaymentResult> {
    match receiver {
        Some(receiver) => {
            let context = "{}".to_string();
            store.apply_seller_payment_with_spilman_receiver(request, receiver, &context)
        }
        None => store.apply_seller_payment(request),
    }
}

fn paid_exit_run_result_json(result: &PaidExitRunResult) -> serde_json::Value {
    json!({
        "config_path": result.config_path.display().to_string(),
        "store_path": result.store_path.display().to_string(),
        "enabled": true,
        "offer": result.offer,
        "event_id": result.event_id,
        "relays": result.relays,
        "stored": result.stored,
        "published": result.publish.is_some(),
        "publish": result.publish,
        "daemon_reload_attempted": result.daemon_reload_attempted,
        "status": result.status,
    })
}

fn print_paid_exit_run_result(result: &PaidExitRunResult) {
    println!("paid_exit_seller: enabled");
    println!("config: {}", result.config_path.display());
    println!(
        "store: {} changed={}",
        result.store_path.display(),
        result.stored
    );
    println!("paid_exit_offer: {}", result.offer.offer_id);
    println!("seller: {}", result.offer.seller_npub);
    println!("event_id: {}", result.event_id);
    println!(
        "price: {}",
        paid_exit_price_text(
            result.offer.pricing.price_msat,
            result.offer.pricing.per_units,
            result.offer.pricing.meter,
        )
    );
    println!(
        "access: upstream={} private_vpn_access={}",
        result.offer.access.upstream.as_str(),
        result.offer.access.private_vpn_access.as_str()
    );
    println!(
        "channel: max={} expiry={}s free_probe={} grace={} accepted_mints={}",
        paid_exit_sat_text(result.offer.channel.max_channel_capacity_sat),
        result.offer.channel.channel_expiry_secs,
        paid_exit_traffic_unit_text(
            result.offer.channel.free_probe_units,
            result.offer.pricing.meter
        ),
        paid_exit_traffic_unit_text(result.offer.channel.grace_units, result.offer.pricing.meter),
        if result.offer.channel.accepted_mints.is_empty() {
            "none".to_string()
        } else {
            result.offer.channel.accepted_mints.join(", ")
        }
    );
    println!(
        "location: country={} region={} class={} asn={}",
        display_or_none(&result.offer.location.country_code),
        display_or_none(&result.offer.location.region),
        result.offer.location.network_class.as_str(),
        result
            .offer
            .location
            .asn
            .map(|asn| asn.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!("relays: {}", result.relays.join(", "));
    if let Some(publish) = &result.publish {
        println!(
            "published: {} success, {} failed",
            publish["success_count"].as_u64().unwrap_or_default(),
            publish["failed_count"].as_u64().unwrap_or_default()
        );
    } else {
        println!("published: false");
    }
    println!(
        "daemon_reload: {}",
        if result.daemon_reload_attempted {
            "attempted"
        } else {
            "skipped"
        }
    );
}

async fn paid_exit_offer_command(args: PaidExitOfferArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    ensure_paid_exit_advertisable(&app)?;
    let keys = app.nostr_keys()?;
    let relays = paid_exit_relay_urls(&app, &args.relays);
    let offer_id = args.offer_id.unwrap_or_else(default_paid_exit_offer_id);
    let receiver_pubkey_hex = paid_exit_spilman_receiver_pubkey_hex(&config_path, &app.paid_exit)?;
    let signed = signed_paid_exit_offer_from_config_with_receiver(
        offer_id,
        &keys,
        &app.paid_exit,
        receiver_pubkey_hex.as_deref(),
        Some(local_paid_exit_quality_hint()),
        unix_timestamp(),
    )?;
    let offer = signed.offer()?;
    let store_path = paid_route_store_file_path(&config_path);
    let stored =
        persist_paid_exit_offer_snapshot(&store_path, &signed, &relays, &offer, unix_timestamp())?;

    let publish = if args.publish {
        Some(publish_paid_exit_offer_to_relays(&app, &signed, &relays).await?)
    } else {
        None
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "offer": offer,
                "event": signed.event,
                "relays": relays,
                "publish": publish,
                "store_path": store_path,
                "stored": stored,
            }))?
        );
    } else {
        println!("paid_exit_offer: {}", offer.offer_id);
        println!("seller: {}", offer.seller_npub);
        println!(
            "price: {}",
            paid_exit_price_text(
                offer.pricing.price_msat,
                offer.pricing.per_units,
                offer.pricing.meter,
            )
        );
        println!(
            "access: upstream={} private_vpn_access={}",
            offer.access.upstream.as_str(),
            offer.access.private_vpn_access.as_str()
        );
        println!(
            "location: country={} region={} class={} asn={}",
            display_or_none(&offer.location.country_code),
            display_or_none(&offer.location.region),
            offer.location.network_class.as_str(),
            offer
                .location
                .asn
                .map(|asn| asn.to_string())
                .unwrap_or_else(|| "none".to_string())
        );
        println!("event_id: {}", signed.event.id);
        println!("relays: {}", relays.join(", "));
        println!("store: {} changed={stored}", store_path.display());
        if let Some(publish) = publish {
            println!(
                "published: {} success, {} failed",
                publish["success_count"].as_u64().unwrap_or_default(),
                publish["failed_count"].as_u64().unwrap_or_default()
            );
        } else {
            println!("published: false");
        }
    }

    Ok(())
}

fn paid_exit_import_offer_command(args: PaidExitImportOfferArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let event_json = read_paid_exit_offer_event(args.event, args.event_stdin, args.event_file)?;
    let event: Event = serde_json::from_str(&event_json)
        .context("failed to decode paid route offer event JSON")?;
    let signed = SignedPaidRouteOffer::from_event(event)
        .context("failed to verify paid route offer event")?;
    let offer = signed.offer()?;
    let store_path = paid_route_store_file_path(&config_path);
    let relays = normalize_relay_urls(args.relays);
    let changed = upsert_paid_route_offer(
        &store_path,
        signed.clone(),
        relays.clone(),
        unix_timestamp(),
    )?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "offer": offer,
                "event": signed.event,
                "relays": relays,
                "store_path": store_path,
                "stored": changed,
            }))?
        );
    } else {
        println!("paid_exit_offer: {}", offer.offer_id);
        println!("seller: {}", offer.seller_npub);
        println!("event_id: {}", signed.event.id);
        println!("store: {} changed={changed}", store_path.display());
    }

    Ok(())
}

async fn paid_exit_discover_command(args: PaidExitDiscoverArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let relays = paid_exit_relay_urls(&app, &args.relays);
    let since_unix = if args.since_secs == 0 {
        None
    } else {
        Some(unix_timestamp().saturating_sub(args.since_secs))
    };
    let offers = discover_paid_exit_offers_from_relays(
        &app,
        &relays,
        args.duration_secs,
        args.limit,
        since_unix,
    )
    .await?;
    let store_path = paid_route_store_file_path(&config_path);
    let stored_count = persist_paid_exit_discovered_offers(&store_path, &offers, &relays)?;

    if args.json {
        let offers_json = paid_exit_offer_results_json(&offers)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "relays": relays,
                "count": offers_json.len(),
                "offers": offers_json,
                "store_path": store_path,
                "stored_count": stored_count,
            }))?
        );
    } else {
        println!("paid_exit_offers: {}", offers.len());
        println!("store: {} changed={stored_count}", store_path.display());
        for signed in &offers {
            let offer = signed.offer()?;
            println!("{}", paid_exit_offer_summary_line(&offer, &signed.event.id));
        }
    }

    Ok(())
}

struct PaidExitBuyResult {
    store_path: PathBuf,
    session: OpenPaidRouteBuyerSessionResult,
    selected_exit_node: Option<String>,
    daemon_reload_attempted: bool,
}

struct PaidExitUseResult {
    config_path: PathBuf,
    store_path: PathBuf,
    session_id: String,
    seller_npub: String,
    selected_exit_node: String,
    daemon_reload_attempted: bool,
}

fn paid_exit_buy_command(args: PaidExitBuyArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_buy_once(args)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_buy_result_json(&result))?
        );
    } else {
        print_paid_exit_buy_result(&result);
    }

    Ok(())
}

fn paid_exit_buy_once(args: PaidExitBuyArgs) -> Result<PaidExitBuyResult> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;
    let buyer_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode buyer npub")?;
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let result = store.open_buyer_session(OpenPaidRouteBuyerSessionRequest {
        offer_selector: args.offer,
        buyer_npub,
        mint_url: args.mint,
        channel_capacity_sat: args.channel_capacity_sat,
        initial_paid_msat: args.initial_paid_msat,
        now_unix: unix_timestamp(),
    })?;
    if result.changed {
        write_paid_route_store(&store_path, &store)?;
    }

    let (selected_exit_node, daemon_reload_attempted) = if args.no_select_exit_node {
        (None, false)
    } else {
        let selected = app.select_public_paid_exit_node(&result.seller_npub)?;
        app.save(&config_path)?;
        let daemon_reload_attempted = !args.no_reload_daemon;
        if daemon_reload_attempted {
            maybe_reload_running_daemon(&config_path);
        }
        (Some(selected), daemon_reload_attempted)
    };

    Ok(PaidExitBuyResult {
        store_path,
        session: result,
        selected_exit_node,
        daemon_reload_attempted,
    })
}

fn paid_exit_use_command(args: PaidExitUseArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_use_once(args)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_use_result_json(&result))?
        );
    } else {
        print_paid_exit_use_result(&result);
    }

    Ok(())
}

fn paid_exit_use_once(args: PaidExitUseArgs) -> Result<PaidExitUseResult> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;
    let store_path = paid_route_store_file_path(&config_path);
    let store = load_paid_route_store(&store_path)?;
    let session_id = args.session.trim().to_string();
    if session_id.is_empty() {
        return Err(anyhow!("paid route session id is empty"));
    }
    let seller_npub = store.buyer_session_seller_npub(&session_id)?;
    let selected_exit_node = app.select_public_paid_exit_node(&seller_npub)?;
    app.save(&config_path)?;
    let daemon_reload_attempted = !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    Ok(PaidExitUseResult {
        config_path,
        store_path,
        session_id,
        seller_npub,
        selected_exit_node,
        daemon_reload_attempted,
    })
}

fn paid_exit_buy_result_json(result: &PaidExitBuyResult) -> serde_json::Value {
    json!({
        "store_path": result.store_path.display().to_string(),
        "session": result.session,
        "selected_exit_node": result.selected_exit_node,
        "daemon_reload_attempted": result.daemon_reload_attempted,
    })
}

fn paid_exit_use_result_json(result: &PaidExitUseResult) -> serde_json::Value {
    json!({
        "config_path": result.config_path.display().to_string(),
        "store_path": result.store_path.display().to_string(),
        "session_id": result.session_id,
        "seller_npub": result.seller_npub,
        "selected_exit_node": result.selected_exit_node,
        "daemon_reload_attempted": result.daemon_reload_attempted,
    })
}

fn print_paid_exit_buy_result(result: &PaidExitBuyResult) {
    println!("paid_exit_session: {}", result.session.session_id);
    println!("seller: {}", result.session.seller_npub);
    println!("offer: {}", result.session.offer_id);
    println!("mint: {}", display_or_none(&result.session.mint_url));
    println!(
        "channel: {} capacity={} expires_at={}",
        result.session.channel_id,
        paid_exit_sat_text(result.session.channel_capacity_sat),
        result.session.expires_at_unix
    );
    println!(
        "store: {} changed={}",
        result.store_path.display(),
        result.session.changed
    );
    if let Some(selected) = result.selected_exit_node.as_deref() {
        println!("selected_exit_node: {selected}");
    } else {
        println!("selected_exit_node: unchanged");
    }
    println!(
        "daemon_reload: {}",
        if result.daemon_reload_attempted {
            "attempted"
        } else {
            "skipped"
        }
    );
}

fn print_paid_exit_use_result(result: &PaidExitUseResult) {
    println!("paid_exit_session: {}", result.session_id);
    println!("seller: {}", result.seller_npub);
    println!("selected_exit_node: {}", result.selected_exit_node);
    println!("config: {}", result.config_path.display());
    println!("store: {}", result.store_path.display());
    println!(
        "daemon_reload: {}",
        if result.daemon_reload_attempted {
            "attempted"
        } else {
            "skipped"
        }
    );
}

struct PaidExitRecordProbeResult {
    store_path: PathBuf,
    probe: UpdatePaidRouteSessionProbeResult,
}

struct PaidExitProbeResult {
    store_path: PathBuf,
    measurement: PaidRouteProbeMeasurement,
    probe: UpdatePaidRouteSessionProbeResult,
    geoip_error: Option<String>,
    bandwidth_error: Option<String>,
}

async fn paid_exit_probe_command(args: PaidExitProbeArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_probe_once(args).await?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_probe_result_json(&result))?
        );
    } else {
        print_paid_exit_probe_result(&result);
    }

    Ok(())
}

async fn paid_exit_probe_once(args: PaidExitProbeArgs) -> Result<PaidExitProbeResult> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let now_unix = unix_timestamp();
    let (measurement, geoip_error, bandwidth_error) =
        paid_exit_probe_measurement(&args, &app, now_unix).await?;
    let record = paid_exit_record_probe_once(PaidExitRecordProbeArgs {
        config: Some(config_path.clone()),
        session: args.session,
        realized_exit_ip: measurement.realized_exit_ip.clone(),
        observed_country_code: measurement.observed_country_code.clone(),
        observed_asn: measurement.observed_asn,
        latency_ms: measurement.quality.latency_ms,
        jitter_ms: measurement.quality.jitter_ms,
        packet_loss_ppm: measurement.quality.packet_loss_ppm,
        down_bps: measurement.quality.down_bps,
        up_bps: measurement.quality.up_bps,
        uptime_secs: measurement.quality.uptime_secs,
        last_seen_unix: measurement.quality.last_seen_unix,
        no_reload_daemon: args.no_reload_daemon,
        json: false,
    })?;

    Ok(PaidExitProbeResult {
        store_path: record.store_path,
        measurement,
        probe: record.probe,
        geoip_error,
        bandwidth_error,
    })
}

async fn paid_exit_probe_measurement(
    args: &PaidExitProbeArgs,
    app: &AppConfig,
    now_unix: u64,
) -> Result<(PaidRouteProbeMeasurement, Option<String>, Option<String>)> {
    let timeout = Duration::from_secs(args.timeout_secs.max(1));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("failed to build paid exit probe HTTP client")?;
    let ip_url = args
        .ip_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PAID_ROUTE_PUBLIC_IP_URL);
    let stun_servers = paid_exit_probe_stun_servers(args, app);
    let sample_count = args.samples.clamp(1, 10);
    let mut samples = Vec::with_capacity(usize::from(sample_count));

    for sample_index in 0..sample_count {
        let stun_server = if stun_servers.is_empty() {
            None
        } else {
            Some(stun_servers[usize::from(sample_index) % stun_servers.len()].as_str())
        };
        samples.push(paid_exit_probe_public_ip_sample(&client, ip_url, stun_server, timeout).await);
    }

    let realized_ip = samples
        .iter()
        .rev()
        .find_map(|sample| sample.realized_exit_ip.as_deref());
    let (observed_country_code, observed_asn, geoip_error) =
        if args.no_geoip || realized_ip.is_none() {
            (None, None, None)
        } else {
            let template = args
                .geoip_url_template
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_PAID_ROUTE_GEOIP_URL_TEMPLATE);
            let url = paid_route_geoip_url(template, realized_ip.expect("realized ip"));
            match paid_exit_probe_fetch_text(&client, &url).await {
                Ok(body) => {
                    let (country, asn) = parse_paid_route_geoip_response(&body);
                    (country, asn, None)
                }
                Err(error) => (None, None, Some(error.to_string())),
            }
        };

    let mut measurement =
        build_paid_route_probe_measurement(samples, observed_country_code, observed_asn, now_unix)?;
    let bandwidth_error = paid_exit_probe_bandwidth(&client, args, &mut measurement).await;
    Ok((measurement, geoip_error, bandwidth_error))
}

fn paid_exit_probe_stun_servers(args: &PaidExitProbeArgs, app: &AppConfig) -> Vec<String> {
    if args.no_stun {
        return Vec::new();
    }

    let configured = if args.stun_servers.is_empty() {
        &app.nat.stun_servers
    } else {
        &args.stun_servers
    };
    configured
        .iter()
        .map(|server| server.trim())
        .filter(|server| !server.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

async fn paid_exit_probe_public_ip_sample(
    client: &reqwest::Client,
    ip_url: &str,
    stun_server: Option<&str>,
    timeout: Duration,
) -> PaidRouteProbeSample {
    let mut stun_error = None;
    if let Some(stun_server) = stun_server {
        let started = Instant::now();
        match paid_exit_probe_stun_public_ip(stun_server, timeout).await {
            Ok(ip) => return PaidRouteProbeSample::success(ip, elapsed_ms_u32(started.elapsed())),
            Err(error) => stun_error = Some(error.to_string()),
        }
    }

    let started = Instant::now();
    match paid_exit_probe_fetch_text(client, ip_url).await {
        Ok(body) => match parse_paid_route_public_ip_response(&body) {
            Some(ip) => PaidRouteProbeSample::success(ip, elapsed_ms_u32(started.elapsed())),
            None => {
                let message = "public IP response did not contain an IP";
                if let Some(stun_error) = stun_error {
                    PaidRouteProbeSample::failure(format!("stun: {stun_error}; https: {message}"))
                } else {
                    PaidRouteProbeSample::failure(message)
                }
            }
        },
        Err(error) => {
            if let Some(stun_error) = stun_error {
                PaidRouteProbeSample::failure(format!("stun: {stun_error}; https: {error}"))
            } else {
                PaidRouteProbeSample::failure(error.to_string())
            }
        }
    }
}

async fn paid_exit_probe_stun_public_ip(server: &str, timeout: Duration) -> Result<String> {
    let server = server.to_string();
    tokio::task::spawn_blocking(move || paid_exit_probe_stun_public_ip_blocking(&server, timeout))
        .await
        .context("paid exit STUN probe task failed")?
}

fn paid_exit_probe_stun_public_ip_blocking(server: &str, timeout: Duration) -> Result<String> {
    let addr = paid_exit_stun_socket_addr(server)?;
    let bind_addr = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket =
        UdpSocket::bind(bind_addr).context("failed to bind paid exit STUN probe socket")?;
    socket
        .set_read_timeout(Some(timeout))
        .context("failed to set paid exit STUN read timeout")?;
    socket
        .set_write_timeout(Some(timeout))
        .context("failed to set paid exit STUN write timeout")?;

    let transaction_id = paid_route_stun_transaction_id();
    let request = paid_route_stun_binding_request(transaction_id);
    socket
        .send_to(&request, addr)
        .with_context(|| format!("failed to send paid exit STUN probe to {server}"))?;

    let mut response = [0_u8; 1500];
    let (len, _) = socket
        .recv_from(&mut response)
        .with_context(|| format!("failed to receive paid exit STUN response from {server}"))?;
    parse_paid_route_stun_binding_response(&response[..len], transaction_id)
}

fn paid_exit_stun_socket_addr(server: &str) -> Result<SocketAddr> {
    let (host, port) = paid_route_stun_host_port(server)
        .ok_or_else(|| anyhow!("invalid paid exit STUN server '{server}'"))?;
    (host.as_str(), port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve paid exit STUN server {server}"))?
        .next()
        .ok_or_else(|| anyhow!("paid exit STUN server {server} did not resolve"))
}

async fn paid_exit_probe_bandwidth(
    client: &reqwest::Client,
    args: &PaidExitProbeArgs,
    measurement: &mut PaidRouteProbeMeasurement,
) -> Option<String> {
    if args.no_bandwidth || args.bandwidth_bytes == 0 {
        return None;
    }

    let bytes = args.bandwidth_bytes;
    let download_base = args
        .download_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PAID_ROUTE_DOWNLOAD_URL);
    let download_url = paid_route_download_url(download_base, bytes);
    let mut errors = Vec::new();

    match paid_exit_probe_download_bps(client, &download_url).await {
        Ok(bps) => measurement.quality.down_bps = Some(bps),
        Err(error) => errors.push(format!("download: {error}")),
    }

    let upload_url = args
        .upload_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PAID_ROUTE_UPLOAD_URL);
    match paid_exit_probe_upload_bps(client, upload_url, bytes).await {
        Ok(bps) => measurement.quality.up_bps = Some(bps),
        Err(error) => errors.push(format!("upload: {error}")),
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    }
}

async fn paid_exit_probe_download_bps(client: &reqwest::Client, url: &str) -> Result<u64> {
    let started = Instant::now();
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("paid exit bandwidth endpoint returned an error for {url}"))?;
    let body = response
        .bytes()
        .await
        .with_context(|| format!("failed to read bandwidth response from {url}"))?;
    paid_route_bandwidth_bps(body.len() as u64, started.elapsed())
        .ok_or_else(|| anyhow!("download bandwidth sample was empty or too fast"))
}

async fn paid_exit_probe_upload_bps(
    client: &reqwest::Client,
    url: &str,
    bytes: u64,
) -> Result<u64> {
    let len = usize::try_from(bytes).context("paid exit bandwidth byte count is too large")?;
    let body = vec![0_u8; len];
    let started = Instant::now();
    client
        .post(url)
        .body(body)
        .send()
        .await
        .with_context(|| format!("failed to upload to {url}"))?
        .error_for_status()
        .with_context(|| format!("paid exit upload endpoint returned an error for {url}"))?;
    paid_route_bandwidth_bps(bytes, started.elapsed())
        .ok_or_else(|| anyhow!("upload bandwidth sample was empty or too fast"))
}

async fn paid_exit_probe_fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("paid exit probe endpoint returned an error for {url}"))?;
    response
        .text()
        .await
        .with_context(|| format!("failed to read response from {url}"))
}

fn elapsed_ms_u32(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

fn paid_exit_probe_result_json(result: &PaidExitProbeResult) -> serde_json::Value {
    json!({
        "store_path": result.store_path.display().to_string(),
        "measurement": result.measurement,
        "probe": result.probe,
        "geoip_error": result.geoip_error,
        "bandwidth_error": result.bandwidth_error,
    })
}

fn print_paid_exit_probe_result(result: &PaidExitProbeResult) {
    println!("paid_exit_probe_session: {}", result.probe.session_id);
    println!("store: {}", result.store_path.display());
    println!("changed: {}", result.probe.changed);
    println!(
        "realized_exit_ip: {}",
        display_or_none(
            result
                .measurement
                .realized_exit_ip
                .as_deref()
                .unwrap_or_default()
        )
    );
    println!(
        "observed_country: {}",
        display_or_none(
            result
                .measurement
                .observed_country_code
                .as_deref()
                .unwrap_or_default()
        )
    );
    println!(
        "observed_asn: {}",
        result
            .measurement
            .observed_asn
            .map(|asn| asn.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "quality: {}",
        paid_exit_quality_text(Some(&result.measurement.quality))
    );
    println!(
        "samples: {} ok, {} failed",
        result.measurement.success_count(),
        result.measurement.failure_count()
    );
    if let Some(error) = result.geoip_error.as_deref() {
        println!("geoip_error: {error}");
    }
    if let Some(error) = result.bandwidth_error.as_deref() {
        println!("bandwidth_error: {error}");
    }
}

fn paid_exit_record_probe_command(args: PaidExitRecordProbeArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_record_probe_once(args)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&paid_exit_record_probe_result_json(&result))?
        );
    } else {
        print_paid_exit_record_probe_result(&result);
    }

    Ok(())
}

fn paid_exit_record_probe_once(args: PaidExitRecordProbeArgs) -> Result<PaidExitRecordProbeResult> {
    let config_path = args.config.clone().unwrap_or_else(default_config_path);
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let quality = paid_exit_probe_quality_from_args(&args);
    let result = store.update_session_probe(UpdatePaidRouteSessionProbeRequest {
        session_id: args.session,
        realized_exit_ip: args.realized_exit_ip,
        observed_country_code: args.observed_country_code,
        observed_asn: args.observed_asn,
        quality,
        now_unix: unix_timestamp(),
    })?;

    if result.changed {
        write_paid_route_store(&store_path, &store)?;
        if !args.no_reload_daemon {
            maybe_reload_running_daemon(&config_path);
        }
    }

    Ok(PaidExitRecordProbeResult {
        store_path,
        probe: result,
    })
}

fn paid_exit_probe_quality_from_args(
    args: &PaidExitRecordProbeArgs,
) -> Option<PaidRouteQualityMetrics> {
    let quality = PaidRouteQualityMetrics {
        latency_ms: args.latency_ms,
        jitter_ms: args.jitter_ms,
        packet_loss_ppm: args.packet_loss_ppm,
        down_bps: args.down_bps,
        up_bps: args.up_bps,
        uptime_secs: args.uptime_secs,
        last_seen_unix: args.last_seen_unix,
    };
    if quality.is_empty() {
        None
    } else {
        Some(quality)
    }
}

fn paid_exit_record_probe_result_json(result: &PaidExitRecordProbeResult) -> serde_json::Value {
    json!({
        "store_path": result.store_path.display().to_string(),
        "probe": result.probe,
    })
}

fn print_paid_exit_record_probe_result(result: &PaidExitRecordProbeResult) {
    println!("paid_exit_probe_session: {}", result.probe.session_id);
    println!("store: {}", result.store_path.display());
    println!("changed: {}", result.probe.changed);
    println!(
        "realized_exit_ip: {}",
        display_or_none(result.probe.realized_exit_ip.as_deref().unwrap_or_default())
    );
    println!(
        "observed_country: {}",
        display_or_none(
            result
                .probe
                .observed_country_code
                .as_deref()
                .unwrap_or_default()
        )
    );
    println!(
        "observed_asn: {}",
        result
            .probe
            .observed_asn
            .map(|asn| asn.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "quality: {}",
        paid_exit_quality_text(result.probe.quality.as_ref())
    );
}

async fn paid_exit_create_payment_command(args: PaidExitCreatePaymentArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let buyer_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode buyer npub")?;
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let now_unix = unix_timestamp();
    let mut changed = false;
    let mut wallet_open_json = None;
    let mut wallet_sign_json = None;
    let result = if args.sign_from_wallet {
        let signer = FileSpilmanPaymentSigner::load(&paid_exit_wallet_data_dir(&config_path))
            .map_err(|error| anyhow!("{error}"))?;
        let result = store.build_buyer_signed_payment_envelope(
            &signer,
            BuildPaidRouteBuyerSignedPaymentEnvelopeRequest {
                session_id: args.session.clone(),
                buyer_npub,
                kind: args.kind.into(),
                delivered_units: args.delivered_units,
                paid_msat: args.paid_msat,
                now_unix,
            },
        )?;
        changed |= result.changed;
        wallet_sign_json = Some(json!({
            "source": "spilman-client-store",
            "data_dir": paid_exit_wallet_data_dir(&config_path).display().to_string(),
        }));
        result
    } else {
        let (payment, paid_msat) = if args.open_from_wallet {
            if args.kind != PaidExitCreatePaymentKind::ChannelOpen {
                return Err(anyhow!(
                    "--open-from-wallet currently creates channel_open payments; pass --kind channel-open"
                ));
            }
            let session_record = store.sessions.get(&args.session).cloned().ok_or_else(|| {
                anyhow!("paid exit buyer session {} does not exist", args.session)
            })?;
            let lease_record = store
                .leases
                .get(&session_record.session.lease_id)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "paid exit lease {} does not exist",
                        session_record.session.lease_id
                    )
                })?;
            let channel_record = store
                .channels
                .get(&session_record.session.payment.channel_id)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "paid exit channel {} does not exist",
                        session_record.session.payment.channel_id
                    )
                })?;
            let quote_record = store
                .quotes
                .get(&lease_record.lease.quote_id)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "paid exit quote {} does not exist",
                        lease_record.lease.quote_id
                    )
                })?;
            let mint_url = args
                .mint
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| channel_record.mint_url.clone());
            if mint_url.trim().is_empty() {
                return Err(anyhow!(
                    "paid exit session has no mint; pass --mint for --open-from-wallet"
                ));
            }
            let cashu_unit = if session_record.session.payment.cashu_unit.trim().is_empty() {
                "sat".to_string()
            } else {
                session_record.session.payment.cashu_unit.clone()
            };
            let keyset_info_json =
                read_optional_paid_exit_keyset_info(args.keyset_info, args.keyset_info_file)?;
            let opened = open_streaming_route_cashu_spilman_channel_from_wallet(
                &paid_exit_wallet_data_dir(&config_path),
                StreamingRouteOpenCashuSpilmanChannelFromWalletRequest {
                    mint_url,
                    receiver_pubkey_hex: quote_record.quote.receiver_pubkey_hex,
                    capacity_sat: session_record.session.payment.capacity_sat,
                    expiry_unix: channel_record.expires_at_unix,
                    max_amount_per_output: args.max_amount_per_output,
                    unit: cashu_unit,
                    opening_paid_msat: args
                        .paid_msat
                        .unwrap_or(session_record.session.payment.paid_msat),
                    keyset_id: args.keyset_id,
                    keyset_info_json,
                },
            )
            .await?;
            let attach =
                store.attach_buyer_spilman_channel(AttachPaidRouteBuyerSpilmanChannelRequest {
                    session_id: args.session.clone(),
                    channel_id: opened.channel.channel_id.clone(),
                    cashu_unit: opened.channel.unit.clone(),
                    capacity_sat: opened.channel.capacity_sat,
                    paid_msat: Some(opened.channel.opening_paid_msat),
                    payment: opened.channel.payment.clone(),
                    now_unix,
                })?;
            changed |= attach.changed;
            let payment = opened.channel.payment.clone();
            let opened_paid_msat = opened.channel.opening_paid_msat;
            wallet_open_json = Some(json!({
                "channel": opened.channel,
                "wallet_send": {
                    "mint_url": opened.wallet_send.mint_url,
                    "unit": opened.wallet_send.unit,
                    "amount_sat": opened.wallet_send.amount_sat,
                    "send_fee_sat": opened.wallet_send.send_fee_sat,
                    "operation_id": opened.wallet_send.operation_id,
                },
                "attached": attach,
            }));
            (payment, Some(opened_paid_msat))
        } else {
            let payment_json = read_paid_exit_spilman_payment(args.payment, args.payment_stdin)?;
            let payment: CashuSpilmanPayment = serde_json::from_str(&payment_json)
                .context("failed to decode Cashu Spilman payment JSON")?;
            (payment, args.paid_msat)
        };
        let result =
            store.build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
                session_id: args.session.clone(),
                buyer_npub,
                kind: args.kind.into(),
                payment,
                delivered_units: args.delivered_units,
                paid_msat,
                now_unix,
            })?;
        changed |= result.changed;
        result
    };
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "payment": result,
                "wallet_open": wallet_open_json,
                "wallet_sign": wallet_sign_json,
            }))?
        );
    } else {
        if let Some(wallet_open) = wallet_open_json.as_ref() {
            if let Some(wallet_send) = wallet_open.get("wallet_send") {
                let amount_sat = wallet_send["amount_sat"].as_u64().unwrap_or_default();
                let fee_sat = wallet_send["send_fee_sat"].as_u64().unwrap_or_default();
                println!(
                    "wallet_funding: amount={} fee={} operation={}",
                    paid_exit_sat_text(amount_sat),
                    paid_exit_sat_text(fee_sat),
                    wallet_send["operation_id"].as_str().unwrap_or_default()
                );
            }
        }
        if let Some(wallet_sign) = wallet_sign_json.as_ref() {
            println!(
                "wallet_sign: {}",
                wallet_sign["source"].as_str().unwrap_or_default()
            );
        }
        println!("paid_exit_payment: {}", result.payload_type);
        println!("session: {}", result.session_id);
        println!("seller: {}", result.seller_npub);
        println!("offer: {}", result.offer_id);
        println!("channel: {}", result.channel_id);
        println!(
            "routing: state={} allow={} paid={} due={} unpaid={} usage={}",
            result.state.as_str(),
            result.allow_routing,
            paid_exit_msat_text(result.paid_msat),
            paid_exit_msat_text(result.amount_due_msat),
            paid_exit_msat_text(result.unpaid_msat),
            paid_exit_usage_text(0, 0, result.delivered_units)
        );
        println!("store: {} changed={}", store_path.display(), result.changed);
        println!(
            "envelope: {}",
            serde_json::to_string(&result.envelope)
                .context("failed to encode paid route payment envelope")?
        );
    }

    Ok(())
}

fn paid_exit_create_token_lease_command(args: PaidExitCreateTokenLeaseArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let buyer_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode buyer npub")?;
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let token = read_paid_exit_wallet_token(args.token, args.token_stdin)?;
    let mint_url = args
        .mint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_mint_url)
        .transpose()?
        .unwrap_or_default();
    let result =
        store.build_buyer_token_lease_envelope(BuildPaidRouteBuyerTokenLeaseEnvelopeRequest {
            session_id: args.session.clone(),
            buyer_npub,
            mint_url,
            cashu_unit: args.unit,
            amount: args.amount,
            paid_msat: args.paid_msat,
            token,
            expires_at_unix: args.expires_at_unix,
            now_unix: unix_timestamp(),
        })?;
    if result.changed {
        write_paid_route_store(&store_path, &store)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "payment": result,
            }))?
        );
    } else {
        println!("paid_exit_payment: {}", result.payload_type);
        println!("session: {}", result.session_id);
        println!("seller: {}", result.seller_npub);
        println!("offer: {}", result.offer_id);
        println!("channel: {}", result.channel_id);
        println!(
            "routing: state={} allow={} paid={} due={} unpaid={} usage={}",
            result.state.as_str(),
            result.allow_routing,
            paid_exit_msat_text(result.paid_msat),
            paid_exit_msat_text(result.amount_due_msat),
            paid_exit_msat_text(result.unpaid_msat),
            paid_exit_usage_text(0, 0, result.delivered_units)
        );
        println!("store: {} changed={}", store_path.display(), result.changed);
        println!(
            "envelope: {}",
            serde_json::to_string(&result.envelope)
                .context("failed to encode paid route token lease envelope")?
        );
    }

    Ok(())
}

#[derive(Debug, Default)]
struct PaidExitStreamPaymentUpdatesResult {
    signed: Vec<serde_json::Value>,
    errors: Vec<serde_json::Value>,
    changed: bool,
}

impl PaidExitStreamPaymentUpdatesResult {
    fn persisted_count(&self) -> usize {
        self.signed
            .iter()
            .filter(|entry| entry["persisted"].as_bool().unwrap_or_default())
            .count()
    }
}

async fn paid_exit_stream_payment_updates_with_signer<S: CashuSpilmanPaymentSigner>(
    app: &AppConfig,
    keys: &Keys,
    store: &mut PaidRouteStore,
    signer: &S,
    buyer_npub: &str,
    due: Vec<PaidRouteBuyerPaymentUpdateDue>,
    relays: &[String],
    publish: bool,
    now_unix: u64,
) -> PaidExitStreamPaymentUpdatesResult {
    let mut result = PaidExitStreamPaymentUpdatesResult::default();

    for update_due in due {
        let signed_update = store.build_buyer_signed_payment_envelope_for_due(
            signer,
            buyer_npub,
            &update_due,
            now_unix,
        );
        let signed_update = match signed_update {
            Ok(signed_update) => signed_update,
            Err(error) => {
                result.errors.push(json!({
                    "due": update_due.clone(),
                    "error": error.to_string(),
                }));
                continue;
            }
        };
        let next_store = signed_update.store;
        let payment = signed_update.payment;
        let payment_changed = payment.changed;

        let mut persisted = !publish;
        let publish_result = if publish {
            match gift_wrap_paid_route_payment(&payment.envelope, keys).await {
                Ok(event) => {
                    let event_id = event.id.to_string();
                    match publish_paid_exit_payment_to_relays(app, &event, relays).await {
                        Ok(publish_result) => {
                            persisted =
                                publish_result["success_count"].as_u64().unwrap_or_default() > 0;
                            if !persisted {
                                result.errors.push(json!({
                                    "due": update_due.clone(),
                                    "session_id": payment.session_id,
                                    "error": "payment update was not accepted by any relay",
                                }));
                            }
                            Some(json!({
                                "event_id": event_id,
                                "result": publish_result,
                            }))
                        }
                        Err(error) => {
                            result.errors.push(json!({
                                "due": update_due.clone(),
                                "session_id": payment.session_id,
                                "error": error.to_string(),
                            }));
                            None
                        }
                    }
                }
                Err(error) => {
                    result.errors.push(json!({
                        "due": update_due.clone(),
                        "session_id": payment.session_id,
                        "error": error.to_string(),
                    }));
                    None
                }
            }
        } else {
            None
        };

        if persisted {
            result.changed |= payment_changed;
            *store = next_store;
        }
        result.signed.push(json!({
            "due": update_due,
            "payment": payment,
            "publish": publish_result,
            "persisted": persisted,
        }));
    }

    result
}

async fn paid_exit_stream_payments_command(args: PaidExitStreamPaymentsArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let keys = app.nostr_keys()?;
    let buyer_npub = keys
        .public_key()
        .to_bech32()
        .context("failed to encode buyer npub")?;
    let relays = if args.publish {
        paid_exit_relay_urls(&app, &args.relays)
    } else {
        Vec::new()
    };
    if args.publish && relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment publishing"
        ));
    }

    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let now_unix = unix_timestamp();
    let mut due = store.buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
        now_unix,
        min_increment_msat: args.min_increment_msat,
    });
    let total_due_count = due.len();
    if args.limit > 0 && due.len() > args.limit {
        due.truncate(args.limit);
    }
    let selected_due_count = due.len();

    let result = if due.is_empty() {
        PaidExitStreamPaymentUpdatesResult::default()
    } else {
        let signer = FileSpilmanPaymentSigner::load(&paid_exit_wallet_data_dir(&config_path))
            .map_err(|error| anyhow!("{error}"))?;
        paid_exit_stream_payment_updates_with_signer(
            &app,
            &keys,
            &mut store,
            &signer,
            &buyer_npub,
            due,
            &relays,
            args.publish,
            now_unix,
        )
        .await
    };
    let changed = result.changed;

    if changed {
        write_paid_route_store(&store_path, &store)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "wallet_sign": {
                    "source": "spilman-client-store",
                    "data_dir": paid_exit_wallet_data_dir(&config_path).display().to_string(),
                },
                "publish_requested": args.publish,
                "relays": relays,
                "total_due_count": total_due_count,
                "processed_due_count": selected_due_count,
                "signed_count": result.signed.len(),
                "persisted_count": result.persisted_count(),
                "error_count": result.errors.len(),
                "changed": changed,
                "signed": result.signed,
                "errors": result.errors,
            }))?
        );
    } else {
        println!(
            "paid_exit_stream_payments: signed={} errors={} due={} changed={}",
            result.signed.len(),
            result.errors.len(),
            total_due_count,
            changed
        );
        if args.publish {
            println!("relays: {}", relays.join(", "));
        }
        for entry in &result.signed {
            let payment = &entry["payment"];
            let paid_msat = payment["paid_msat"].as_u64().unwrap_or_default();
            let due_msat = payment["amount_due_msat"].as_u64().unwrap_or_default();
            let unpaid_msat = payment["unpaid_msat"].as_u64().unwrap_or_default();
            println!(
                "session: {} seller: {} paid={} due={} unpaid={}",
                payment["session_id"].as_str().unwrap_or_default(),
                payment["seller_npub"].as_str().unwrap_or_default(),
                paid_exit_msat_text(paid_msat),
                paid_exit_msat_text(due_msat),
                paid_exit_msat_text(unpaid_msat)
            );
            println!(
                "persisted: {}",
                entry["persisted"].as_bool().unwrap_or_default()
            );
            println!(
                "envelope: {}",
                serde_json::to_string(&payment["envelope"])
                    .context("failed to encode paid route payment envelope")?
            );
            if let Some(event_id) = entry["publish"]["event_id"].as_str() {
                println!("published_event: {event_id}");
            }
        }
        for entry in &result.errors {
            println!(
                "error: session={} {}",
                entry["due"]["session_id"].as_str().unwrap_or_default(),
                entry["error"].as_str().unwrap_or_default()
            );
        }
        println!("store: {}", store_path.display());
    }

    Ok(())
}

#[derive(Debug)]
struct PaidExitSettleResult {
    payment: BuildPaidRouteBuyerPaymentEnvelopeResult,
    wallet_sign: serde_json::Value,
    publish_requested: bool,
    relays: Vec<String>,
    publish: Option<serde_json::Value>,
    persisted: bool,
}

async fn paid_exit_settle_with_signer<S: CashuSpilmanPaymentSigner>(
    app: &AppConfig,
    keys: &Keys,
    store: &mut PaidRouteStore,
    signer: &S,
    session_id: &str,
    relays: &[String],
    publish: bool,
    wallet_data_dir: &Path,
    now_unix: u64,
) -> Result<PaidExitSettleResult> {
    if publish && relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment publishing"
        ));
    }
    let buyer_npub = keys
        .public_key()
        .to_bech32()
        .context("failed to encode buyer npub")?;
    let before = store.clone();
    let payment = store.build_buyer_signed_payment_envelope(
        signer,
        BuildPaidRouteBuyerSignedPaymentEnvelopeRequest {
            session_id: session_id.trim().to_string(),
            buyer_npub,
            kind: BuildPaidRouteBuyerPaymentEnvelopeKind::CooperativeClose,
            delivered_units: None,
            paid_msat: None,
            now_unix,
        },
    )?;
    let mut persisted = !publish;
    let publish_result = if publish {
        let event = match gift_wrap_paid_route_payment(&payment.envelope, keys).await {
            Ok(event) => event,
            Err(error) => {
                *store = before;
                return Err(error);
            }
        };
        let event_id = event.id.to_string();
        let publish_result = match publish_paid_exit_payment_to_relays(app, &event, relays).await {
            Ok(result) => result,
            Err(error) => {
                *store = before;
                return Err(error);
            }
        };
        persisted = publish_result["success_count"].as_u64().unwrap_or_default() > 0;
        Some(json!({
            "event_id": event_id,
            "result": publish_result,
        }))
    } else {
        None
    };
    if !persisted {
        *store = before;
    }

    Ok(PaidExitSettleResult {
        payment,
        wallet_sign: json!({
            "source": "spilman-client-store",
            "data_dir": wallet_data_dir.display().to_string(),
        }),
        publish_requested: publish,
        relays: relays.to_vec(),
        publish: publish_result,
        persisted,
    })
}

async fn paid_exit_settle_command(args: PaidExitSettleArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let keys = app.nostr_keys()?;
    let publish = !args.no_publish;
    let relays = if publish {
        paid_exit_relay_urls(&app, &args.relays)
    } else {
        Vec::new()
    };
    if publish && relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment publishing"
        ));
    }

    let store_path = paid_route_store_file_path(&config_path);
    let wallet_data_dir = paid_exit_wallet_data_dir(&config_path);
    let signer =
        FileSpilmanPaymentSigner::load(&wallet_data_dir).map_err(|error| anyhow!("{error}"))?;
    let mut store = load_paid_route_store(&store_path)?;
    let result = paid_exit_settle_with_signer(
        &app,
        &keys,
        &mut store,
        &signer,
        &args.session,
        &relays,
        publish,
        &wallet_data_dir,
        unix_timestamp(),
    )
    .await?;

    let changed = result.persisted && result.payment.changed;
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "wallet_sign": result.wallet_sign,
                "publish_requested": result.publish_requested,
                "relays": result.relays,
                "payment": result.payment,
                "publish": result.publish,
                "persisted": result.persisted,
                "changed": changed,
            }))?
        );
    } else {
        println!("paid_exit_settle: {}", result.payment.session_id);
        println!("seller: {}", result.payment.seller_npub);
        println!("offer: {}", result.payment.offer_id);
        println!("channel: {}", result.payment.channel_id);
        println!(
            "routing: state={} allow={} paid={} due={} unpaid={} usage={}",
            result.payment.state.as_str(),
            result.payment.allow_routing,
            paid_exit_msat_text(result.payment.paid_msat),
            paid_exit_msat_text(result.payment.amount_due_msat),
            paid_exit_msat_text(result.payment.unpaid_msat),
            paid_exit_usage_text(0, 0, result.payment.delivered_units)
        );
        println!(
            "wallet_sign: {}",
            result.wallet_sign["source"].as_str().unwrap_or_default()
        );
        if result.publish_requested {
            println!("relays: {}", result.relays.join(", "));
            if let Some(publish) = result.publish.as_ref() {
                println!(
                    "published: {} success, {} failed",
                    publish["result"]["success_count"]
                        .as_u64()
                        .unwrap_or_default(),
                    publish["result"]["failed_count"]
                        .as_u64()
                        .unwrap_or_default()
                );
                println!(
                    "published_event: {}",
                    publish["event_id"].as_str().unwrap_or_default()
                );
            }
        } else {
            println!("published: false");
            println!(
                "envelope: {}",
                serde_json::to_string(&result.payment.envelope)
                    .context("failed to encode paid route cooperative close envelope")?
            );
        }
        println!("persisted: {}", result.persisted);
        println!("store: {} changed={}", store_path.display(), changed);
    }

    Ok(())
}

async fn paid_exit_apply_payment_command(args: PaidExitApplyPaymentArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    if !app.paid_exit.enabled {
        return Err(anyhow!("paid exit selling is disabled"));
    }
    let seller_npub = app
        .nostr_keys()?
        .public_key()
        .to_bech32()
        .context("failed to encode seller npub")?;
    let envelope_json = read_paid_exit_payment_envelope(args.envelope, args.envelope_stdin)?;
    let envelope: StreamingRoutePaymentEnvelope = serde_json::from_str(&envelope_json)
        .context("failed to decode paid route payment envelope JSON")?;
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let (spilman_receiver, spilman_receiver_error) =
        try_load_paid_exit_spilman_receiver(&config_path, &app.paid_exit).await;
    let spilman_receiver_processing = spilman_receiver.is_some();
    let result = apply_paid_route_seller_payment(
        &mut store,
        ApplyPaidRouteSellerPaymentRequest {
            envelope,
            seller_npub,
            config: app.paid_exit.clone(),
            now_unix: unix_timestamp(),
        },
        spilman_receiver.as_ref(),
    )?;
    if result.changed {
        write_paid_route_store(&store_path, &store)?;
    }
    let daemon_reload_attempted = result.changed && !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "payment": result,
                "spilman_receiver_processing": spilman_receiver_processing,
                "spilman_receiver_mode": paid_exit_spilman_receiver_mode(spilman_receiver_processing),
                "spilman_receiver_validation": spilman_receiver_processing,
                "spilman_receiver_error": spilman_receiver_error,
                "daemon_reload_attempted": daemon_reload_attempted,
                "status": paid_exit_status_snapshot_json(&app, &store_path, &store),
            }))?
        );
    } else {
        println!("paid_exit_payment: {}", result.payload_type);
        println!("buyer: {}", result.buyer_npub);
        println!("seller: {}", result.seller_npub);
        println!("service: {}", result.service_id);
        println!("lease: {}", result.lease_id);
        println!("channel: {}", result.channel_id);
        println!(
            "routing: state={} allow={} paid={} due={} unpaid={} usage={}",
            result.state.as_str(),
            result.allow_routing,
            paid_exit_msat_text(result.paid_msat),
            paid_exit_msat_text(result.amount_due_msat),
            paid_exit_msat_text(result.unpaid_msat),
            paid_exit_usage_text(0, 0, result.delivered_units)
        );
        println!("store: {} changed={}", store_path.display(), result.changed);
        println!(
            "spilman_receiver_processing: {}",
            paid_exit_spilman_receiver_mode(spilman_receiver_processing)
        );
        if let Some(error) = spilman_receiver_error {
            println!("spilman_receiver_error: {error}");
        }
        println!(
            "daemon_reload: {}",
            if daemon_reload_attempted {
                "attempted"
            } else {
                "skipped"
            }
        );
    }

    Ok(())
}

async fn paid_exit_send_payment_command(args: PaidExitSendPaymentArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let relays = paid_exit_relay_urls(&app, &args.relays);
    let envelope_json = read_paid_exit_payment_envelope(args.envelope, args.envelope_stdin)?;
    let envelope: StreamingRoutePaymentEnvelope = serde_json::from_str(&envelope_json)
        .context("failed to decode paid route payment envelope JSON")?;
    let keys = app.nostr_keys()?;
    let event = gift_wrap_paid_route_payment(&envelope, &keys).await?;
    let publish = publish_paid_exit_payment_to_relays(&app, &event, &relays).await?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "event_id": event.id.to_string(),
                "seller": envelope.seller,
                "buyer": envelope.buyer,
                "service_id": envelope.service_id,
                "lease_id": envelope.lease_id,
                "channel_id": envelope.channel_id(),
                "relays": relays,
                "publish": publish,
            }))?
        );
    } else {
        println!("paid_exit_payment_sent: {}", event.id);
        println!("buyer: {}", envelope.buyer);
        println!("seller: {}", envelope.seller);
        println!("service: {}", envelope.service_id);
        println!("lease: {}", envelope.lease_id);
        println!("channel: {}", envelope.channel_id());
        println!("relays: {}", relays.join(", "));
        println!(
            "published: {} success, {} failed",
            publish["success_count"].as_u64().unwrap_or_default(),
            publish["failed_count"].as_u64().unwrap_or_default()
        );
    }

    Ok(())
}

async fn paid_exit_receive_payments_command(args: PaidExitReceivePaymentsArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    if !app.paid_exit.enabled {
        return Err(anyhow!("paid exit selling is disabled"));
    }
    let keys = app.nostr_keys()?;
    let seller_npub = keys
        .public_key()
        .to_bech32()
        .context("failed to encode seller npub")?;
    let relays = paid_exit_relay_urls(&app, &args.relays);
    let since_unix = if args.since_secs == 0 {
        None
    } else {
        Some(unix_timestamp().saturating_sub(args.since_secs))
    };
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let mut seen_events = HashSet::new();
    let mut applied = Vec::new();
    let mut errors = Vec::new();
    let (spilman_receiver, spilman_receiver_error) =
        try_load_paid_exit_spilman_receiver(&config_path, &app.paid_exit).await;
    let spilman_receiver_processing = spilman_receiver.is_some();

    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment receiving"
        ));
    }

    let client = Client::new(keys.clone());
    for relay in &relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    let mut notifications = client.notifications();
    client
        .subscribe_to(
            relays.clone(),
            paid_route_payment_filter(keys.public_key(), args.limit, since_unix),
            None,
        )
        .await
        .map_err(|error| anyhow!("failed to subscribe paid exit payments: {error}"))?;

    let timeout = tokio::time::sleep(Duration::from_secs(args.duration_secs));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = &mut timeout => break,
            notification = notifications.recv() => {
                match notification {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        let event = (*event).clone();
                        let event_id = event.id.to_string();
                        if !seen_events.insert(event_id.clone()) {
                            continue;
                        }
                        match unwrap_paid_route_payment(&event, &keys).await {
                            Ok(envelope) => {
                                match apply_paid_route_seller_payment(
                                    &mut store,
                                    ApplyPaidRouteSellerPaymentRequest {
                                        envelope,
                                        seller_npub: seller_npub.clone(),
                                        config: app.paid_exit.clone(),
                                        now_unix: unix_timestamp(),
                                    },
                                    spilman_receiver.as_ref(),
                                ) {
                                    Ok(result) => applied.push(json!({
                                        "event_id": event_id,
                                        "payment": result,
                                    })),
                                    Err(error) => errors.push(json!({
                                        "event_id": event_id,
                                        "error": error.to_string(),
                                    })),
                                }
                            }
                            Err(error) => errors.push(json!({
                                "event_id": event_id,
                                "error": error.to_string(),
                            })),
                        }
                        if args.limit > 0 && seen_events.len() >= args.limit {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        errors.push(json!({
                            "event_id": "",
                            "error": format!("payment subscription lagged by {count} events"),
                        }));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    client.disconnect().await;

    let changed = applied
        .iter()
        .any(|entry| entry["payment"]["changed"].as_bool().unwrap_or_default());
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }
    let daemon_reload_attempted = changed && !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "seller": seller_npub,
                "relays": relays,
                "received_count": seen_events.len(),
                "applied_count": applied.len(),
                "error_count": errors.len(),
                "changed": changed,
                "spilman_receiver_processing": spilman_receiver_processing,
                "spilman_receiver_mode": paid_exit_spilman_receiver_mode(spilman_receiver_processing),
                "spilman_receiver_validation": spilman_receiver_processing,
                "spilman_receiver_error": spilman_receiver_error,
                "daemon_reload_attempted": daemon_reload_attempted,
                "applied": applied,
                "errors": errors,
                "status": paid_exit_status_snapshot_json(&app, &store_path, &store),
            }))?
        );
    } else {
        println!("paid_exit_payments_received: {}", seen_events.len());
        println!("seller: {}", seller_npub);
        println!("store: {} changed={changed}", store_path.display());
        println!("applied: {}", applied.len());
        println!("errors: {}", errors.len());
        println!(
            "spilman_receiver_processing: {}",
            paid_exit_spilman_receiver_mode(spilman_receiver_processing)
        );
        if let Some(error) = spilman_receiver_error {
            println!("spilman_receiver_error: {error}");
        }
        println!(
            "daemon_reload: {}",
            if daemon_reload_attempted {
                "attempted"
            } else {
                "skipped"
            }
        );
    }

    Ok(())
}

struct PaidExitCollectChannelOutcome {
    close: CashuSpilmanReceiverCloseResult,
    wallet_collect: Option<serde_json::Value>,
    changed: bool,
}

async fn paid_exit_collect_channel_with_receiver(
    receiver: &FileSpilmanPaymentReceiver,
    wallet_data_dir: &Path,
    store_path: &Path,
    store: &mut PaidRouteStore,
    channel_id: &str,
) -> Result<PaidExitCollectChannelOutcome> {
    let close = receiver
        .close_cashu_spilman_channel(channel_id)
        .await
        .map_err(|error| anyhow!("{error}"))?;

    let changed = store.mark_seller_channel_closed(
        &close.channel_id,
        close.closed_amount.saturating_mul(1_000),
        unix_timestamp(),
    )?;
    if changed {
        write_paid_route_store(store_path, store)?;
    }
    let wallet_collect = if close.receiver_proofs_json.trim().is_empty() {
        None
    } else {
        Some(json!(
            import_payment_proofs(
                wallet_data_dir,
                &close.mint_url,
                &close.unit,
                &close.receiver_proofs_json,
            )
            .await?
        ))
    };

    Ok(PaidExitCollectChannelOutcome {
        close,
        wallet_collect,
        changed,
    })
}

async fn paid_exit_collect_command(args: PaidExitCollectArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    if !app.paid_exit.enabled {
        return Err(anyhow!("paid exit selling is disabled"));
    }

    let receiver_config = paid_exit_spilman_receiver_config(&app.paid_exit)
        .ok_or_else(|| anyhow!("no accepted Cashu mints configured"))?;
    let wallet_data_dir = paid_exit_wallet_data_dir(&config_path);
    let receiver =
        FileSpilmanPaymentReceiver::load_with_keyset_refresh(&wallet_data_dir, receiver_config)
            .await
            .map_err(|error| anyhow!("{error}"))?;

    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let outcome = paid_exit_collect_channel_with_receiver(
        &receiver,
        &wallet_data_dir,
        &store_path,
        &mut store,
        &args.channel,
    )
    .await?;
    let mut changed = outcome.changed;
    let overview = load_wallet_overview(&wallet_data_dir, false).await?;
    changed |= sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }
    let daemon_reload_attempted = changed && !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "wallet_data_dir": wallet_data_dir.display().to_string(),
                "spilman_close": paid_exit_spilman_close_result_json(&outcome.close),
                "wallet_collect": outcome.wallet_collect,
                "cashu": cashu_wallet_overview_json(&overview),
                "changed": changed,
                "daemon_reload_attempted": daemon_reload_attempted,
                "status": paid_exit_status_snapshot_json(&app, &store_path, &store),
            }))?
        );
    } else {
        println!("paid_exit_collect: {}", outcome.close.channel_id);
        println!(
            "collected: {}",
            paid_exit_sat_text(outcome.close.receiver_sum)
        );
        println!(
            "buyer_refund: {}",
            paid_exit_sat_text(outcome.close.sender_sum)
        );
        println!(
            "receiver_proofs: {}",
            if outcome.close.receiver_proofs_json.trim().is_empty() {
                "missing"
            } else {
                "saved"
            }
        );
        let wallet_collect_amount_sat =
            paid_exit_wallet_collect_amount_sat(outcome.wallet_collect.as_ref());
        match outcome.wallet_collect {
            Some(_) if wallet_collect_amount_sat > 0 => {
                println!(
                    "wallet_collected: {}",
                    paid_exit_sat_text(wallet_collect_amount_sat)
                );
            }
            Some(_) => {
                println!("wallet_collected: already imported");
            }
            None => {
                println!("wallet_collected: skipped");
            }
        }
        println!("store: {} changed={changed}", store_path.display());
        println!(
            "daemon_reload: {}",
            if daemon_reload_attempted {
                "attempted"
            } else {
                "skipped"
            }
        );
    }

    Ok(())
}

async fn paid_exit_collect_due_command(args: PaidExitCollectDueArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let wallet_data_dir = paid_exit_wallet_data_dir(&config_path);
    let mut due = store
        .seller_collection_states(&app.paid_exit, unix_timestamp())
        .into_iter()
        .filter(|state| state.auto_collect_due)
        .collect::<Vec<_>>();
    if args.limit > 0 {
        due.truncate(args.limit);
    }

    let mut collected = Vec::new();
    let mut errors = Vec::new();
    let mut changed = false;
    if !due.is_empty() {
        let receiver_config = paid_exit_spilman_receiver_config(&app.paid_exit)
            .ok_or_else(|| anyhow!("no accepted Cashu mints configured"))?;
        let receiver =
            FileSpilmanPaymentReceiver::load_with_keyset_refresh(&wallet_data_dir, receiver_config)
                .await
                .map_err(|error| anyhow!("{error}"))?;
        for state in &due {
            match paid_exit_collect_channel_with_receiver(
                &receiver,
                &wallet_data_dir,
                &store_path,
                &mut store,
                &state.channel_id,
            )
            .await
            {
                Ok(outcome) => {
                    changed |= outcome.changed;
                    collected.push(paid_exit_collect_channel_outcome_json(&outcome));
                }
                Err(error) => {
                    errors.push(json!({
                        "channel_id": state.channel_id,
                        "session_id": state.session_id,
                        "error": error.to_string(),
                    }));
                }
            }
        }
    }

    let cashu = if collected.is_empty() {
        serde_json::Value::Null
    } else {
        let overview = load_wallet_overview(&wallet_data_dir, false).await?;
        changed |= sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
        json!(cashu_wallet_overview_json(&overview))
    };
    if changed {
        write_paid_route_store(&store_path, &store)?;
    }
    let daemon_reload_attempted = changed && !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "wallet_data_dir": wallet_data_dir.display().to_string(),
                "due_count": due.len(),
                "collected_count": collected.len(),
                "error_count": errors.len(),
                "collected": collected,
                "errors": errors,
                "cashu": cashu,
                "changed": changed,
                "daemon_reload_attempted": daemon_reload_attempted,
                "status": paid_exit_status_snapshot_json(&app, &store_path, &store),
            }))?
        );
    } else if due.is_empty() {
        println!("paid_exit_collect_due: none");
    } else {
        println!(
            "paid_exit_collect_due: collected={} errors={}",
            collected.len(),
            errors.len()
        );
        for entry in &collected {
            let close = entry
                .get("spilman_close")
                .unwrap_or(&serde_json::Value::Null);
            println!(
                "  {} collected={}",
                paid_exit_json_string(close, "channel_id"),
                paid_exit_sat_text(paid_exit_json_u64(close, "receiver_amount_sat"))
            );
        }
        for entry in &errors {
            println!(
                "  {} error={}",
                paid_exit_json_string(entry, "channel_id"),
                paid_exit_json_string(entry, "error")
            );
        }
        println!("store: {} changed={changed}", store_path.display());
        println!(
            "daemon_reload: {}",
            if daemon_reload_attempted {
                "attempted"
            } else {
                "skipped"
            }
        );
    }

    Ok(())
}

fn paid_exit_collect_channel_outcome_json(
    outcome: &PaidExitCollectChannelOutcome,
) -> serde_json::Value {
    json!({
        "spilman_close": paid_exit_spilman_close_result_json(&outcome.close),
        "wallet_collect": outcome.wallet_collect,
        "changed": outcome.changed,
    })
}

fn paid_exit_wallet_collect_amount_sat(value: Option<&serde_json::Value>) -> u64 {
    value
        .and_then(|value| value.get("amount_sat"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}

fn paid_exit_json_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn paid_exit_json_u64(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}

fn paid_exit_spilman_close_result_json(
    close: &CashuSpilmanReceiverCloseResult,
) -> serde_json::Value {
    json!({
        "channel_id": close.channel_id,
        "mint_url": close.mint_url,
        "unit": close.unit,
        "closed_amount_sat": close.closed_amount,
        "closed_amount_text": paid_exit_sat_text(close.closed_amount),
        "total_value_sat": close.total_value,
        "total_value_text": paid_exit_sat_text(close.total_value),
        "receiver_amount_sat": close.receiver_sum,
        "receiver_amount_text": paid_exit_sat_text(close.receiver_sum),
        "sender_refund_sat": close.sender_sum,
        "sender_refund_text": paid_exit_sat_text(close.sender_sum),
        "receiver_proofs_saved": !close.receiver_proofs_json.trim().is_empty(),
        "sender_proofs_saved": !close.sender_proofs_json.trim().is_empty(),
        "already_closed": close.already_closed,
    })
}

const DEFAULT_PAID_EXIT_WALLET_MINT: &str = "https://mint.minibits.cash/Bitcoin";

async fn paid_exit_wallet_command(args: PaidExitWalletArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let data_dir = paid_exit_wallet_data_dir(&config_path);
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let mut changed = false;
    let json_output = args.json;

    match args.command {
        PaidExitWalletCommand::Show(show) => {
            let overview = load_wallet_overview(&data_dir, show.refresh).await?;
            changed |=
                sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
            if changed {
                write_paid_route_store(&store_path, &store)?;
            }
            let activity = if show.activity {
                Some(load_wallet_activity(&data_dir).await?)
            } else {
                None
            };

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "store_path": store_path.display().to_string(),
                        "data_dir": data_dir.display().to_string(),
                        "changed": changed,
                        "wallet": store.wallet,
                        "cashu": cashu_wallet_overview_json(&overview),
                        "activity": activity,
                    }))?
                );
            } else {
                println!("store: {} changed={changed}", store_path.display());
                println!("data_dir: {}", data_dir.display());
                print_paid_exit_wallet(&store);
                print_cashu_wallet_overview(&overview);
                if let Some(activity) = activity {
                    println!("activity: {}", activity.len());
                    for entry in activity.iter().take(20) {
                        println!(
                            "  {:?} {:?} {} mint={} id={}",
                            entry.kind,
                            entry.status,
                            paid_exit_sat_text(entry.amount_sat),
                            entry.mint_url,
                            entry.id
                        );
                    }
                }
            }
            return Ok(());
        }
        PaidExitWalletCommand::Topup(topup) => {
            let mint = paid_exit_wallet_mint(&store, topup.mint.as_deref())?;
            let quote = create_topup_quote(&data_dir, &mint, topup.amount_sat).await?;
            changed |= ensure_paid_exit_wallet_mint(&mut store, &quote.mint_url, None)?;
            if changed {
                write_paid_route_store(&store_path, &store)?;
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "store_path": store_path.display().to_string(),
                        "data_dir": data_dir.display().to_string(),
                        "changed": changed,
                        "quote": {
                            "mint_url": quote.mint_url,
                            "unit": quote.unit,
                            "amount_sat": quote.amount,
                            "quote_id": quote.quote_id,
                            "payment_request": quote.payment_request,
                            "expiry_unix": quote.expiry_unix,
                        },
                        "wallet": store.wallet,
                    }))?
                );
            } else {
                println!("topup_quote: {}", quote.quote_id);
                println!("mint: {}", quote.mint_url);
                println!("amount: {}", paid_exit_sat_text(quote.amount));
                println!("expires_at: {}", quote.expiry_unix);
                println!("invoice: {}", quote.payment_request);
            }
            return Ok(());
        }
        PaidExitWalletCommand::Receive(receive) => {
            let token = read_paid_exit_wallet_token(receive.token, receive.token_stdin)?;
            let payment = receive_payment_token(&data_dir, &token).await?;
            let overview = load_wallet_overview(&data_dir, false).await?;
            changed |=
                sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
            if changed {
                write_paid_route_store(&store_path, &store)?;
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "store_path": store_path.display().to_string(),
                        "data_dir": data_dir.display().to_string(),
                        "changed": changed,
                        "received": payment,
                        "wallet": store.wallet,
                        "cashu": cashu_wallet_overview_json(&overview),
                    }))?
                );
            } else {
                println!("received: {}", paid_exit_sat_text(payment.amount_sat));
                println!("mint: {}", payment.mint_url);
                println!("store: {} changed={changed}", store_path.display());
            }
            return Ok(());
        }
        PaidExitWalletCommand::Send(send) => {
            let mint = paid_exit_wallet_mint(&store, send.mint.as_deref())?;
            let payment = send_payment_token(&data_dir, &mint, send.amount_sat).await?;
            let overview = load_wallet_overview(&data_dir, false).await?;
            changed |=
                sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
            if changed {
                write_paid_route_store(&store_path, &store)?;
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "store_path": store_path.display().to_string(),
                        "data_dir": data_dir.display().to_string(),
                        "changed": changed,
                        "sent": payment,
                        "wallet": store.wallet,
                        "cashu": cashu_wallet_overview_json(&overview),
                    }))?
                );
            } else {
                println!(
                    "sent: {} fee={}",
                    paid_exit_sat_text(payment.amount_sat),
                    paid_exit_sat_text(payment.send_fee_sat)
                );
                println!("mint: {}", payment.mint_url);
                println!("operation_id: {}", payment.operation_id);
                println!("token: {}", payment.token);
            }
            return Ok(());
        }
        PaidExitWalletCommand::Withdraw(withdraw) => {
            let mint = paid_exit_wallet_mint(&store, withdraw.mint.as_deref())?;
            let payment = send_lightning_payment(&data_dir, &mint, &withdraw.invoice).await?;
            let overview = load_wallet_overview(&data_dir, false).await?;
            changed |=
                sync_paid_exit_wallet_store_from_cashu(&mut store, &overview, unix_timestamp());
            if changed {
                write_paid_route_store(&store_path, &store)?;
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "store_path": store_path.display().to_string(),
                        "data_dir": data_dir.display().to_string(),
                        "changed": changed,
                        "withdrawal": payment,
                        "wallet": store.wallet,
                        "cashu": cashu_wallet_overview_json(&overview),
                    }))?
                );
            } else {
                println!(
                    "withdrawn: {} fee={}",
                    paid_exit_sat_text(payment.amount_sat),
                    paid_exit_sat_text(payment.fee_paid_sat)
                );
                println!("mint: {}", payment.mint_url);
                println!("quote_id: {}", payment.quote_id);
                println!("preimage: {}", payment.preimage);
            }
            return Ok(());
        }
        PaidExitWalletCommand::AddMint(add) => {
            let url = normalize_mint_url(&add.url)?;
            let label = add.label.unwrap_or_default();
            changed |= store.upsert_wallet_mint(&url, label, add.balance_msat, unix_timestamp());
            if add.make_default {
                changed |= store.set_default_mint(&url);
            }
        }
        PaidExitWalletCommand::RemoveMint(mint) => {
            changed |= store.remove_wallet_mint(normalize_mint_url(&mint.url)?);
        }
        PaidExitWalletCommand::SetDefault(mint) => {
            changed |= store.set_default_mint(normalize_mint_url(&mint.url)?);
        }
    }

    if changed {
        write_paid_route_store(&store_path, &store)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "changed": changed,
                "wallet": store.wallet,
            }))?
        );
    } else {
        println!("store: {} changed={changed}", store_path.display());
        print_paid_exit_wallet(&store);
    }

    Ok(())
}

fn paid_exit_wallet_data_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn paid_exit_wallet_mint(store: &PaidRouteStore, explicit: Option<&str>) -> Result<String> {
    if let Some(mint) = explicit {
        return normalize_mint_url(mint);
    }
    if !store.wallet.default_mint.trim().is_empty() {
        return normalize_mint_url(&store.wallet.default_mint);
    }
    normalize_mint_url(DEFAULT_PAID_EXIT_WALLET_MINT)
}

fn ensure_paid_exit_wallet_mint(
    store: &mut PaidRouteStore,
    mint_url: &str,
    balance_msat: Option<u64>,
) -> Result<bool> {
    let mint_url = normalize_mint_url(mint_url)?;
    let label = store
        .wallet
        .mints
        .iter()
        .find(|mint| mint.url == mint_url)
        .map(|mint| mint.label.clone())
        .unwrap_or_else(|| {
            if mint_url == DEFAULT_PAID_EXIT_WALLET_MINT {
                "Minibits".to_string()
            } else {
                String::new()
            }
        });
    Ok(store.upsert_wallet_mint(&mint_url, label, balance_msat, unix_timestamp()))
}

fn sync_paid_exit_wallet_store_from_cashu(
    store: &mut PaidRouteStore,
    overview: &CashuWalletOverview,
    now_unix: u64,
) -> bool {
    let mut changed = false;
    for entry in &overview.entries {
        if entry.unit != "sat" {
            continue;
        }
        let label = store
            .wallet
            .mints
            .iter()
            .find(|mint| mint.url == entry.mint_url)
            .map(|mint| mint.label.clone())
            .unwrap_or_else(|| {
                if entry.mint_url == DEFAULT_PAID_EXIT_WALLET_MINT {
                    "Minibits".to_string()
                } else {
                    String::new()
                }
            });
        changed |= store.upsert_wallet_mint(
            &entry.mint_url,
            label,
            Some(entry.balance.saturating_mul(1000)),
            now_unix,
        );
    }
    changed
}

fn read_paid_exit_wallet_token(token: Option<String>, token_stdin: bool) -> Result<String> {
    if token_stdin {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("failed to read Cashu token from stdin")?;
        let token = input.trim().to_string();
        if token.is_empty() {
            return Err(anyhow!("Cashu token from stdin is empty"));
        }
        return Ok(token);
    }

    let Some(token) = token else {
        return Err(anyhow!(
            "missing Cashu token; pass a token or --token-stdin"
        ));
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("Cashu token is empty"));
    }
    Ok(token)
}

fn read_paid_exit_payment_envelope(
    envelope: Option<String>,
    envelope_stdin: bool,
) -> Result<String> {
    if envelope_stdin {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("failed to read paid route payment envelope from stdin")?;
        let envelope = input.trim().to_string();
        if envelope.is_empty() {
            return Err(anyhow!("paid route payment envelope from stdin is empty"));
        }
        return Ok(envelope);
    }

    let Some(envelope) = envelope else {
        return Err(anyhow!(
            "missing paid route payment envelope; pass --envelope or --envelope-stdin"
        ));
    };
    let envelope = envelope.trim().to_string();
    if envelope.is_empty() {
        return Err(anyhow!("paid route payment envelope is empty"));
    }
    Ok(envelope)
}

fn read_paid_exit_offer_event(
    event: Option<String>,
    event_stdin: bool,
    event_file: Option<PathBuf>,
) -> Result<String> {
    if event_stdin {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("failed to read paid route offer event from stdin")?;
        let event = input.trim().to_string();
        if event.is_empty() {
            return Err(anyhow!("paid route offer event from stdin is empty"));
        }
        return Ok(event);
    }

    if let Some(path) = event_file {
        let event = fs::read_to_string(&path)
            .with_context(|| format!("failed to read paid route offer event {}", path.display()))?
            .trim()
            .to_string();
        if event.is_empty() {
            return Err(anyhow!("paid route offer event file is empty"));
        }
        return Ok(event);
    }

    let Some(event) = event else {
        return Err(anyhow!(
            "missing paid route offer event; pass --event, --event-stdin, or --event-file"
        ));
    };
    let event = event.trim().to_string();
    if event.is_empty() {
        return Err(anyhow!("paid route offer event is empty"));
    }
    Ok(event)
}

fn read_paid_exit_spilman_payment(payment: Option<String>, payment_stdin: bool) -> Result<String> {
    if payment_stdin {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("failed to read Cashu Spilman payment from stdin")?;
        let payment = input.trim().to_string();
        if payment.is_empty() {
            return Err(anyhow!("Cashu Spilman payment from stdin is empty"));
        }
        return Ok(payment);
    }

    let Some(payment) = payment else {
        return Err(anyhow!(
            "missing Cashu Spilman payment; pass --payment or --payment-stdin"
        ));
    };
    let payment = payment.trim().to_string();
    if payment.is_empty() {
        return Err(anyhow!("Cashu Spilman payment is empty"));
    }
    Ok(payment)
}

fn read_optional_paid_exit_keyset_info(
    keyset_info: Option<String>,
    keyset_info_file: Option<PathBuf>,
) -> Result<Option<String>> {
    if let Some(path) = keyset_info_file {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read keyset info file {}", path.display()))?;
        let content = content.trim().to_string();
        if content.is_empty() {
            return Err(anyhow!("keyset info file {} is empty", path.display()));
        }
        return Ok(Some(content));
    }
    match keyset_info {
        Some(value) if value.trim().is_empty() => Err(anyhow!("keyset info JSON is empty")),
        Some(value) => Ok(Some(value.trim().to_string())),
        None => Ok(None),
    }
}

fn cashu_wallet_overview_json(overview: &CashuWalletOverview) -> serde_json::Value {
    json!({
        "totals": overview.totals.iter().map(|total| json!({
            "unit": total.unit,
            "balance": total.balance,
        })).collect::<Vec<_>>(),
        "entries": overview.entries.iter().map(|entry| json!({
            "mint_url": entry.mint_url,
            "unit": entry.unit,
            "balance": entry.balance,
        })).collect::<Vec<_>>(),
        "warnings": overview.warnings,
        "legacy_state_detected": overview.legacy_state_detected,
    })
}

fn print_cashu_wallet_overview(overview: &CashuWalletOverview) {
    if overview.totals.is_empty() && overview.entries.is_empty() {
        println!("cashu_wallet: empty");
    } else {
        println!("cashu_totals: {}", overview.totals.len());
        for total in &overview.totals {
            println!("  {} {}", total.balance, total.unit);
        }
        println!("cashu_mints: {}", overview.entries.len());
        for entry in &overview.entries {
            println!("  {} {} {}", entry.mint_url, entry.balance, entry.unit);
        }
    }
    for warning in &overview.warnings {
        println!("warning: {warning}");
    }
    if overview.legacy_state_detected {
        println!("legacy_cashu_wallet_state: detected");
    }
}

fn print_paid_exit_wallet(store: &PaidRouteStore) {
    println!(
        "default_mint: {}",
        display_or_none(&store.wallet.default_mint)
    );
    if store.wallet.mints.is_empty() {
        println!("mints: none");
        return;
    }

    println!("mints: {}", store.wallet.mints.len());
    for mint in &store.wallet.mints {
        let balance = mint
            .balance_msat
            .map(paid_exit_msat_text)
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "  {} label={} balance={} last_checked={}",
            mint.url,
            display_or_none(&mint.label),
            balance,
            mint.last_checked_unix
        );
    }
}

fn ensure_paid_exit_advertisable(app: &AppConfig) -> Result<()> {
    if app.paid_exit.access.upstream == PaidExitUpstream::WireGuardExit {
        if !app.wireguard_exit.configured() {
            return Err(anyhow!(
                "paid exit is configured to resell a WireGuard upstream, but wireguard_exit is incomplete"
            ));
        }
        if !app.wireguard_exit.enabled {
            return Err(anyhow!(
                "paid exit is configured to resell a WireGuard upstream, but wireguard_exit is disabled"
            ));
        }
    }
    Ok(())
}

fn default_paid_exit_offer_id() -> String {
    "internet-exit".to_string()
}

fn local_paid_exit_quality_hint() -> PaidRouteQualityMetrics {
    PaidRouteQualityMetrics {
        last_seen_unix: Some(unix_timestamp()),
        ..PaidRouteQualityMetrics::default()
    }
}

fn paid_exit_relay_urls(app: &AppConfig, overrides: &[String]) -> Vec<String> {
    let relays = if overrides.is_empty() {
        app.nostr.relays.clone()
    } else {
        overrides.to_vec()
    };
    let disabled = normalize_relay_urls(app.nostr.disabled_relays.clone())
        .into_iter()
        .collect::<HashSet<_>>();
    normalize_relay_urls(relays)
        .into_iter()
        .filter(|relay| !disabled.contains(relay))
        .collect()
}

fn persist_paid_exit_offer_snapshot(
    store_path: &Path,
    signed: &SignedPaidRouteOffer,
    relays: &[String],
    offer: &PaidRouteOffer,
    seen_at_unix: u64,
) -> Result<bool> {
    let mut store = load_paid_route_store(store_path)?;
    let mut changed = store.upsert_signed_offer(signed.clone(), relays.to_vec(), seen_at_unix)?;
    for mint in &offer.channel.accepted_mints {
        changed |= store.upsert_wallet_mint(mint, "", None, 0);
    }
    if changed {
        write_paid_route_store(store_path, &store)?;
    }
    Ok(changed)
}

fn persist_paid_exit_discovered_offers(
    store_path: &Path,
    offers: &[SignedPaidRouteOffer],
    relays: &[String],
) -> Result<usize> {
    let mut store = load_paid_route_store(store_path)?;
    let mut changed_count = 0usize;
    let seen_at_unix = unix_timestamp();
    for signed in offers {
        if store.upsert_signed_offer(signed.clone(), relays.to_vec(), seen_at_unix)? {
            changed_count += 1;
        }
    }
    if changed_count > 0 {
        write_paid_route_store(store_path, &store)?;
    }
    Ok(changed_count)
}

async fn publish_paid_exit_offer_to_relays(
    app: &AppConfig,
    signed: &SignedPaidRouteOffer,
    relays: &[String],
) -> Result<serde_json::Value> {
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit publishing"
        ));
    }

    let client = Client::new(app.nostr_keys()?);
    for relay in relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    let output = client
        .send_event_to(relays.to_vec(), &signed.event)
        .await
        .map_err(|error| anyhow!("failed to publish paid exit offer: {error}"))?;
    client.disconnect().await;

    let failed = output
        .failed
        .iter()
        .map(|(relay, error)| {
            json!({
                "relay": relay.to_string(),
                "error": error,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "event_id": output.val.to_string(),
        "success_count": output.success.len(),
        "failed_count": output.failed.len(),
        "success_relays": output.success.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "failed_relays": failed,
    }))
}

async fn publish_paid_exit_payment_to_relays(
    app: &AppConfig,
    event: &Event,
    relays: &[String],
) -> Result<serde_json::Value> {
    publish_paid_exit_payment_event_to_relays(&app.nostr_keys()?, event, relays).await
}

pub(crate) async fn publish_paid_exit_payment_event_to_relays(
    keys: &Keys,
    event: &Event,
    relays: &[String],
) -> Result<serde_json::Value> {
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment publishing"
        ));
    }

    let client = Client::new(keys.clone());
    for relay in relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    let output = client
        .send_event_to(relays.to_vec(), event)
        .await
        .map_err(|error| anyhow!("failed to publish paid exit payment: {error}"))?;
    client.disconnect().await;

    let failed = output
        .failed
        .iter()
        .map(|(relay, error)| {
            json!({
                "relay": relay.to_string(),
                "error": error,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "event_id": output.val.to_string(),
        "success_count": output.success.len(),
        "failed_count": output.failed.len(),
        "success_relays": output.success.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "failed_relays": failed,
    }))
}

async fn discover_paid_exit_offers_from_relays(
    app: &AppConfig,
    relays: &[String],
    duration_secs: u64,
    limit: usize,
    since_unix: Option<u64>,
) -> Result<Vec<SignedPaidRouteOffer>> {
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit discovery"
        ));
    }

    let client = Client::new(app.nostr_keys()?);
    for relay in relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    let mut notifications = client.notifications();
    client
        .subscribe_to(
            relays.to_vec(),
            paid_route_offer_filter(limit, since_unix),
            None,
        )
        .await
        .map_err(|error| anyhow!("failed to subscribe paid exit offers: {error}"))?;

    let timeout = tokio::time::sleep(Duration::from_secs(duration_secs));
    tokio::pin!(timeout);
    let mut seen_events = HashSet::new();
    let mut offers = Vec::new();
    loop {
        tokio::select! {
            () = &mut timeout => break,
            notification = notifications.recv() => {
                match notification {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        let event = (*event).clone();
                        if !seen_events.insert(event.id.to_string()) {
                            continue;
                        }
                        if let Ok(signed) = SignedPaidRouteOffer::from_event(event) {
                            offers.push(signed);
                            if limit > 0 && offers.len() >= limit {
                                break;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    client.disconnect().await;
    offers.sort_by_key(|signed| std::cmp::Reverse(signed.event.created_at.as_secs()));
    Ok(offers)
}

fn paid_exit_offer_results_json(offers: &[SignedPaidRouteOffer]) -> Result<Vec<serde_json::Value>> {
    offers
        .iter()
        .map(|signed| {
            let offer: PaidRouteOffer = signed.offer()?;
            Ok(json!({
                "event_id": signed.event.id.to_string(),
                "created_at": signed.event.created_at.as_secs(),
                "offer": offer,
            }))
        })
        .collect()
}
