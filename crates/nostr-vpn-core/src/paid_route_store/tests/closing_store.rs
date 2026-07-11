use super::*;

#[test]
fn seller_payment_rejects_regressing_balance_update_without_mutating_store() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let seller_npub = seller.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
    store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                120,
                StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                    delivered_units: 100,
                    amount_due_msat: 1_000,
                    paid_msat: 1_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub: seller_npub.clone(),
            config: config.clone(),
            now_unix: 120,
        })
        .expect("apply first update");
    let before = store.clone();

    let error = store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                121,
                StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                    delivered_units: 100,
                    amount_due_msat: 1_000,
                    paid_msat: 0,
                    payment: sample_spilman_payment("channel-1", 0),
                }),
            ),
            seller_npub,
            config,
            now_unix: 121,
        })
        .expect_err("regressing update rejected");

    assert!(error.to_string().contains("regressed"));
    assert_eq!(store, before);
}

#[test]
fn seller_payment_rejects_overclaimed_spilman_balance_without_mutating_store() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let seller_npub = seller.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
    let before = store.clone();
    let error = store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                120,
                StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                    delivered_units: 100,
                    amount_due_msat: 1_000,
                    paid_msat: 2_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub,
            config,
            now_unix: 120,
        })
        .expect_err("overclaimed balance update should fail");

    assert!(error.to_string().contains("does not match"));
    assert_eq!(store, before);
}

#[test]
fn seller_payment_rejects_underpaid_cooperative_close_without_mutating_store() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let seller_npub = seller.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
    store
        .record_seller_usage(RecordPaidRouteSellerUsageRequest {
            buyer_pubkey: buyer.public_key().to_hex(),
            config: config.clone(),
            usage_delta: PaidRouteUsage {
                billable_bytes: 200,
                ..PaidRouteUsage::default()
            },
            now_unix: 120,
        })
        .expect("record seller-observed usage")
        .expect("matched seller session");
    let before = store.clone();

    let error = store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                130,
                StreamingRoutePaymentPayload::CooperativeClose(StreamingRouteCooperativeClose {
                    final_paid_msat: 1_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub,
            config,
            now_unix: 130,
        })
        .expect_err("underpaid close should fail");

    assert!(error.to_string().contains("underpays amount due"));
    assert_eq!(store, before);
}

#[test]
fn seller_payment_cooperative_close_suspends_admission() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let seller_npub = seller.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
    store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                120,
                StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                    delivered_units: 100,
                    amount_due_msat: 1_000,
                    paid_msat: 1_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub: seller_npub.clone(),
            config: config.clone(),
            now_unix: 120,
        })
        .expect("apply first update");

    let result = store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                130,
                StreamingRoutePaymentPayload::CooperativeClose(StreamingRouteCooperativeClose {
                    final_paid_msat: 1_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub,
            config: config.clone(),
            now_unix: 130,
        })
        .expect("apply close");

    assert_eq!(result.payload_type, "cooperative_close");
    assert_eq!(result.state, PaidRouteAccessState::Suspended);
    assert!(!result.allow_routing);
    assert_eq!(
        store.channels["channel-1"].status,
        PaidRouteLifecycleStatus::Closing
    );
    assert_eq!(
        store.leases["lease-1"].status,
        PaidRouteLifecycleStatus::Closing
    );
    assert!(!store.seller_admissions(&config, 131)[0].allow_routing);
    let collection = store.seller_collection_states(&config, 131);
    assert_eq!(collection.len(), 1);
    assert!(collection[0].collectable);
    assert!(collection[0].manual_collect);

    assert!(
        store
            .mark_seller_channel_closed("channel-1", 1_000, 132)
            .expect("settled close")
    );
    assert_eq!(
        store.channels["channel-1"].status,
        PaidRouteLifecycleStatus::Closed
    );
}

