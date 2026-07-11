
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaidExitRatingScore {
    score: i64,
    created_at: u64,
}

fn load_paid_exit_rating_scores(
    path: &Path,
    scope: &str,
    trusted_authors: &HashSet<String>,
) -> Result<HashMap<String, PaidExitRatingScore>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read paid exit ratings {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse paid exit ratings {}", path.display()))?;
    paid_exit_rating_scores_from_value(&value, scope, trusted_authors)
}

fn paid_exit_rating_scores_from_value(
    value: &serde_json::Value,
    scope: &str,
    trusted_authors: &HashSet<String>,
) -> Result<HashMap<String, PaidExitRatingScore>> {
    let mut scores: HashMap<String, PaidExitRatingScore> = HashMap::new();
    for rating in paid_exit_rating_records(value, trusted_authors)? {
        if !paid_exit_rating_matches_scope(&rating, scope) {
            continue;
        }
        let subject = paid_exit_rating_string_field(&rating, "subject")?;
        let rating_value = paid_exit_rating_i64_field(&rating, "rating")?;
        let min_rating = paid_exit_rating_i64_field(&rating, "min_rating")?;
        let max_rating = paid_exit_rating_i64_field(&rating, "max_rating")?;
        let score = paid_exit_normalized_rating_score(rating_value, min_rating, max_rating)?;
        let created_at = paid_exit_rating_u64_field(&rating, "created_at").unwrap_or_default();
        let incoming = PaidExitRatingScore { score, created_at };
        scores
            .entry(subject)
            .and_modify(|existing| {
                if incoming.created_at >= existing.created_at {
                    *existing = incoming;
                }
            })
            .or_insert(incoming);
    }
    Ok(scores)
}

fn merge_paid_exit_rating_scores(
    target: &mut Option<HashMap<String, PaidExitRatingScore>>,
    incoming: HashMap<String, PaidExitRatingScore>,
) {
    if incoming.is_empty() {
        return;
    }
    let target = target.get_or_insert_with(HashMap::new);
    for (subject, incoming_score) in incoming {
        target
            .entry(subject)
            .and_modify(|existing| {
                if incoming_score.created_at >= existing.created_at {
                    *existing = incoming_score;
                }
            })
            .or_insert(incoming_score);
    }
}

fn paid_exit_rating_records(
    value: &serde_json::Value,
    trusted_authors: &HashSet<String>,
) -> Result<Vec<serde_json::Value>> {
    if let Some(records) = value.as_array() {
        return Ok(records
            .iter()
            .filter(|record| paid_exit_rating_record_author_is_trusted(record, trusted_authors))
            .cloned()
            .collect());
    }

    if let Some(records) = value
        .get("ratings")
        .and_then(|ratings| ratings.as_array())
    {
        return Ok(records
            .iter()
            .filter(|record| paid_exit_rating_record_author_is_trusted(record, trusted_authors))
            .cloned()
            .collect());
    }

    if let Some(events) = value.get("events").and_then(|events| events.as_array()) {
        return paid_exit_rating_records_from_events(events, trusted_authors);
    }

    Err(anyhow!(
        "ratings JSON must be an array, an object with a ratings array, or an object with an events array"
    ))
}

fn paid_exit_rating_records_from_events(
    events: &[serde_json::Value],
    trusted_authors: &HashSet<String>,
) -> Result<Vec<serde_json::Value>> {
    let mut records = Vec::new();
    for event_value in events {
        let Ok(event) = paid_exit_verified_rating_fact_event(event_value) else {
            continue;
        };
        if !paid_exit_rating_event_author_is_trusted(&event, trusted_authors) {
            continue;
        }
        if let Ok(record) = paid_exit_rating_record_from_verified_fact_event(&event) {
            records.push(record);
        }
    }
    Ok(records)
}

fn paid_exit_trusted_rating_author_set(authors: &[String]) -> Result<HashSet<String>> {
    let mut normalized = HashSet::new();
    for author in authors.iter().flat_map(|value| value.split(',')) {
        let author = author.trim();
        if author.is_empty() {
            continue;
        }
        normalized.insert(paid_exit_normalize_rating_author(author)?);
    }
    Ok(normalized)
}

fn paid_exit_normalize_rating_author(author: &str) -> Result<String> {
    PublicKey::parse(author.trim())
        .map(|public_key| public_key.to_hex())
        .map_err(|error| anyhow!("invalid trusted rating author {author}: {error}"))
}

