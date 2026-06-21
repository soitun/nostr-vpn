#[cfg(feature = "paid-exit")]
#[tokio::test]
async fn paid_exit_offer_publish_and_discover_roundtrips_through_local_relay() {
    use nostr_vpn_core::paid_routes::PaidRouteMeter;

    let relay = LocalNostrRelay::spawn().await;
    let mut app = AppConfig::generated();
    app.nostr.relays = vec![relay.url.clone()];
    app.paid_exit.enabled = true;
    app.paid_exit.pricing.meter = PaidRouteMeter::Bytes;
    app.paid_exit.pricing.price_msat = 750;
    app.paid_exit.pricing.per_units = 1_000_000;
    app.paid_exit.channel.accepted_mints = vec!["https://mint.example".to_string()];
    app.paid_exit.location.country_code = "fi".to_string();
    app.paid_exit.normalize();

    let keys = app.nostr_keys().expect("app keys");
    let signed = signed_paid_exit_offer_from_config(
        "internet-exit",
        &keys,
        &app.paid_exit,
        Some(local_paid_exit_quality_hint()),
        unix_timestamp(),
    )
    .expect("signed offer");
    let offer = signed.offer().expect("offer");

    let publish =
        publish_paid_exit_offer_to_relays(&app, &signed, std::slice::from_ref(&relay.url))
            .await
            .expect("publish paid exit offer");
    assert_eq!(publish["success_count"].as_u64(), Some(1));
    assert_eq!(publish["failed_count"].as_u64(), Some(0));

    let discovered =
        discover_paid_exit_offers_from_relays(&app, std::slice::from_ref(&relay.url), 1, 10, None)
            .await
            .expect("discover paid exit offers");
    assert_eq!(discovered.len(), 1);
    let discovered_offer = discovered[0].offer().expect("discovered offer");
    assert_eq!(discovered_offer.offer_id, offer.offer_id);
    assert_eq!(discovered_offer.seller_npub, offer.seller_npub);
    assert_eq!(discovered_offer.location.country_code, "FI");
    assert_eq!(discovered_offer.pricing.price_msat, 750);

    relay.stop().await;
}

#[cfg(feature = "paid-exit")]
struct LocalNostrRelay {
    url: String,
    shutdown: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "paid-exit")]
impl LocalNostrRelay {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local relay");
        let url = format!("ws://{}", listener.local_addr().expect("relay addr"));
        let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let (shutdown, shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(run_local_nostr_relay(listener, events, shutdown_rx));
        Self {
            url,
            shutdown: Some(shutdown),
            handle,
        }
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.handle.await;
    }
}

#[cfg(feature = "paid-exit")]
async fn run_local_nostr_relay(
    listener: TcpListener,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    continue;
                };
                let events = Arc::clone(&events);
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    while let Some(message) = ws.next().await {
                        let Ok(message) = message else {
                            break;
                        };
                        let Some(text) = relay_message_text(&message) else {
                            continue;
                        };
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
                            continue;
                        };
                        let Some(items) = value.as_array() else {
                            continue;
                        };
                        match items.first().and_then(serde_json::Value::as_str) {
                            Some("EVENT") => {
                                if let Some(event) = items.get(1).cloned() {
                                    let event_id = event
                                        .get("id")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or_default()
                                        .to_string();
                                    events.lock().expect("relay events lock").push(event);
                                    let ok = serde_json::json!(["OK", event_id, true, ""]);
                                    let _ = ws.send(Message::Text(ok.to_string().into())).await;
                                }
                            }
                            Some("REQ") => {
                                let Some(subscription_id) =
                                    items.get(1).and_then(serde_json::Value::as_str)
                                else {
                                    continue;
                                };
                                let snapshot = events.lock().expect("relay events lock").clone();
                                for event in snapshot {
                                    let response =
                                        serde_json::json!(["EVENT", subscription_id, event]);
                                    let _ =
                                        ws.send(Message::Text(response.to_string().into())).await;
                                }
                                let eose = serde_json::json!(["EOSE", subscription_id]);
                                let _ = ws.send(Message::Text(eose.to_string().into())).await;
                            }
                            Some("CLOSE") => break,
                            _ => {}
                        }
                    }
                });
            }
        }
    }
}

