
struct PaidExitRatingEventResult {
    config_path: PathBuf,
    store_path: PathBuf,
    session_id: String,
    seller_npub: String,
    rater_npub: String,
    scope: String,
    rating: i64,
    score: i64,
    created_at: u64,
    event: Event,
}

async fn paid_exit_ratings_command(args: PaidExitRatingsArgs) -> Result<()> {
    match args.command {
        PaidExitRatingsCommand::Export(args) => paid_exit_ratings_export_command(args),
        PaidExitRatingsCommand::Publish(args) => paid_exit_ratings_publish_command(args).await,
    }
}

fn paid_exit_ratings_export_command(args: PaidExitRatingsExportArgs) -> Result<()> {
    let json_output = args.json || args.output.is_none();
    let result = paid_exit_rating_event_once(
        args.config,
        args.session,
        args.rating_scope,
        unix_timestamp(),
    )?;
    let export = paid_exit_rating_event_export_json(&result);

    if let Some(path) = args.output {
        fs::write(&path, serde_json::to_string_pretty(&export)?)
            .with_context(|| format!("failed to write paid exit rating event {}", path.display()))?;
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&export)?);
    } else {
        print_paid_exit_rating_event_result(&result, None);
    }

    Ok(())
}

async fn paid_exit_ratings_publish_command(args: PaidExitRatingsPublishArgs) -> Result<()> {
    let json_output = args.json;
    let result = paid_exit_rating_event_once(
        args.config.clone(),
        args.session,
        args.rating_scope,
        unix_timestamp(),
    )?;
    let app = load_or_default_config(&result.config_path)?;
    let publish = publish_paid_exit_rating_event_pubsub(
        &app,
        &result.config_path,
        &result.event,
    )?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "rating": paid_exit_rating_event_result_json(&result),
                "events": [result.event],
                "publish": publish,
            }))?
        );
    } else {
        print_paid_exit_rating_event_result(&result, Some(&publish));
    }

    Ok(())
}

fn paid_exit_rating_event_once(
    config: Option<PathBuf>,
    session_id: String,
    rating_scope: String,
    now_unix: u64,
) -> Result<PaidExitRatingEventResult> {
    let config_path = config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    let keys = app.nostr_keys()?;
    let rater_npub = keys
        .public_key()
        .to_bech32()
        .context("failed to encode paid exit rating rater npub")?;
    let store_path = paid_route_store_file_path(&config_path);
    let store = load_paid_route_store(&store_path)?;
    let session_record = store.sessions.get(&session_id).ok_or_else(|| {
        anyhow!("paid route session {session_id} does not exist")
    })?;
    let seller_npub = store.buyer_session_seller_npub(&session_id)?;
    let (rating, created_at) = paid_exit_rating_from_session_probe(session_record, now_unix)?;
    let score = paid_exit_normalized_rating_score(rating, 0, 100)?;
    let scope = paid_exit_normalized_rating_scope(&rating_scope);
    let event = build_paid_exit_rating_fact_event(
        &keys,
        &rater_npub,
        &seller_npub,
        &scope,
        &session_id,
        rating,
        created_at,
    )?;

    Ok(PaidExitRatingEventResult {
        config_path,
        store_path,
        session_id,
        seller_npub,
        rater_npub,
        scope,
        rating,
        score,
        created_at,
        event,
    })
}

fn paid_exit_rating_from_session_probe(
    record: &PaidRouteSessionRecord,
    now_unix: u64,
) -> Result<(i64, u64)> {
    let session = &record.session;
    let quality = session.quality.as_ref();
    if session.realized_exit_ip.is_none() && quality.is_none_or(|quality| quality.is_empty()) {
        return Err(anyhow!(
            "paid route session {} has no stored probe result; run `nvpn paid-exit probe` first",
            session.session_id
        ));
    }

    let mut rating = 50_i64;
    if session.realized_exit_ip.is_some() {
        rating += 20;
    } else {
        rating -= 30;
    }

    if let Some(quality) = quality {
        if let Some(loss) = quality.packet_loss_ppm {
            rating += match loss {
                0 => 10,
                1..=10_000 => 5,
                10_001..=50_000 => 0,
                50_001..=200_000 => -20,
                _ => -40,
            };
        }
        if let Some(latency) = quality.latency_ms {
            rating += match latency {
                0..=100 => 10,
                101..=300 => 5,
                301..=500 => 0,
                501..=1_000 => -10,
                _ => -20,
            };
        }
        if let Some(jitter) = quality.jitter_ms {
            rating += match jitter {
                0..=30 => 5,
                31..=100 => 0,
                101..=200 => -5,
                _ => -10,
            };
        }
        let best_bps = quality.down_bps.into_iter().chain(quality.up_bps).max();
        if let Some(best_bps) = best_bps {
            rating += match best_bps {
                0..=249_999 => -10,
                250_000..=999_999 => 0,
                1_000_000..=9_999_999 => 5,
                _ => 10,
            };
        }
    }

    let created_at = quality
        .and_then(|quality| quality.last_seen_unix)
        .or_else(|| (record.updated_at_unix > 0).then_some(record.updated_at_unix))
        .unwrap_or(now_unix);
    Ok((rating.clamp(0, 100), created_at))
}