#[test]
fn seller_manual_channel_close_suspends_admission() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);

    let changed = store
        .mark_seller_channel_closed("channel-1", 1_000, 130)
        .expect("mark closed");

    assert!(changed);
    assert_eq!(
        store.channels["channel-1"].status,
        PaidRouteLifecycleStatus::Closed
    );
    assert_eq!(store.channels["channel-1"].payment.paid_msat, 1_000);
    assert_eq!(
        store.leases["lease-1"].status,
        PaidRouteLifecycleStatus::Closed
    );
    assert_eq!(
        store.sessions["seller-session-lease-1"]
            .session
            .payment
            .paid_msat,
        1_000
    );
    let admissions = store.seller_admissions(&config, 131);
    assert_eq!(admissions.len(), 1);
    assert!(!admissions[0].allow_routing);
    assert!(
        !store
            .mark_seller_channel_closed("channel-1", 1_000, 131)
            .expect("idempotent mark")
    );
}

#[test]
fn seller_payment_rejects_overclaimed_spilman_close_without_mutating_store() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let seller_npub = seller.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");
    let mut config = sample_config();
    config.pricing.price_msat = 1_000;
    config.pricing.per_units = 100;
    config.channel.free_probe_units = 0;
    config.channel.grace_units = 0;

    let mut store = seller_store_with_open_channel(&seller, &buyer, &config);
    let before = store.clone();
    let error = store
        .apply_seller_payment(ApplyPaidRouteSellerPaymentRequest {
            envelope: seller_payment_envelope(
                "internet-exit",
                "lease-1",
                &buyer_npub,
                &seller_npub,
                130,
                StreamingRoutePaymentPayload::CooperativeClose(StreamingRouteCooperativeClose {
                    final_paid_msat: 2_000,
                    payment: sample_spilman_payment("channel-1", 1),
                }),
            ),
            seller_npub,
            config,
            now_unix: 130,
        })
        .expect_err("overclaimed close should fail");

    assert!(error.to_string().contains("does not match"));
    assert_eq!(store, before);
}

#[test]
fn paid_route_store_rejects_incompatible_buyer_mint() {
    let seller = Keys::generate();
    let buyer = Keys::generate();
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &sample_config(), None, 100)
            .expect("signed offer");
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 101)
        .expect("store offer");

    let error = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
            mint_url: Some("https://other-mint.example".to_string()),
            channel_capacity_sat: None,
            initial_paid_msat: 0,
            now_unix: 120,
        })
        .expect_err("incompatible mint is rejected");

    assert!(error.to_string().contains("not accepted"));
    assert!(store.sessions.is_empty());
}

#[test]
fn paid_route_store_upserts_newer_offer_and_merges_relays() {
    let seller = Keys::generate();
    let old =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &sample_config(), None, 100)
            .expect("old offer");
    let new =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &sample_config(), None, 200)
            .expect("new offer");
    let mut store = PaidRouteStore::default();

    assert!(
        store
            .upsert_signed_offer(old.clone(), vec!["wss://a.example".to_string()], 101)
            .expect("old insert")
    );
    assert!(
        store
            .upsert_signed_offer(old, vec!["wss://b.example".to_string()], 102)
            .expect("same offer relay merge")
    );
    assert!(
        store
            .upsert_signed_offer(new.clone(), vec!["wss://c.example".to_string()], 201)
            .expect("newer replace")
    );

    let key = paid_route_offer_store_key(&new.offer().expect("offer").seller_npub, "internet-exit");
    let record = &store.offers[&key];
    assert_eq!(record.signed_offer.event.created_at.as_secs(), 200);
    assert_eq!(record.first_seen_unix, 101);
    assert_eq!(record.last_seen_unix, 201);
    assert_eq!(record.relay_urls, vec!["wss://c.example"]);
}