#[cfg(feature = "paid-exit")]
fn relay_message_text(message: &Message) -> Option<&str> {
    match message {
        Message::Text(text) => Some(text.as_ref()),
        _ => None,
    }
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_buy_and_use_select_public_exit_route() {
    use nostr_sdk::prelude::Keys;
    use nostr_vpn_core::paid_route_store::{PaidRouteStore, write_paid_route_store};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, signed_paid_exit_offer_from_config,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-use-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let mut app = AppConfig::generated();
    app.connect_to_non_roster_fips_peers = false;
    app.fips_nostr_discovery_enabled = false;
    app.wireguard_exit.enabled = true;
    app.save(&config_path).expect("save buyer config");

    let seller = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 1_000_000;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");
    let offer = signed_offer.offer().expect("offer");

    let store_path = paid_route_store_file_path(&config_path);
    let mut store = PaidRouteStore::default();
    store.upsert_wallet_mint("https://mint.example", "Example", None, 122);
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    write_paid_route_store(&store_path, &store).expect("write store");

    let buy = paid_exit_buy_once(PaidExitBuyArgs {
        config: Some(config_path.clone()),
        offer: "internet-exit".to_string(),
        mint: None,
        channel_capacity_sat: Some(10),
        initial_paid_msat: 0,
        no_select_exit_node: false,
        no_reload_daemon: true,
        json: false,
    })
    .expect("buy paid exit");

    assert_eq!(buy.session.seller_npub, offer.seller_npub);
    let seller_hex = seller.public_key().to_hex();
    assert_eq!(buy.selected_exit_node.as_deref(), Some(seller_hex.as_str()));
    assert!(!buy.daemon_reload_attempted);
    let saved = AppConfig::load(&config_path).expect("load selected config");
    assert_eq!(saved.exit_node, seller_hex);
    assert!(saved.connect_to_non_roster_fips_peers);
    assert!(saved.fips_nostr_discovery_enabled);
    assert!(!saved.wireguard_exit.enabled);

    let mut reset = saved;
    reset.exit_node.clear();
    reset.connect_to_non_roster_fips_peers = false;
    reset.fips_nostr_discovery_enabled = false;
    reset.wireguard_exit.enabled = true;
    reset.save(&config_path).expect("save reset config");

    let selected = paid_exit_use_once(PaidExitUseArgs {
        config: Some(config_path.clone()),
        session: buy.session.session_id,
        no_reload_daemon: true,
        json: false,
    })
    .expect("use paid exit session");

    assert_eq!(selected.seller_npub, offer.seller_npub);
    assert_eq!(selected.selected_exit_node, seller_hex);
    assert!(!selected.daemon_reload_attempted);
    let saved = AppConfig::load(&config_path).expect("load used config");
    assert_eq!(saved.exit_node, seller_hex);
    assert!(saved.connect_to_non_roster_fips_peers);
    assert!(saved.fips_nostr_discovery_enabled);
    assert!(!saved.wireguard_exit.enabled);

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[tokio::test]
async fn paid_exit_create_payment_command_updates_buyer_session() {
    use cashu_service::CashuSpilmanPayment;
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, signed_paid_exit_offer_from_config,
    };
    use serde_json::json;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-create-payment-{nonce}"));
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
    write_paid_route_store(&store_path, &store).expect("write store");

    paid_exit_create_payment_command(PaidExitCreatePaymentArgs {
        config: Some(config_path.clone()),
        session: session.session_id.clone(),
        kind: PaidExitCreatePaymentKind::BalanceUpdate,
        payment: Some(
            serde_json::to_string(&runtime_spilman_payment(&session.channel_id, 1))
                .expect("serialize payment"),
        ),
        payment_stdin: false,
        open_from_wallet: false,
        sign_from_wallet: false,
        mint: None,
        keyset_id: None,
        keyset_info: None,
        keyset_info_file: None,
        max_amount_per_output: 64,
        delivered_units: Some(100),
        paid_msat: Some(1_000),
        json: false,
    })
    .await
    .expect("create buyer payment");

    let store = load_paid_route_store(&store_path).expect("load store");
    let record = &store.sessions[&session.session_id];
    assert_eq!(record.session.usage.rx_bytes, 100);
    assert_eq!(record.session.payment.paid_msat, 1_000);
    assert_eq!(
        record
            .session
            .payment
            .cashu_spilman_payment
            .as_ref()
            .map(|payment| payment.balance),
        Some(1)
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
async fn paid_exit_stream_payments_signs_due_buyer_usage_update() {
    use cashu_service::{CashuSpilmanPayment, CashuSpilmanPaymentSigner};
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{
        OpenPaidRouteBuyerSessionRequest, PaidRouteBuyerPaymentUpdatesDueRequest, PaidRouteStore,
        RecordPaidRouteBuyerUsageRequest,
    };
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, PaidRouteUsage, signed_paid_exit_offer_from_config,
    };
    use serde_json::json;

    let app = AppConfig::generated();
    let buyer_keys = app.nostr_keys().expect("buyer keys");
    let buyer_npub = buyer_keys.public_key().to_bech32().expect("buyer npub");
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

    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let session = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer_npub.clone(),
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 0,
            now_unix: 125,
        })
        .expect("open buyer session");
    store
        .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
            seller_pubkey: seller.public_key().to_hex(),
            usage_delta: PaidRouteUsage {
                rx_bytes: 60,
                tx_bytes: 50,
                ..PaidRouteUsage::default()
            },
            now_unix: 126,
        })
        .expect("record buyer usage")
        .expect("matched buyer session");

    let mut due = store.buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
        now_unix: 127,
        min_increment_msat: 1,
    });
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].session_id, session.session_id);
    assert_eq!(due[0].delivered_units, 110);
    assert_eq!(due[0].target_paid_msat, 2_000);

    let result = paid_exit_stream_payment_updates_with_signer(
        &app,
        &buyer_keys,
        &mut store,
        &RuntimeFakePaymentSigner,
        &buyer_npub,
        std::mem::take(&mut due),
        &[],
        false,
        128,
    )
    .await;

    assert!(result.changed);
    assert_eq!(result.signed.len(), 1);
    assert_eq!(result.persisted_count(), 1);
    assert!(result.errors.is_empty());
    assert_eq!(
        result.signed[0]["due"]["target_paid_msat"].as_u64(),
        Some(2_000)
    );
    assert_eq!(
        result.signed[0]["payment"]["paid_msat"].as_u64(),
        Some(2_000)
    );
    assert_eq!(result.signed[0]["persisted"].as_bool(), Some(true));

    let record = &store.sessions[&session.session_id];
    assert_eq!(record.session.usage.rx_bytes, 60);
    assert_eq!(record.session.usage.tx_bytes, 50);
    assert_eq!(record.session.payment.paid_msat, 2_000);
    assert_eq!(
        record
            .session
            .payment
            .cashu_spilman_payment
            .as_ref()
            .map(|payment| payment.balance),
        Some(2)
    );
    assert!(
        store
            .buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
                now_unix: 129,
                min_increment_msat: 1,
            })
            .is_empty()
    );

    struct RuntimeFakePaymentSigner;

    impl CashuSpilmanPaymentSigner for RuntimeFakePaymentSigner {
        fn sign_cashu_spilman_payment(
            &self,
            channel_id: &str,
            balance: u64,
            include_funding: bool,
        ) -> std::result::Result<CashuSpilmanPayment, String> {
            Ok(CashuSpilmanPayment {
                channel_id: channel_id.to_string(),
                balance,
                signature: format!("signed-{channel_id}-{balance}"),
                params: include_funding.then(|| json!({"channel": channel_id})),
                funding_proofs: include_funding.then(|| json!({"proofs": []})),
            })
        }
    }
}