fn paid_exit_rating_event_author_is_trusted(
    event: &Event,
    trusted_authors: &HashSet<String>,
) -> bool {
    if trusted_authors.is_empty() {
        return true;
    }
    trusted_authors.contains(&event.pubkey.to_hex())
}

fn paid_exit_rating_record_author_is_trusted(
    record: &serde_json::Value,
    trusted_authors: &HashSet<String>,
) -> bool {
    if trusted_authors.is_empty() {
        return true;
    }
    record
        .get("rater")
        .and_then(|value| value.as_str())
        .and_then(|rater| paid_exit_normalize_rating_author(rater).ok())
        .is_some_and(|author| trusted_authors.contains(&author))
}

fn paid_exit_verified_rating_fact_event(event_value: &serde_json::Value) -> Result<Event> {
    let event: Event = serde_json::from_value(event_value.clone())
        .context("rating fact event is not valid Nostr event JSON")?;
    event
        .verify()
        .map_err(|error| anyhow!("rating fact event verification failed: {error}"))?;
    if event.kind != Kind::Custom(RATING_FACT_KIND as u16) {
        return Err(anyhow!(
            "rating fact event kind must be {RATING_FACT_KIND}, got {:?}",
            event.kind
        ));
    }
    Ok(event)
}

fn paid_exit_rating_record_from_verified_fact_event(event: &Event) -> Result<serde_json::Value> {
    let event_value = serde_json::to_value(event).context("failed to encode rating fact event JSON")?;
    let event_value = &event_value;

    let record_type = paid_exit_fact_scalar(event_value, "type")?;
    if record_type != RATING_FACT_TYPE {
        return Err(anyhow!("unexpected rating fact event type {record_type}"));
    }
    let schema = paid_exit_fact_scalar(event_value, "schema")?;
    if schema != RATING_FACT_SCHEMA {
        return Err(anyhow!("unsupported rating fact schema {schema}"));
    }

    let id = paid_exit_fact_subject_id(event_value)
        .or_else(|| event_value.get("id").and_then(|value| value.as_str()).map(ToOwned::to_owned))
        .ok_or_else(|| anyhow!("rating fact event is missing subject id"))?;
    let created_at = paid_exit_fact_optional_scalar(event_value, "created_at")
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| event_value.get("created_at").and_then(|value| value.as_u64()))
        .unwrap_or_default();
    let mut record = json!({
        "id": id,
        "rater": paid_exit_fact_scalar(event_value, "rater")?,
        "subject": paid_exit_fact_scalar(event_value, "subject")?,
        "rating": paid_exit_fact_scalar(event_value, "rating")?.parse::<i64>()
            .context("rating fact event has invalid integer rating")?,
        "min_rating": paid_exit_fact_scalar(event_value, "min_rating")?.parse::<i64>()
            .context("rating fact event has invalid integer min_rating")?,
        "max_rating": paid_exit_fact_scalar(event_value, "max_rating")?.parse::<i64>()
            .context("rating fact event has invalid integer max_rating")?,
        "created_at": created_at,
    });
    if let Some(scope) = paid_exit_fact_optional_scalar(event_value, "scope") {
        record["scope"] = json!(scope);
    }
    if let Some(sample_count) = paid_exit_fact_optional_scalar(event_value, "sample_count")
        .and_then(|value| value.parse::<u64>().ok())
    {
        record["sample_count"] = json!(sample_count);
    }
    if let Some(window_start) = paid_exit_fact_optional_scalar(event_value, "window_start")
        .and_then(|value| value.parse::<u64>().ok())
    {
        record["window_start"] = json!(window_start);
    }
    if let Some(window_end) = paid_exit_fact_optional_scalar(event_value, "window_end")
        .and_then(|value| value.parse::<u64>().ok())
    {
        record["window_end"] = json!(window_end);
    }
    if let Some(reason) = paid_exit_fact_optional_scalar(event_value, "reason") {
        record["reason"] = json!(reason);
    }
    let tags = paid_exit_fact_values(event_value, "tag");
    if !tags.is_empty() {
        record["tags"] = json!(tags);
    }
    let evidence = paid_exit_fact_values(event_value, "evidence");
    if !evidence.is_empty() {
        record["evidence"] = json!(evidence);
    }
    Ok(record)
}

