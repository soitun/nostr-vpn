#[cfg(feature = "paid-exit")]
fn paid_route_channel_open_frame(
    buyer_npub: &str,
    seller_npub: &str,
    seller_pubkey: &str,
    now_unix: u64,
) -> FipsControlFrame {
    let frame: FipsControlFrame = serde_json::from_value(serde_json::json!({
        "kind": "paid_route_payment",
        "id": "pending",
        "envelope": {
            "version": 1,
            "service_id": "mobile-paid-exit",
            "lease_id": "mobile-lease-1",
            "buyer": buyer_npub,
            "seller": seller_npub,
            "sent_at_unix": now_unix,
            "payload": {
                "type": "channel_open",
                "mint_url": "https://mint.example",
                "unit": "sat",
                "capacity": 10,
                "expires_unix": now_unix.saturating_add(600),
                "receiver_pubkey_hex": seller_pubkey,
                "paid_msat": 0,
                "payment": {
                    "channel_id": "mobile-channel-1",
                    "balance": 0,
                    "signature": "s".repeat(2_000),
                    "params": {"channel": "mobile-channel-1"},
                    "funding_proofs": {"proofs": []}
                }
            }
        }
    }))
    .expect("decode paid route payment frame");
    let FipsControlFrame::PaidRoutePayment { envelope, .. } = frame else {
        panic!("expected paid route payment frame");
    };
    let id =
        nostr_vpn_core::paid_route_store::paid_route_payment_id(&envelope).expect("payment id");
    FipsControlFrame::PaidRoutePayment { id, envelope }
}