#[cfg(feature = "paid-exit")]
#[tokio::test]
async fn paid_exit_settle_signs_manual_cooperative_close_from_wallet() {
    use cashu_service::{CashuSpilmanPayment, CashuSpilmanPaymentSigner};
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{
        OpenPaidRouteBuyerSessionRequest, PaidRouteBuyerPaymentUpdatesDueRequest,
        PaidRouteLifecycleStatus, PaidRouteStore, RecordPaidRouteBuyerUsageRequest,
    };
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, PaidRouteUsage, signed_paid_exit_offer_from_config,
    };
    use serde_json::json;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-settle-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");

    let app = AppConfig::generated();
    let buyer_keys = app.nostr_keys().expect("buyer keys");
    let buyer_npub = buyer_keys.public_key().to_bech32().expect("buyer npub");
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
        .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
            seller_pubkey: seller.public_key().to_hex(),
            usage_delta: PaidRouteUsage {
                rx_bytes: 60,
                tx_bytes: 50,
                ..PaidRouteUsage::default()
            },
            now_unix: 126,
        })
        .expect("record buyer usage")
        .expect("matched buyer session");

    let result = paid_exit_settle_with_signer(
        &app,
        &buyer_keys,
        &mut store,
        &RuntimeFakePaymentSigner,
        &session.session_id,
        &[],
        false,
        &dir.join("wallet"),
        128,
    )
    .await
    .expect("settle channel");

    assert!(result.payment.changed);
    assert_eq!(result.payment.payload_type, "cooperative_close");
    assert_eq!(result.payment.session_id, session.session_id);
    assert_eq!(result.payment.channel_id, session.channel_id);
    assert_eq!(result.payment.lease_id, session.lease_id);
    assert_eq!(result.payment.delivered_units, 110);
    assert_eq!(result.payment.amount_due_msat, 1_100);
    assert_eq!(result.payment.paid_msat, 2_000);
    assert!(!result.publish_requested);
    assert!(result.publish.is_none());
    assert!(result.relays.is_empty());
    assert!(result.persisted);

    let channel = store.channels.get(&session.channel_id).expect("channel");
    assert_eq!(channel.status, PaidRouteLifecycleStatus::Closed);
    assert_eq!(channel.payment.paid_msat, 2_000);
    assert_eq!(
        channel
            .payment
            .cashu_spilman_payment
            .as_ref()
            .map(|payment| payment.balance),
        Some(2)
    );
    let expected_signature = format!("closed-{}-2", session.channel_id);
    assert_eq!(
        channel
            .payment
            .cashu_spilman_payment
            .as_ref()
            .map(|payment| payment.signature.as_str()),
        Some(expected_signature.as_str())
    );
    let lease = store.leases.get(&session.lease_id).expect("lease");
    assert_eq!(lease.status, PaidRouteLifecycleStatus::Closed);
    assert!(
        store
            .buyer_payment_updates_due(PaidRouteBuyerPaymentUpdatesDueRequest {
                now_unix: 129,
                min_increment_msat: 1,
            })
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(&dir);

    struct RuntimeFakePaymentSigner;

    impl CashuSpilmanPaymentSigner for RuntimeFakePaymentSigner {
        fn sign_cashu_spilman_payment(
            &self,
            channel_id: &str,
            balance: u64,
            include_funding: bool,
        ) -> std::result::Result<CashuSpilmanPayment, String> {
            Ok(CashuSpilmanPayment {
                channel_id: channel_id.to_string(),
                balance,
                signature: format!("signed-{channel_id}-{balance}"),
                params: include_funding.then(|| json!({"channel": channel_id})),
                funding_proofs: include_funding.then(|| json!({"proofs": []})),
            })
        }

        fn sign_cashu_spilman_close(
            &self,
            channel_id: &str,
            final_balance: u64,
        ) -> std::result::Result<CashuSpilmanPayment, String> {
            Ok(CashuSpilmanPayment {
                channel_id: channel_id.to_string(),
                balance: final_balance,
                signature: format!("closed-{channel_id}-{final_balance}"),
                params: None,
                funding_proofs: None,
            })
        }
    }
}