fn build_paid_exit_rating_fact_event(
    keys: &Keys,
    rater_npub: &str,
    seller_npub: &str,
    scope: &str,
    session_id: &str,
    rating: i64,
    created_at: u64,
) -> Result<Event> {
    let record_id =
        paid_exit_rating_record_id(rater_npub, seller_npub, scope, session_id, created_at);
    let tags = vec![
        paid_exit_rating_fact_tag(["i", &record_id, "subject"])?,
        paid_exit_rating_fact_tag(["i", &rater_npub.to_lowercase()])?,
        paid_exit_rating_fact_tag(["i", &seller_npub.to_lowercase()])?,
        paid_exit_rating_fact_tag(["i", &scope.to_lowercase()])?,
        paid_exit_rating_fact_tag(["type", RATING_FACT_TYPE])?,
        paid_exit_rating_fact_tag(["schema", RATING_FACT_SCHEMA])?,
        paid_exit_rating_fact_tag(["created_at", &created_at.to_string()])?,
        paid_exit_rating_fact_tag(["rater", rater_npub])?,
        paid_exit_rating_fact_tag(["subject", seller_npub])?,
        paid_exit_rating_fact_tag(["scope", scope])?,
        paid_exit_rating_fact_tag(["rating", &rating.to_string()])?,
        paid_exit_rating_fact_tag(["min_rating", "0"])?,
        paid_exit_rating_fact_tag(["max_rating", "100"])?,
        paid_exit_rating_fact_tag(["sample_count", "1"])?,
        paid_exit_rating_fact_tag(["reason", "paid_exit_probe"])?,
        paid_exit_rating_fact_tag(["tag", "paid-exit"])?,
        paid_exit_rating_fact_tag(["tag", "fips"])?,
        paid_exit_rating_fact_tag(["tag", "peer"])?,
    ];
    EventBuilder::new(Kind::Custom(RATING_FACT_KIND as u16), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .context("failed to sign paid exit rating fact event")
}

fn paid_exit_rating_record_id(
    rater_npub: &str,
    seller_npub: &str,
    scope: &str,
    session_id: &str,
    created_at: u64,
) -> String {
    use sha2::Digest;

    let mut hasher = sha2::Sha256::new();
    hasher.update(rater_npub.as_bytes());
    hasher.update([0]);
    hasher.update(seller_npub.as_bytes());
    hasher.update([0]);
    hasher.update(scope.as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    hasher.update([0]);
    hasher.update(created_at.to_be_bytes());
    let digest = hasher.finalize();
    format!("paid-exit-rating:{}", hex::encode(&digest[..16]))
}

fn paid_exit_normalized_rating_scope(scope: &str) -> String {
    let scope = scope.trim();
    if scope.is_empty() {
        DEFAULT_FIPS_PEER_RATING_SCOPE.to_string()
    } else {
        scope.to_string()
    }
}

fn paid_exit_rating_fact_tag<const N: usize>(parts: [&str; N]) -> Result<Tag> {
    Tag::parse(parts).map_err(|error| anyhow!("failed to build paid exit rating fact tag: {error}"))
}

fn paid_exit_rating_event_export_json(result: &PaidExitRatingEventResult) -> serde_json::Value {
    json!({
        "rating": paid_exit_rating_event_result_json(result),
        "events": [result.event],
    })
}

fn paid_exit_rating_event_result_json(result: &PaidExitRatingEventResult) -> serde_json::Value {
    json!({
        "config_path": result.config_path.display().to_string(),
        "store_path": result.store_path.display().to_string(),
        "session_id": result.session_id,
        "rater": result.rater_npub,
        "subject": result.seller_npub,
        "scope": result.scope,
        "rating": result.rating,
        "min_rating": 0,
        "max_rating": 100,
        "score": result.score,
        "created_at": result.created_at,
        "event_id": result.event.id.to_string(),
    })
}

fn print_paid_exit_rating_event_result(
    result: &PaidExitRatingEventResult,
    publish: Option<&serde_json::Value>,
) {
    println!("paid_exit_rating_event: {}", result.event.id);
    println!("session: {}", result.session_id);
    println!("rater: {}", result.rater_npub);
    println!("subject: {}", result.seller_npub);
    println!("scope: {}", result.scope);
    println!("rating: {} / 100", result.rating);
    println!("score: {}", result.score);
    println!("created_at: {}", result.created_at);
    if let Some(publish) = publish {
        println!(
            "published: nostr-pubsub queued={}",
            publish["nostr_pubsub_queued"].as_bool().unwrap_or_default()
        );
    }
}