fn paid_exit_fact_subject_id(event_value: &serde_json::Value) -> Option<String> {
    paid_exit_fact_tags(event_value).into_iter().find_map(|tag| {
        let parts = tag.as_array()?;
        let name = parts.first()?.as_str()?;
        if name == "i" && parts.get(2).and_then(|value| value.as_str()) == Some("subject") {
            parts.get(1)?.as_str().map(ToOwned::to_owned)
        } else {
            None
        }
    })
}

fn paid_exit_fact_scalar(event_value: &serde_json::Value, key: &str) -> Result<String> {
    paid_exit_fact_optional_scalar(event_value, key)
        .ok_or_else(|| anyhow!("rating fact event is missing scalar tag {key}"))
}

fn paid_exit_fact_optional_scalar(event_value: &serde_json::Value, key: &str) -> Option<String> {
    paid_exit_fact_values(event_value, key).into_iter().next()
}

fn paid_exit_fact_values(event_value: &serde_json::Value, key: &str) -> Vec<String> {
    paid_exit_fact_tags(event_value)
        .into_iter()
        .filter_map(|tag| {
            let parts = tag.as_array()?;
            if parts.first().and_then(|value| value.as_str()) != Some(key) {
                return None;
            }
            parts.get(1).and_then(|value| value.as_str()).map(str::trim)
        })
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn paid_exit_fact_tags(event_value: &serde_json::Value) -> Vec<&serde_json::Value> {
    event_value
        .get("tags")
        .and_then(|tags| tags.as_array())
        .map(|tags| tags.iter().collect())
        .unwrap_or_default()
}

fn paid_exit_rating_matches_scope(rating: &serde_json::Value, expected_scope: &str) -> bool {
    let expected_scope = expected_scope.trim();
    expected_scope.is_empty()
        || rating
            .get("scope")
            .and_then(|value| value.as_str())
            .is_some_and(|scope| scope.trim() == expected_scope)
}

fn paid_exit_rating_fact_matches_scope(
    event_value: &serde_json::Value,
    expected_scope: &str,
) -> bool {
    let expected_scope = expected_scope.trim();
    expected_scope.is_empty()
        || paid_exit_fact_optional_scalar(event_value, "scope")
            .is_some_and(|scope| scope.trim() == expected_scope)
}

fn paid_exit_rating_string_field(rating: &serde_json::Value, key: &str) -> Result<String> {
    rating
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("rating record is missing string field {key}"))
}

fn paid_exit_rating_i64_field(rating: &serde_json::Value, key: &str) -> Result<i64> {
    rating
        .get(key)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| anyhow!("rating record is missing integer field {key}"))
}

fn paid_exit_rating_u64_field(rating: &serde_json::Value, key: &str) -> Option<u64> {
    rating.get(key).and_then(|value| value.as_u64())
}

fn paid_exit_normalized_rating_score(
    rating: i64,
    min_rating: i64,
    max_rating: i64,
) -> Result<i64> {
    if min_rating >= max_rating {
        return Err(anyhow!(
            "invalid rating range {min_rating}..{max_rating}"
        ));
    }
    if rating < min_rating || rating > max_rating {
        return Err(anyhow!(
            "rating {rating} outside range {min_rating}..{max_rating}"
        ));
    }
    let rating = i128::from(rating);
    let min = i128::from(min_rating);
    let max = i128::from(max_rating);
    let width = max - min;
    let centered = rating.saturating_mul(2) - min - max;
    Ok(((centered.saturating_mul(100)) / width) as i64)
}

fn paid_exit_sort_offers_by_rating(
    offers: &mut [SignedPaidRouteOffer],
    rating_scores: &HashMap<String, PaidExitRatingScore>,
) {
    offers.sort_by(|left, right| {
        let left_score = paid_exit_signed_offer_rating_score(left, rating_scores)
            .map_or(0, |score| score.score);
        let right_score = paid_exit_signed_offer_rating_score(right, rating_scores)
            .map_or(0, |score| score.score);
        right_score
            .cmp(&left_score)
            .then_with(|| right.event.created_at.as_secs().cmp(&left.event.created_at.as_secs()))
            .then_with(|| left.event.id.to_string().cmp(&right.event.id.to_string()))
    });
}

fn paid_exit_signed_offer_rating_score(
    signed: &SignedPaidRouteOffer,
    rating_scores: &HashMap<String, PaidExitRatingScore>,
) -> Option<PaidExitRatingScore> {
    signed
        .offer()
        .ok()
        .and_then(|offer| rating_scores.get(&offer.seller_npub).copied())
}

