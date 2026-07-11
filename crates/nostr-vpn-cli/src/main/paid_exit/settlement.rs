
#[derive(Debug)]
struct PaidExitSettleResult {
    payment: BuildPaidRouteBuyerPaymentEnvelopeResult,
    wallet_sign: serde_json::Value,
    publish_requested: bool,
    relays: Vec<String>,
    publish: Option<serde_json::Value>,
    persisted: bool,
}

struct PaidExitSettleRequest<'a, S: CashuSpilmanPaymentSigner> {
    app: &'a AppConfig,
    keys: &'a Keys,
    store: &'a mut PaidRouteStore,
    signer: &'a S,
    session_id: &'a str,
    relays: &'a [String],
    publish: bool,
    wallet_data_dir: &'a Path,
    now_unix: u64,
}

async fn paid_exit_settle_with_signer<S: CashuSpilmanPaymentSigner>(
    request: PaidExitSettleRequest<'_, S>,
) -> Result<PaidExitSettleResult> {
    let PaidExitSettleRequest {
        app,
        keys,
        store,
        signer,
        session_id,
        relays,
        publish,
        wallet_data_dir,
        now_unix,
    } = request;
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
        PaidExitSettleRequest {
            app: &app,
            keys: &keys,
            store: &mut store,
            signer: &signer,
            session_id: &args.session,
            relays: &relays,
            publish,
            wallet_data_dir: &wallet_data_dir,
            now_unix: unix_timestamp(),
        },
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
        spilman_receiver_error.as_deref(),
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
