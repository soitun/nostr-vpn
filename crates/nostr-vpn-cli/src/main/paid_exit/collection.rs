
#[derive(Debug)]
struct PaidExitReceivePaymentsResult {
    seller_npub: String,
    relays: Vec<String>,
    received_count: usize,
    applied: Vec<serde_json::Value>,
    errors: Vec<serde_json::Value>,
    changed: bool,
    spilman_receiver_processing: bool,
    spilman_receiver_error: Option<String>,
    status: serde_json::Value,
}

impl PaidExitReceivePaymentsResult {
    fn applied_count(&self) -> usize {
        self.applied.len()
    }

    fn error_count(&self) -> usize {
        self.errors.len()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PaidExitDaemonReceivePaymentsResult {
    pub(crate) received_count: usize,
    pub(crate) applied_count: usize,
    pub(crate) error_count: usize,
    pub(crate) changed: bool,
    pub(crate) spilman_receiver_processing: bool,
}

async fn paid_exit_receive_payments(
    app: &AppConfig,
    config_path: &Path,
    relays: Vec<String>,
    duration_secs: u64,
    limit: usize,
    since_secs: u64,
) -> Result<PaidExitReceivePaymentsResult> {
    if !app.paid_exit.enabled {
        return Err(anyhow!("paid exit selling is disabled"));
    }
    let keys = app.nostr_keys()?;
    let seller_npub = keys
        .public_key()
        .to_bech32()
        .context("failed to encode seller npub")?;
    let since_unix = if since_secs == 0 {
        None
    } else {
        Some(unix_timestamp().saturating_sub(since_secs))
    };
    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let mut seen_events = HashSet::new();
    let mut applied = Vec::new();
    let mut errors = Vec::new();
    let (spilman_receiver, spilman_receiver_error) =
        try_load_paid_exit_spilman_receiver(config_path, &app.paid_exit).await;
    let spilman_receiver_processing = spilman_receiver.is_some();

    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit payment receiving"
        ));
    }

    let pubsub_sources = paid_exit_pubsub_relay_sources(&relays);
    let relays = paid_exit_pubsub_relay_urls(&pubsub_sources);
    let retention_policy = paid_exit_payment_retention_policy(keys.public_key(), limit, since_unix);
    let retention_filter = paid_exit_retention_filter(&retention_policy, "payment")?;
    let effective_limit = retention_policy.max_events;
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
        .subscribe_to(relays.clone(), retention_filter, None)
        .await
        .map_err(|error| anyhow!("failed to subscribe paid exit payments: {error}"))?;

    let timeout = tokio::time::sleep(Duration::from_secs(duration_secs));
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
                        let Ok(verified_event) = nostr_pubsub::VerifiedEvent::try_from(event.clone()) else {
                            continue;
                        };
                        if !retention_policy.accepts(&verified_event) {
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
                                    spilman_receiver_error.as_deref(),
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
                        if seen_events.len() >= effective_limit {
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

    Ok(PaidExitReceivePaymentsResult {
        seller_npub,
        relays,
        received_count: seen_events.len(),
        applied,
        errors,
        changed,
        spilman_receiver_processing,
        spilman_receiver_error,
        status: paid_exit_status_snapshot_json(app, &store_path, &store),
    })
}

pub(crate) async fn paid_exit_receive_payments_for_daemon(
    app: &AppConfig,
    config_path: &Path,
    duration_secs: u64,
    limit: usize,
) -> Result<PaidExitDaemonReceivePaymentsResult> {
    if !app.paid_exit.enabled {
        return Ok(PaidExitDaemonReceivePaymentsResult::default());
    }
    let result = paid_exit_receive_payments(
        app,
        config_path,
        paid_exit_relay_urls(app, &[]),
        duration_secs,
        limit,
        0,
    )
    .await?;
    Ok(PaidExitDaemonReceivePaymentsResult {
        received_count: result.received_count,
        applied_count: result.applied_count(),
        error_count: result.error_count(),
        changed: result.changed,
        spilman_receiver_processing: result.spilman_receiver_processing,
    })
}

async fn paid_exit_receive_payments_command(args: PaidExitReceivePaymentsArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let store_path = paid_route_store_file_path(&config_path);
    let result = paid_exit_receive_payments(
        &app,
        &config_path,
        paid_exit_relay_urls(&app, &args.relays),
        args.duration_secs,
        args.limit,
        args.since_secs,
    )
    .await?;
    let daemon_reload_attempted = result.changed && !args.no_reload_daemon;
    if daemon_reload_attempted {
        maybe_reload_running_daemon(&config_path);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "store_path": store_path.display().to_string(),
                "seller": result.seller_npub,
                "relays": result.relays,
                "received_count": result.received_count,
                "applied_count": result.applied_count(),
                "error_count": result.error_count(),
                "changed": result.changed,
                "spilman_receiver_processing": result.spilman_receiver_processing,
                "spilman_receiver_mode": paid_exit_spilman_receiver_mode(result.spilman_receiver_processing),
                "spilman_receiver_validation": result.spilman_receiver_processing,
                "spilman_receiver_error": result.spilman_receiver_error,
                "daemon_reload_attempted": daemon_reload_attempted,
                "applied": result.applied,
                "errors": result.errors,
                "status": result.status,
            }))?
        );
    } else {
        println!("paid_exit_payments_received: {}", result.received_count);
        println!("seller: {}", result.seller_npub);
        println!("store: {} changed={}", store_path.display(), result.changed);
        println!("applied: {}", result.applied_count());
        println!("errors: {}", result.error_count());
        println!(
            "spilman_receiver_processing: {}",
            paid_exit_spilman_receiver_mode(result.spilman_receiver_processing)
        );
        if let Some(error) = result.spilman_receiver_error {
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
