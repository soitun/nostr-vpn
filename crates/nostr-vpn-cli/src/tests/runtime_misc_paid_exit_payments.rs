#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_create_token_lease_command_updates_buyer_session() {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, PaidRoutePaymentMode, signed_paid_exit_offer_from_config,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-create-token-lease-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let app = AppConfig::generated();
    app.save(&config_path).expect("save buyer config");

    let seller = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 100;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    offer_config.channel.free_probe_units = 0;
    offer_config.channel.grace_units = 0;
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");

    let buyer_npub = app
        .nostr_keys()
        .expect("buyer keys")
        .public_key()
        .to_bech32()
        .expect("buyer npub");
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let session = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub,
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 0,
            now_unix: 125,
        })
        .expect("open buyer session");
    store
        .sessions
        .get_mut(&session.session_id)
        .expect("buyer session")
        .session
        .usage
        .rx_bytes = 100;
    write_paid_route_store(&store_path, &store).expect("write store");

    paid_exit_create_token_lease_command(PaidExitCreateTokenLeaseArgs {
        config: Some(config_path.clone()),
        session: session.session_id.clone(),
        token: Some("cashuBdevtoken".to_string()),
        token_stdin: false,
        mint: Some("https://mint.example".to_string()),
        unit: "sat".to_string(),
        amount: 2,
        paid_msat: Some(1_500),
        expires_at_unix: Some(unix_timestamp() + 600),
        json: false,
    })
    .expect("create token lease");

    let store = load_paid_route_store(&store_path).expect("load store");
    let record = &store.sessions[&session.session_id];
    assert_eq!(record.session.usage.rx_bytes, 100);
    assert_eq!(
        record.session.payment.mode,
        PaidRoutePaymentMode::CashuTokenLease
    );
    assert_eq!(record.session.payment.paid_msat, 1_500);
    assert!(record.session.payment.cashu_spilman_payment.is_none());
    let token_lease = record
        .session
        .payment
        .cashu_token_lease
        .as_ref()
        .expect("token lease");
    assert_eq!(token_lease.token, "cashuBdevtoken");
    assert_eq!(token_lease.amount, 2);
    assert_eq!(token_lease.paid_msat, 1_500);
    let snapshot = paid_exit_status_snapshot_json(&app, &store_path, &store);
    let status_token_lease = &snapshot["sessions"][0]["payment"]["cashu_token_lease"];
    assert_eq!(status_token_lease["amount"].as_u64(), Some(2));
    assert_eq!(status_token_lease["paid_msat"].as_u64(), Some(1_500));
    assert_eq!(status_token_lease["has_token"].as_bool(), Some(true));
    assert!(status_token_lease.get("token").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_apply_payment_command_updates_seller_admission() {
    use cashu_service::{
        CashuSpilmanPayment, StreamingRouteBalanceUpdate, StreamingRouteChannelOpen,
        StreamingRoutePaymentEnvelope, StreamingRoutePaymentPayload,
    };
    use nostr_sdk::prelude::{Keys, ToBech32};
    use serde_json::json;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-apply-payment-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime
        .block_on(paid_exit_run_once(PaidExitRunArgs {
            config: Some(config_path.clone()),
            offer_id: Some("internet-exit".to_string()),
            relays: vec![],
            publish: false,
            no_reload_daemon: true,
            upstream: Some("host-default".to_string()),
            meter: Some("bytes".to_string()),
            price_msat: Some(1_000),
            per_units: Some("100".to_string()),
            accepted_mints: Some("https://mint.example".to_string()),
            accepted_mint: vec![],
            country_code: Some("fi".to_string()),
            region: None,
            asn: None,
            network_class: Some("datacenter".to_string()),
            ipv4: Some(true),
            ipv6: Some(false),
            max_channel_capacity_sat: Some(10),
            channel_expiry_secs: Some(600),
            free_probe_units: Some("0".to_string()),
            grace_units: Some("0".to_string()),
            json: false,
        }))
        .expect("configure seller");

    let app = load_or_default_config(&config_path).expect("load seller config");
    let seller_npub = app
        .nostr_keys()
        .expect("seller keys")
        .public_key()
        .to_bech32()
        .expect("seller npub");
    let buyer = Keys::generate();
    let buyer_npub = buyer.public_key().to_bech32().expect("buyer npub");

    runtime
        .block_on(paid_exit_apply_payment_command(PaidExitApplyPaymentArgs {
            config: Some(config_path.clone()),
            envelope: Some(
                serde_json::to_string(&StreamingRoutePaymentEnvelope::new(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    100,
                    StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                        mint_url: "https://mint.example".to_string(),
                        unit: "sat".to_string(),
                        capacity: 10,
                        expires_unix: 0,
                        receiver_pubkey_hex: app
                            .nostr_keys()
                            .expect("seller keys")
                            .public_key()
                            .to_hex(),
                        paid_msat: 0,
                        payment: runtime_spilman_payment("channel-1", 0),
                    }),
                ))
                .expect("serialize open"),
            ),
            envelope_stdin: false,
            no_reload_daemon: true,
            json: false,
        }))
        .expect("apply channel open");
    runtime
        .block_on(paid_exit_apply_payment_command(PaidExitApplyPaymentArgs {
            config: Some(config_path.clone()),
            envelope: Some(
                serde_json::to_string(&StreamingRoutePaymentEnvelope::new(
                    "internet-exit",
                    "lease-1",
                    &buyer_npub,
                    &seller_npub,
                    101,
                    StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
                        delivered_units: 100,
                        amount_due_msat: 1_000,
                        paid_msat: 1_000,
                        payment: runtime_spilman_payment("channel-1", 1),
                    }),
                ))
                .expect("serialize update"),
            ),
            envelope_stdin: false,
            no_reload_daemon: true,
            json: false,
        }))
        .expect("apply balance update");

    let store =
        load_paid_route_store(&paid_route_store_file_path(&config_path)).expect("load store");
    let admissions = store.seller_admissions(&app.paid_exit, unix_timestamp());
    assert_eq!(admissions.len(), 1);
    assert_eq!(admissions[0].buyer_npub, buyer_npub);
    assert!(admissions[0].allow_routing);
    assert_eq!(admissions[0].paid_msat, 1_000);
    assert_eq!(admissions[0].amount_due_msat, 1_000);
    let snapshot =
        paid_exit_status_snapshot_json(&app, &paid_route_store_file_path(&config_path), &store);
    assert_eq!(
        snapshot["sessions"][0]["routing"]["state"].as_str(),
        Some("paid")
    );
    assert_eq!(
        snapshot["sessions"][0]["routing"]["amount_due_msat"].as_u64(),
        Some(1_000)
    );
    assert_eq!(
        snapshot["sessions"][0]["routing"]["allow_routing"].as_bool(),
        Some(true)
    );
    assert_eq!(
        snapshot["seller_accounting"]["pending_buyer_credit_msat"].as_u64(),
        Some(1_000)
    );
    assert_eq!(
        snapshot["seller_accounting"]["pending_buyer_credit_text"].as_str(),
        Some("1 sat")
    );
    assert_eq!(
        snapshot["seller_accounting"]["pending_buyer_credit_help_text"].as_str(),
        Some("collect to move it into wallet")
    );
    assert_eq!(
        snapshot["seller_accounting"]["collectable_channel_count"].as_u64(),
        Some(1)
    );
    assert_eq!(
        snapshot["seller_accounting"]["auto_collect_due_count"].as_u64(),
        Some(0)
    );
    assert_eq!(
        snapshot["seller_accounting"]["auto_collect_due_msat"].as_u64(),
        Some(0)
    );
    assert_eq!(
        snapshot["seller_collection"][0]["manual_collect"].as_bool(),
        Some(true)
    );
    assert_eq!(
        snapshot["seller_collection"][0]["auto_collect_due"].as_bool(),
        Some(false)
    );
    assert_eq!(
        snapshot["sessions"][0]["collection"]["reason"].as_str(),
        Some("manual")
    );

    let _ = std::fs::remove_dir_all(&dir);

    fn runtime_spilman_payment(channel_id: &str, balance: u64) -> CashuSpilmanPayment {
        CashuSpilmanPayment {
            channel_id: channel_id.to_string(),
            balance,
            signature: format!("signature-{channel_id}-{balance}"),
            params: Some(json!({"channel": channel_id})),
            funding_proofs: Some(json!({"proofs": []})),
        }
    }
}

