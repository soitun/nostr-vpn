
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

fn paid_exit_pubsub_relay_sources(relays: &[String]) -> Vec<nostr_pubsub::SourceRoute> {
    relays
        .iter()
        .map(|relay| {
            nostr_pubsub::SourceRoute::relay(relay.clone())
                .with_reason("nostr-vpn app relay config")
        })
        .collect()
}

fn paid_exit_pubsub_relay_urls(routes: &[nostr_pubsub::SourceRoute]) -> Vec<String> {
    routes
        .iter()
        .filter(|route| route.source.kind == nostr_pubsub::EventSourceKind::Relay)
        .filter_map(|route| route.source.url.clone())
        .collect()
}

fn paid_exit_retention_event_limit(requested_limit: usize, fallback_limit: usize) -> usize {
    if requested_limit == 0 {
        fallback_limit
    } else {
        requested_limit
    }
}

fn paid_exit_offer_retention_policy(
    limit: usize,
    since_unix: Option<u64>,
) -> nostr_pubsub::EventRetentionPolicy {
    nostr_pubsub::EventRetentionPolicy::new(
        paid_exit_retention_event_limit(limit, PAID_EXIT_OFFER_EVENT_CACHE_LIMIT),
        vec![paid_route_offer_filter(limit, since_unix)],
    )
}

fn paid_exit_rating_retention_policy(
    limit: usize,
    since_unix: Option<u64>,
    scope: &str,
) -> nostr_pubsub::EventRetentionPolicy {
    nostr_pubsub::EventRetentionPolicy::new(
        paid_exit_retention_event_limit(limit, PAID_EXIT_RATING_EVENT_LOOKUP_LIMIT),
        vec![paid_exit_rating_fact_filter(limit, since_unix, scope)],
    )
}

fn paid_exit_retention_filter(
    policy: &nostr_pubsub::EventRetentionPolicy,
    label: &str,
) -> Result<Filter> {
    policy
        .filters
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("paid exit {label} pubsub retention policy has no filters"))
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
    rating_scores: Option<&HashMap<String, PaidExitRatingScore>>,
) -> Result<usize> {
    let mut store = load_paid_route_store(store_path)?;
    let mut changed_count = 0usize;
    let seen_at_unix = unix_timestamp();
    for signed in offers {
        let offer = signed.offer()?;
        let mut changed = store.upsert_signed_offer(signed.clone(), relays.to_vec(), seen_at_unix)?;
        if let Some(score) = rating_scores.and_then(|scores| scores.get(&offer.seller_npub)) {
            changed |= store.upsert_offer_rating_score(
                &offer.seller_npub,
                score.score,
                score.created_at,
            );
        }
        if changed {
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
    let pubsub_sources = paid_exit_pubsub_relay_sources(relays);
    let relays = paid_exit_pubsub_relay_urls(&pubsub_sources);
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit publishing"
        ));
    }

    let client = Client::new(app.nostr_keys()?);
    for relay in &relays {
        client
            .add_relay(relay)
            .await
            .map_err(|error| anyhow!("failed to add Nostr relay {relay}: {error}"))?;
    }
    client.connect().await;
    let output = client
        .send_event_to(relays.clone(), &signed.event)
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

async fn publish_paid_exit_offer_hybrid(
    app: &AppConfig,
    config_path: &Path,
    signed: &SignedPaidRouteOffer,
    relays: &[String],
) -> Result<serde_json::Value> {
    let p2p_enabled = app.nostr.pubsub.enabled();
    let p2p_queued = if p2p_enabled {
        crate::control_pubsub_runtime::queue_control_pubsub_event(config_path, &signed.event)?
    } else {
        false
    };
    let mut output = if relays.is_empty() {
        if !p2p_enabled {
            return Err(anyhow!(
                "no publication path: Nostr relays are empty and nostr.pubsub.mode is off"
            ));
        }
        json!({
            "event_id": signed.event.id.to_string(),
            "success_count": 0,
            "failed_count": 0,
            "success_relays": [],
            "failed_relays": [],
        })
    } else {
        publish_paid_exit_offer_to_relays(app, signed, relays).await?
    };
    output["p2p_enabled"] = json!(p2p_enabled);
    output["p2p_queued"] = json!(p2p_queued);
    output["p2p_outbox"] =
        json!(crate::control_pubsub_runtime::control_pubsub_outbox_directory(config_path));
    Ok(output)
}

async fn publish_paid_exit_rating_event_to_relays(
    keys: &Keys,
    event: &Event,
    relays: &[String],
) -> Result<serde_json::Value> {
    let pubsub_sources = paid_exit_pubsub_relay_sources(relays);
    let relays = paid_exit_pubsub_relay_urls(&pubsub_sources);
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit rating publishing"
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
    let output = client
        .send_event_to(relays.clone(), event)
        .await
        .map_err(|error| anyhow!("failed to publish paid exit rating: {error}"))?;
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