#[test]
fn paid_route_store_persists_offer_rating_score() {
    let seller = Keys::generate();
    let old =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &sample_config(), None, 100)
            .expect("old offer");
    let new =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &sample_config(), None, 200)
            .expect("new offer");
    let seller_npub = old.offer().expect("offer").seller_npub;
    let key = paid_route_offer_store_key(&seller_npub, "internet-exit");
    let mut store = PaidRouteStore::default();

    store
        .upsert_signed_offer(old, vec!["wss://relay.example".to_string()], 101)
        .expect("store offer");
    assert!(store.upsert_offer_rating_score(&seller_npub, 80, 120));
    assert!(!store.upsert_offer_rating_score(&seller_npub, -80, 110));
    store
        .upsert_signed_offer(new, vec!["wss://relay.example".to_string()], 201)
        .expect("replace offer");

    let record = &store.offers[&key];
    assert_eq!(record.rating_score, Some(80));
    assert_eq!(record.rating_updated_at_unix, 120);

    assert!(store.upsert_offer_rating_score(&seller_npub, -120, 220));
    let record = &store.offers[&key];
    assert_eq!(record.rating_score, Some(-100));
    assert_eq!(record.rating_updated_at_unix, 220);
}

#[test]
fn best_rated_offer_key_prefers_good_then_newcomer_over_degraded() {
    let good_seller = Keys::generate();
    let newcomer_seller = Keys::generate();
    let bad_seller = Keys::generate();
    let good_offer = signed_paid_exit_offer_from_config(
        "internet-exit",
        &good_seller,
        &sample_config(),
        None,
        100,
    )
    .expect("good offer");
    let newcomer_offer = signed_paid_exit_offer_from_config(
        "internet-exit",
        &newcomer_seller,
        &sample_config(),
        None,
        100,
    )
    .expect("newcomer offer");
    let bad_offer = signed_paid_exit_offer_from_config(
        "internet-exit",
        &bad_seller,
        &sample_config(),
        None,
        100,
    )
    .expect("bad offer");
    let good = good_offer.offer().expect("good offer record");
    let newcomer = newcomer_offer.offer().expect("newcomer offer record");
    let bad = bad_offer.offer().expect("bad offer record");
    let good_key = paid_route_offer_store_key(&good.seller_npub, &good.offer_id);
    let newcomer_key = paid_route_offer_store_key(&newcomer.seller_npub, &newcomer.offer_id);
    let bad_key = paid_route_offer_store_key(&bad.seller_npub, &bad.offer_id);
    let mut store = PaidRouteStore::default();

    store
        .upsert_signed_offer(good_offer, vec!["wss://relay.example".to_string()], 100)
        .expect("store good");
    store
        .upsert_signed_offer(newcomer_offer, vec!["wss://relay.example".to_string()], 110)
        .expect("store newcomer");
    store
        .upsert_signed_offer(bad_offer, vec!["wss://relay.example".to_string()], 120)
        .expect("store bad");
    assert!(store.upsert_offer_rating_score(&good.seller_npub, 80, 130));
    assert!(store.upsert_offer_rating_score(&bad.seller_npub, -80, 130));

    assert_eq!(
        store.best_rated_offer_key().expect("best rated offer"),
        good_key
    );

    assert!(store.upsert_offer_rating_score(&good.seller_npub, -90, 140));
    assert_eq!(
        store.best_rated_offer_key().expect("newcomer before bad"),
        newcomer_key
    );

    assert!(store.upsert_offer_rating_score(&newcomer.seller_npub, -10, 150));
    assert_eq!(
        store.best_rated_offer_key().expect("least bad offer"),
        newcomer_key
    );
    assert_ne!(
        store.best_rated_offer_key().expect("not worse bad offer"),
        bad_key
    );
}

#[test]
fn unreadable_paid_route_store_is_discarded() {
    let scratch = ScratchDir::new("unreadable");
    let store_path = scratch.path().join("paid-routes.json");
    fs::write(&store_path, "not json").expect("write junk");

    let store = load_paid_route_store(&store_path).expect("load default");

    assert_eq!(store, PaidRouteStore::default());
}