fn paid_exit_rating_fact_filter(limit: usize, since_unix: Option<u64>, scope: &str) -> Filter {
    let mut filter = Filter::new().kind(Kind::Custom(RATING_FACT_KIND as u16));
    if limit > 0 {
        filter = filter.limit(limit);
    }
    if let Some(since_unix) = since_unix {
        filter = filter.since(Timestamp::from(since_unix));
    }
    let scope = scope.trim();
    if !scope.is_empty() {
        filter = filter.custom_tag(
            SingleLetterTag::lowercase(Alphabet::I),
            scope.to_lowercase(),
        );
    }
    filter
}

async fn discover_paid_exit_rating_events_from_relays(
    app: &AppConfig,
    relays: &[String],
    duration_secs: u64,
    limit: usize,
    since_unix: Option<u64>,
    scope: &str,
    trusted_authors: &HashSet<String>,
) -> Result<serde_json::Value> {
    if relays.is_empty() {
        return Err(anyhow!(
            "no Nostr relays configured for paid exit rating discovery"
        ));
    }

    let pubsub_sources = paid_exit_pubsub_relay_sources(relays);
    let relays = paid_exit_pubsub_relay_urls(&pubsub_sources);
    let retention_policy = paid_exit_rating_retention_policy(limit, since_unix, scope);
    let retention_filter = paid_exit_retention_filter(&retention_policy, "rating")?;
    let effective_limit = retention_policy.max_events;
    let client = Client::new(app.nostr_keys()?);
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
        .map_err(|error| anyhow!("failed to subscribe paid exit rating facts: {error}"))?;

    let timeout = tokio::time::sleep(Duration::from_secs(duration_secs));
    tokio::pin!(timeout);
    let mut seen_events = HashSet::new();
    let mut events = Vec::new();
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
                        if event.verify().is_err() {
                            continue;
                        }
                        let Ok(verified_event) = nostr_pubsub::VerifiedEvent::try_from(event.clone()) else {
                            continue;
                        };
                        if !retention_policy.accepts(&verified_event) {
                            continue;
                        }
                        if !trusted_authors.is_empty()
                            && !trusted_authors.contains(&event.pubkey.to_hex())
                        {
                            continue;
                        }
                        let value = serde_json::to_value(&event)
                            .context("failed to encode rating fact event JSON")?;
                        if paid_exit_fact_optional_scalar(&value, "type").as_deref()
                            != Some(RATING_FACT_TYPE)
                        {
                            continue;
                        }
                        if !paid_exit_rating_fact_matches_scope(&value, scope) {
                            continue;
                        }
                        events.push(value);
                        if events.len() >= effective_limit {
                            break;
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
    Ok(json!({ "events": events }))
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

    let pubsub_sources = paid_exit_pubsub_relay_sources(relays);
    let relays = paid_exit_pubsub_relay_urls(&pubsub_sources);
    let retention_policy = paid_exit_offer_retention_policy(limit, since_unix);
    let retention_filter = paid_exit_retention_filter(&retention_policy, "offer")?;
    let effective_limit = retention_policy.max_events;
    let client = Client::new(app.nostr_keys()?);
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
                        let Ok(verified_event) = nostr_pubsub::VerifiedEvent::try_from(event.clone()) else {
                            continue;
                        };
                        if !retention_policy.accepts(&verified_event) {
                            continue;
                        }
                        if let Ok(signed) = SignedPaidRouteOffer::from_event(event) {
                            offers.push(signed);
                            if offers.len() >= effective_limit {
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

fn paid_exit_offer_results_json(
    offers: &[SignedPaidRouteOffer],
    rating_scores: Option<&HashMap<String, PaidExitRatingScore>>,
) -> Result<Vec<serde_json::Value>> {
    offers
        .iter()
        .map(|signed| {
            let offer: PaidRouteOffer = signed.offer()?;
            let rating_score = rating_scores
                .and_then(|scores| scores.get(&offer.seller_npub))
                .map(|score| score.score);
            let mut value = json!({
                "event_id": signed.event.id.to_string(),
                "created_at": signed.event.created_at.as_secs(),
                "offer": offer,
            });
            if rating_scores.is_some() {
                value["rating_score"] = rating_score
                    .map(|score| json!(score))
                    .unwrap_or(serde_json::Value::Null);
            }
            Ok(value)
        })
        .collect()
}
