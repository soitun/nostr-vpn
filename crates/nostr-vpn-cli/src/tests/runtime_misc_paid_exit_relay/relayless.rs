#[cfg(feature = "paid-exit")]
#[tokio::test]
async fn paid_exit_publish_queues_for_nostr_pubsub_when_no_relays_are_configured() {
    use nostr_vpn_core::config::NostrPubsubMode;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("nvpn-paid-exit-p2p-{nonce}"));
    let config_path = directory.join("config.toml");
    let mut app = AppConfig::generated();
    app.nostr.relays.clear();
    app.nostr.pubsub.mode = NostrPubsubMode::Client;
    app.paid_exit.enabled = true;
    app.paid_exit.pricing.price_msat = 100;
    app.paid_exit.pricing.per_units = 1_000_000;
    app.paid_exit.channel.accepted_mints = vec!["https://mint.example".to_string()];
    app.paid_exit.normalize();
    let signed = signed_paid_exit_offer_from_config(
        "relayless-exit",
        &app.nostr_keys().expect("app keys"),
        &app.paid_exit,
        Some(local_paid_exit_quality_hint()),
        unix_timestamp(),
    )
    .expect("signed paid exit offer");

    let output = publish_paid_exit_offer_pubsub(&app, &config_path, &signed)
        .expect("queue relayless paid exit offer");

    assert_eq!(output["nostr_pubsub_enabled"].as_bool(), Some(true));
    assert_eq!(output["nostr_pubsub_queued"].as_bool(), Some(true));
    let queued = std::fs::read_dir(
        crate::control_pubsub_runtime::control_pubsub_outbox_directory(&config_path),
    )
    .expect("read control pubsub outbox")
    .count();
    assert_eq!(queued, 1);

    let _ = std::fs::remove_dir_all(directory);
}