#[cfg(feature = "paid-exit")]
async fn send_control_messages_until_received(
    sender: &FipsEndpoint,
    recipient: &FipsEndpoint,
    recipient_peer: PeerIdentity,
    frame: &FipsControlFrame,
) -> Vec<FipsEndpointMessage> {
    let encoded = encode_fips_control_messages(frame).expect("encode FIPS control messages");
    let mut received = Vec::with_capacity(encoded.len());
    let mut batch = Vec::with_capacity(encoded.len());
    for _ in 0..50 {
        sender
            .send_batch_to_peer(recipient_peer, encoded.clone())
            .await
            .expect("send FIPS control messages");
        match tokio::time::timeout(
            Duration::from_millis(100),
            recipient.recv_batch_into(&mut batch, encoded.len()),
        )
        .await
        {
            Ok(Some(count)) if count > 0 => {
                for message in batch.drain(..) {
                    if message.source_peer.npub() == sender.npub()
                        && !received.iter().any(|existing: &FipsEndpointMessage| {
                            existing.data.as_slice() == message.data.as_slice()
                        })
                    {
                        received.push(message);
                    }
                }
                if received.len() == encoded.len() {
                    return received;
                }
            }
            Ok(Some(_) | None) | Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("recipient should receive complete FIPS control frame");
}

#[cfg(feature = "paid-exit")]
async fn receive_control_message_until_received(
    sender: &FipsEndpoint,
    recipient: &FipsEndpoint,
) -> FipsEndpointMessage {
    let mut messages = Vec::with_capacity(1);
    for _ in 0..50 {
        match tokio::time::timeout(
            Duration::from_millis(100),
            recipient.recv_batch_into(&mut messages, 1),
        )
        .await
        {
            Ok(Some(count)) if count > 0 => {
                if let Some(index) = messages
                    .iter()
                    .position(|message| message.source_peer.npub() == sender.npub())
                {
                    return messages.swap_remove(index);
                }
                messages.clear();
            }
            Ok(Some(_) | None) | Err(_) => {}
        }
    }
    panic!("recipient should receive FIPS control reply");
}

#[cfg(feature = "paid-exit")]
#[test]
fn mobile_paid_route_payment_and_ack_roundtrip_over_real_fips_endpoint() {
    std::thread::Builder::new()
        .name("mobile-paid-route-fips".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mobile paid route test runtime")
                .block_on(mobile_paid_route_payment_and_ack_roundtrip());
        })
        .expect("spawn mobile paid route test")
        .join()
        .expect("mobile paid route test thread");
}

#[cfg(feature = "paid-exit")]
#[allow(clippy::too_many_lines)]
async fn mobile_paid_route_payment_and_ack_roundtrip() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-mobile-paid-route-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let seller_config_path = dir.join("seller/config.toml");
    let buyer_config_path = dir.join("buyer/config.toml");

    let seller_keys = Keys::generate();
    let buyer_keys = Keys::generate();
    let seller_pubkey = seller_keys.public_key().to_hex();
    let buyer_pubkey = buyer_keys.public_key().to_hex();
    let seller_npub = seller_keys.public_key().to_bech32().expect("seller npub");
    let buyer_npub = buyer_keys.public_key().to_bech32().expect("buyer npub");
    let seller_nsec = seller_keys.secret_key().to_bech32().expect("seller nsec");
    let buyer_nsec = buyer_keys.secret_key().to_bech32().expect("buyer nsec");
    let network_id = format!("mobile-paid-route-{nonce}");
    let scope = format!("nostr-vpn:{network_id}");

    let seller_mobile = fips_exit_mobile_config(
        seller_nsec.clone(),
        &seller_pubkey,
        &network_id,
        available_udp_port(),
    );
    let seller_endpoint = bind_local_mobile_endpoint(&scope, &seller_mobile).await;
    let seller_peer = FipsMeshPeerConfig::from_participant_pubkey(&seller_pubkey, Vec::new())
        .expect("seller peer");
    let mut buyer_hints = HashMap::new();
    buyer_hints.insert(
        seller_pubkey.clone(),
        vec![FipsPeerAddressHint {
            addr: format!("127.0.0.1:{}", seller_mobile.listen_port),
            seen_at_ms: None,
            priority: FIPS_STATIC_PEER_ENDPOINT_PRIORITY,
        }],
    );
    let buyer_mobile = MobileTunnelConfig {
        identity_nsec: buyer_nsec.clone(),
        node_name: "paid-route-buyer".to_string(),
        network_id: network_id.clone(),
        local_address: derive_mesh_tunnel_ip(&network_id, &buyer_pubkey).expect("buyer tunnel ip"),
        listen_port: available_udp_port(),
        peers: vec![seller_peer],
        peer_hints: buyer_hints,
        nostr_discovery_enabled: false,
        ..empty_config()
    };
    let buyer_endpoint = bind_local_mobile_endpoint(&scope, &buyer_mobile).await;

    let now_unix = unix_timestamp();
    let payment =
        paid_route_channel_open_frame(&buyer_npub, &seller_npub, &seller_pubkey, now_unix);
    let (payment_id, payment_envelope) = match &payment {
        FipsControlFrame::PaidRoutePayment { id, envelope } => (id.clone(), envelope.clone()),
        _ => panic!("expected payment frame"),
    };
    let encoded = encode_fips_control_messages(&payment).expect("encode payment fragments");
    assert!(
        encoded.len() > 1,
        "test payment must exercise fragmentation"
    );

    let outbox =
        nostr_vpn_core::paid_route_store::paid_route_payment_outbox_directory(&buyer_config_path);
    std::fs::create_dir_all(&outbox).expect("create payment outbox");
    let outbox_entry = outbox.join(format!("{payment_id}.json"));
    std::fs::write(
        &outbox_entry,
        serde_json::to_vec(&payment_envelope).expect("encode payment envelope"),
    )
    .expect("write payment outbox entry");

    let mut seller_app = AppConfig::generated();
    seller_app.nostr.public_key = seller_pubkey.clone();
    seller_app.nostr.secret_key = seller_nsec;
    seller_app.paid_exit.enabled = true;
    seller_app.paid_exit.pricing.price_msat = 2_500;
    seller_app.paid_exit.pricing.per_units = 1_000_000;
    seller_app.paid_exit.channel.accepted_mints = vec!["https://mint.example".to_string()];
    seller_app.paid_exit.channel.max_channel_capacity_sat = 100;
    seller_app.paid_exit.channel.channel_expiry_secs = 600;
    seller_app.paid_exit.channel.free_probe_units = 1_048_576;
    seller_app.ensure_defaults();

    let seller_mesh = new_mobile_mesh(FipsMeshRuntime::with_local_routes(
        Vec::new(),
        vec![seller_mobile.local_address.clone()],
    ));
    let seller_mesh_peers = Arc::new(RwLock::new(Vec::new()));
    let seller_peer_identities = Arc::new(RwLock::new(MobilePeerIdentityMap::default()));
    let seller_peer_hints = Arc::new(RwLock::new(HashMap::new()));
    let seller_presence = Arc::new(RwLock::new(HashMap::new()));
    let seller_config_state = Arc::new(RwLock::new(seller_mobile));
    let seller_app_config = Arc::new(RwLock::new(seller_app));
    let seller_dirty = AtomicBool::new(false);
    let seller_join = AtomicBool::new(false);
    let seller_control = MobileEndpointReceiveContext {
        endpoint: &seller_endpoint,
        mesh: &seller_mesh,
        mesh_peers: &seller_mesh_peers,
        peer_identities: &seller_peer_identities,
        peer_hints: &seller_peer_hints,
        presence: &seller_presence,
        config_state: &seller_config_state,
        app_config: &seller_app_config,
        app_config_dirty: &seller_dirty,
        config_path: Some(&seller_config_path),
        network_id: &network_id,
        join_request_active: &seller_join,
    };
    let seller_identity =
        PeerIdentity::from_npub(seller_endpoint.npub()).expect("seller endpoint identity");
    let forged_buyer = Keys::generate()
        .public_key()
        .to_bech32()
        .expect("forged buyer npub");
    let forged_payment =
        paid_route_channel_open_frame(&forged_buyer, &seller_npub, &seller_pubkey, now_unix);
    let forged_messages = send_control_messages_until_received(
        &buyer_endpoint,
        &seller_endpoint,
        seller_identity,
        &forged_payment,
    )
    .await;
    let mut forged_fragments = FipsControlFragmentBuffer::default();
    let mut forged_packets = Vec::new();
    let mut forged_error = None;
    for message in forged_messages {
        match handle_mobile_endpoint_message(
            &seller_control,
            &mut forged_fragments,
            &mut forged_packets,
            message,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => panic!("payment control must be consumed"),
            Err(error) => forged_error = Some(error),
        }
    }
    assert!(
        forged_packets.is_empty(),
        "rejected payment is never tunnel data"
    );
    assert!(
        forged_error.is_some_and(|error| error
            .to_string()
            .contains("buyer does not match authenticated FIPS source")),
        "spoofed payment must be source-bound"
    );
    assert!(
        !nostr_vpn_core::paid_route_store::paid_route_store_file_path(&seller_config_path).exists(),
        "rejected payment must not create seller state"
    );

    let payment_messages = send_control_messages_until_received(
        &buyer_endpoint,
        &seller_endpoint,
        seller_identity,
        &payment,
    )
    .await;
    let mut seller_fragments = FipsControlFragmentBuffer::default();
    let mut seller_packets = Vec::new();
    for message in payment_messages {
        assert!(
            handle_mobile_endpoint_message(
                &seller_control,
                &mut seller_fragments,
                &mut seller_packets,
                message,
            )
            .await
            .expect("handle mobile payment control")
        );
    }
    assert!(
        seller_packets.is_empty(),
        "control fragments are never tunnel data"
    );

    let seller_store_path =
        nostr_vpn_core::paid_route_store::paid_route_store_file_path(&seller_config_path);
    let seller_store = nostr_vpn_core::paid_route_store::load_paid_route_store(&seller_store_path)
        .expect("load seller paid route store");
    assert!(seller_store.channels.contains_key("mobile-channel-1"));

    let ack_message =
        receive_control_message_until_received(&seller_endpoint, &buyer_endpoint).await;
    assert!(matches!(
        decode_fips_control_frame(ack_message.data.as_slice()).expect("decode payment ack"),
        Some(FipsControlFrame::PaidRoutePaymentAck { id }) if id == payment_id
    ));
    let wrong_source = nostr_vpn_core::paid_route_store::acknowledge_paid_route_payment_outbox(
        &buyer_config_path,
        &buyer_pubkey,
        &payment_id,
    )
    .expect_err("non-seller source must not clear payment outbox");
    assert!(
        wrong_source
            .to_string()
            .contains("acknowledgment source does not match seller")
    );
    assert!(outbox_entry.exists());
    let buyer_mesh = new_mobile_mesh(FipsMeshRuntime::with_local_routes(
        buyer_mobile.peers.clone(),
        vec![buyer_mobile.local_address.clone()],
    ));
    let buyer_mesh_peers = Arc::new(RwLock::new(buyer_mobile.peers.clone()));
    let buyer_peer_identities =
        Arc::new(RwLock::new(mobile_peer_identity_map(&buyer_mobile.peers)));
    let buyer_peer_hints = Arc::new(RwLock::new(buyer_mobile.peer_hints.clone()));
    let buyer_presence = Arc::new(RwLock::new(HashMap::new()));
    let buyer_config_state = Arc::new(RwLock::new(buyer_mobile));
    let mut buyer_app = AppConfig::generated();
    buyer_app.nostr.public_key = buyer_pubkey;
    buyer_app.nostr.secret_key = buyer_nsec;
    buyer_app.ensure_defaults();
    let buyer_app_config = Arc::new(RwLock::new(buyer_app));
    let buyer_dirty = AtomicBool::new(false);
    let buyer_join = AtomicBool::new(false);
    let buyer_control = MobileEndpointReceiveContext {
        endpoint: &buyer_endpoint,
        mesh: &buyer_mesh,
        mesh_peers: &buyer_mesh_peers,
        peer_identities: &buyer_peer_identities,
        peer_hints: &buyer_peer_hints,
        presence: &buyer_presence,
        config_state: &buyer_config_state,
        app_config: &buyer_app_config,
        app_config_dirty: &buyer_dirty,
        config_path: Some(&buyer_config_path),
        network_id: &network_id,
        join_request_active: &buyer_join,
    };
    let mut buyer_fragments = FipsControlFragmentBuffer::default();
    let mut buyer_packets = Vec::new();
    assert!(
        handle_mobile_endpoint_message(
            &buyer_control,
            &mut buyer_fragments,
            &mut buyer_packets,
            ack_message,
        )
        .await
        .expect("handle mobile payment ack")
    );
    assert!(buyer_packets.is_empty(), "payment ack is never tunnel data");
    assert!(
        !outbox_entry.exists(),
        "seller-bound ack clears payment outbox"
    );

    let _ = buyer_endpoint.shutdown().await;
    let _ = seller_endpoint.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}