#[cfg(feature = "paid-exit")]
#[tokio::test]
async fn paid_exit_buyer_payment_roundtrips_through_local_relay() {
    use cashu_service::CashuSpilmanPayment;
    use nostr_sdk::prelude::ToBech32;
    use nostr_vpn_core::paid_route_store::{
        BuildPaidRouteBuyerPaymentEnvelopeKind, BuildPaidRouteBuyerPaymentEnvelopeRequest,
    };
    use serde_json::json;

    let offer_relay = LocalNostrRelay::spawn().await;
    let payment_relay = LocalNostrRelay::spawn().await;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-payment-relay-{nonce}"));
    let seller_dir = dir.join("seller");
    let buyer_dir = dir.join("buyer");
    std::fs::create_dir_all(&seller_dir).expect("create seller test dir");
    std::fs::create_dir_all(&buyer_dir).expect("create buyer test dir");
    let seller_config_path = seller_dir.join("config.toml");
    let buyer_config_path = buyer_dir.join("config.toml");

    paid_exit_run_once(PaidExitRunArgs {
        config: Some(seller_config_path.clone()),
        offer_id: Some("internet-exit".to_string()),
        relays: vec![offer_relay.url.clone()],
        publish: true,
        no_reload_daemon: true,
        upstream: Some("host-default".to_string()),
        meter: Some("bytes".to_string()),
        price_msat: Some(1_000),
        per_units: Some("100".to_string()),
        accepted_mints: Some("https://mint.example".to_string()),
        accepted_mint: vec![],
        country_code: Some("fi".to_string()),
        region: None,
        asn: None,
        network_class: Some("datacenter".to_string()),
        ipv4: Some(true),
        ipv6: Some(false),
        max_channel_capacity_sat: Some(10),
        channel_expiry_secs: Some(600),
        free_probe_units: Some("0".to_string()),
        grace_units: Some("0".to_string()),
        json: false,
    })
    .await
    .expect("publish seller offer");

    let mut buyer_app = AppConfig::generated();
    buyer_app.nostr.relays = vec![offer_relay.url.clone(), payment_relay.url.clone()];
    buyer_app
        .save(&buyer_config_path)
        .expect("save buyer config");
    paid_exit_discover_command(PaidExitDiscoverArgs {
        config: Some(buyer_config_path.clone()),
        relays: vec![offer_relay.url.clone()],
        duration_secs: 1,
        limit: 10,
        since_secs: 0,
        json: false,
    })
    .await
    .expect("discover paid exit offer");

    let buy = paid_exit_buy_once(PaidExitBuyArgs {
        config: Some(buyer_config_path.clone()),
        offer: "internet-exit".to_string(),
        mint: Some("https://mint.example".to_string()),
        channel_capacity_sat: Some(10),
        initial_paid_msat: 0,
        no_select_exit_node: false,
        no_reload_daemon: true,
        json: false,
    })
    .expect("buy paid exit");
    let buyer_npub = buyer_app
        .nostr_keys()
        .expect("buyer keys")
        .public_key()
        .to_bech32()
        .expect("buyer npub");

    let buyer_store_path = paid_route_store_file_path(&buyer_config_path);
    let mut buyer_store = load_paid_route_store(&buyer_store_path).expect("load buyer store");
    let channel_open = buyer_store
        .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
            session_id: buy.session.session_id.clone(),
            buyer_npub: buyer_npub.clone(),
            kind: BuildPaidRouteBuyerPaymentEnvelopeKind::ChannelOpen,
            payment: relay_spilman_payment(&buy.session.channel_id, 0),
            delivered_units: Some(0),
            paid_msat: Some(0),
            now_unix: unix_timestamp(),
        })
        .expect("build channel-open envelope");
    let balance_update = buyer_store
        .build_buyer_payment_envelope(BuildPaidRouteBuyerPaymentEnvelopeRequest {
            session_id: buy.session.session_id.clone(),
            buyer_npub: buyer_npub.clone(),
            kind: BuildPaidRouteBuyerPaymentEnvelopeKind::BalanceUpdate,
            payment: relay_spilman_payment(&buy.session.channel_id, 1),
            delivered_units: Some(100),
            paid_msat: Some(1_000),
            now_unix: unix_timestamp(),
        })
        .expect("build balance update envelope");
    write_paid_route_store(&buyer_store_path, &buyer_store).expect("write buyer store");

    for envelope in [channel_open.envelope, balance_update.envelope] {
        paid_exit_send_payment_command(PaidExitSendPaymentArgs {
            config: Some(buyer_config_path.clone()),
            relays: vec![payment_relay.url.clone()],
            envelope: Some(serde_json::to_string(&envelope).expect("serialize envelope")),
            envelope_stdin: false,
            json: false,
        })
        .await
        .expect("send payment over relay");
    }

    paid_exit_receive_payments_command(PaidExitReceivePaymentsArgs {
        config: Some(seller_config_path.clone()),
        relays: vec![payment_relay.url.clone()],
        duration_secs: 1,
        limit: 10,
        since_secs: 0,
        no_reload_daemon: true,
        json: false,
    })
    .await
    .expect("receive seller payments");

    let seller_app = load_or_default_config(&seller_config_path).expect("load seller config");
    let seller_store_path = paid_route_store_file_path(&seller_config_path);
    let seller_store = load_paid_route_store(&seller_store_path).expect("load seller store");
    let admissions = seller_store.seller_admissions(&seller_app.paid_exit, unix_timestamp());
    assert_eq!(admissions.len(), 1);
    assert_eq!(admissions[0].buyer_npub, buyer_npub);
    assert!(admissions[0].allow_routing);
    assert_eq!(admissions[0].paid_msat, 1_000);
    assert_eq!(admissions[0].amount_due_msat, 1_000);
    let snapshot = paid_exit_status_snapshot_json(&seller_app, &seller_store_path, &seller_store);
    assert_eq!(
        snapshot["seller_admissions"][0]["state"].as_str(),
        Some("paid")
    );
    assert_eq!(
        snapshot["sessions"][0]["routing"]["allow_routing"].as_bool(),
        Some(true)
    );

    offer_relay.stop().await;
    payment_relay.stop().await;
    let _ = std::fs::remove_dir_all(&dir);

    fn relay_spilman_payment(channel_id: &str, balance: u64) -> CashuSpilmanPayment {
        CashuSpilmanPayment {
            channel_id: channel_id.to_string(),
            balance,
            signature: format!("signature-{channel_id}-{balance}"),
            params: Some(json!({"channel": channel_id})),
            funding_proofs: Some(json!({"proofs": []})),
        }
    }
}

