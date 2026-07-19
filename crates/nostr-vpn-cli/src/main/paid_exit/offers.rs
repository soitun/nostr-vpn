
async fn paid_exit_offer_command(args: PaidExitOfferArgs) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;
    ensure_paid_exit_advertisable(&app)?;
    let keys = app.nostr_keys()?;
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
        persist_paid_exit_offer_snapshot(&store_path, &signed, &[], &offer, unix_timestamp())?;

    let publish = if args.publish {
        Some(publish_paid_exit_offer_pubsub(&app, &config_path, &signed)?)
    } else {
        None
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "offer": offer,
                "event": signed.event,
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
        println!("store: {} changed={stored}", store_path.display());
        if let Some(publish) = publish {
            println!(
                "published: nostr-pubsub queued={}",
                publish["nostr_pubsub_queued"].as_bool().unwrap_or_default()
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
    let changed =
        upsert_paid_route_offer(&store_path, signed.clone(), vec![], unix_timestamp())?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "offer": offer,
                "event": signed.event,
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
    let trusted_rating_authors =
        paid_exit_trusted_rating_author_set(&args.trusted_rating_authors)?;
    let mut rating_scores = args
        .fips_peer_ratings
        .as_deref()
        .map(|path| load_paid_exit_rating_scores(path, &args.rating_scope, &trusted_rating_authors))
        .transpose()?;
    let since_unix = if args.since_secs == 0 {
        None
    } else {
        Some(unix_timestamp().saturating_sub(args.since_secs))
    };
    let cached_control_events =
        crate::control_pubsub_runtime::load_control_pubsub_events(&config_path)?;
    let cached_rating_events = cached_control_events
        .iter()
        .filter(|event| event.kind == Kind::Custom(RATING_FACT_KIND as u16))
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let cached_rating_event_count = cached_rating_events.len();
    if !cached_rating_events.is_empty() {
        let cached_scores = paid_exit_rating_scores_from_value(
            &json!({ "events": cached_rating_events }),
            &args.rating_scope,
            &trusted_rating_authors,
        )?;
        merge_paid_exit_rating_scores(&mut rating_scores, cached_scores);
    }
    let retention_policy = paid_exit_offer_retention_policy(args.limit, since_unix);
    let cached_offers = cached_control_events
        .into_iter()
        .filter_map(|event| {
            let verified = nostr_pubsub::VerifiedEvent::try_from(event.clone()).ok()?;
            retention_policy
                .accepts(&verified)
                .then(|| SignedPaidRouteOffer::from_event(event).ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    let cached_offer_count = cached_offers.len();
    let mut offers = cached_offers.clone();
    offers.sort_by_key(|signed| std::cmp::Reverse(signed.event.created_at.as_secs()));
    let mut seen_offer_ids = HashSet::new();
    offers.retain(|signed| seen_offer_ids.insert(signed.event.id));
    offers.truncate(retention_policy.max_events);
    if let Some(scores) = rating_scores.as_ref() {
        paid_exit_sort_offers_by_rating(&mut offers, scores);
    }
    let store_path = paid_route_store_file_path(&config_path);
    let stored_count = persist_paid_exit_discovered_offers(
        &store_path,
        &cached_offers,
        &[],
        rating_scores.as_ref(),
    )?;

    if args.json {
        let offers_json = paid_exit_offer_results_json(&offers, rating_scores.as_ref())?;
        let ratings_json = if args.fips_peer_ratings.is_some() || cached_rating_event_count > 0 {
            Some(json!({
                "path": args.fips_peer_ratings.as_ref().map(|path| path.display().to_string()),
                "scope": args.rating_scope,
                "subject_count": rating_scores.as_ref().map_or(0, HashMap::len),
                "nostr_pubsub_cached_event_count": cached_rating_event_count,
                "trusted_author_count": trusted_rating_authors.len(),
            }))
        } else {
            None
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "count": offers_json.len(),
                "offers": offers_json,
                "store_path": store_path,
                "stored_count": stored_count,
                "nostr_pubsub_cached_offer_count": cached_offer_count,
                "nostr_pubsub_cached_rating_event_count": cached_rating_event_count,
                "ratings": ratings_json,
            }))?
        );
    } else {
        println!("paid_exit_offers: {}", offers.len());
        println!(
            "nostr_pubsub_cache: offers={} rating_events={}",
            cached_offer_count, cached_rating_event_count
        );
        println!("store: {} changed={stored_count}", store_path.display());
        if args.fips_peer_ratings.is_some() || cached_rating_event_count > 0 {
            let subject_count = rating_scores.as_ref().map_or(0, HashMap::len);
            let file = args
                .fips_peer_ratings
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "ratings: file={} scope={} subjects={} nostr_pubsub_events={} trusted_authors={}",
                file,
                args.rating_scope,
                subject_count,
                cached_rating_event_count,
                trusted_rating_authors.len()
            );
        }
        for signed in &offers {
            let offer = signed.offer()?;
            println!(
                "{}",
                paid_exit_offer_summary_line_with_rating(
                    &offer,
                    signed.event.id,
                    rating_scores.as_ref()
                )
            );
        }
    }

    Ok(())
}
