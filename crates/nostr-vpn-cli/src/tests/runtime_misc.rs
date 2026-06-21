use crate::*;
#[cfg(feature = "paid-exit")]
use futures_util::{SinkExt, StreamExt};
#[cfg(feature = "embedded-fips")]
use nostr_sdk::prelude::{Keys, ToBech32};
#[cfg(feature = "embedded-fips")]
use std::collections::HashSet;
#[cfg(feature = "embedded-fips")]
use std::net::Ipv4Addr;
use std::path::Path;
#[cfg(feature = "paid-exit")]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[cfg(feature = "paid-exit")]
use tokio::net::TcpListener;
#[cfg(feature = "paid-exit")]
use tokio::sync::oneshot;
#[cfg(feature = "paid-exit")]
use tokio_tungstenite::tungstenite::Message;

#[test]
fn daemon_vpn_requires_remote_participants_to_be_active() {
    assert!(!daemon_vpn_active(true, 0));
    assert!(daemon_vpn_active(true, 1));
    assert!(!daemon_vpn_active(false, 1));
}

#[test]
fn daemon_vpn_idle_status_distinguishes_waiting_from_paused() {
    assert_eq!(
        daemon_vpn_idle_status(true, 0, false),
        crate::WAITING_FOR_PARTICIPANTS_STATUS
    );
    assert_eq!(
        daemon_vpn_idle_status(false, 0, true),
        "Listening for join requests"
    );
    assert_eq!(daemon_vpn_idle_status(false, 0, false), "Paused");
    assert_eq!(daemon_vpn_idle_status(true, 2, false), "Paused");
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_spilman_receiver_mode_names_processing_state() {
    assert_eq!(paid_exit_spilman_receiver_mode(true), "processing");
    assert_eq!(paid_exit_spilman_receiver_mode(false), "claim_only");
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_status_snapshot_reports_store_sessions_and_routing() {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, PaidRouteQualityMetrics, signed_paid_exit_offer_from_config,
    };

    let seller = Keys::generate();
    let buyer = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 100;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    offer_config.channel.free_probe_units = 0;
    offer_config.channel.grace_units = 0;
    offer_config.location.country_code = "fi".to_string();

    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let result = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 1_000,
            now_unix: 125,
        })
        .expect("open buyer session");
    let session = &mut store
        .sessions
        .get_mut(&result.session_id)
        .expect("stored session")
        .session;
    session.usage.rx_bytes = 100;
    session.realized_exit_ip = Some("198.51.100.42".to_string());
    session.observed_country_code = Some("FI".to_string());
    session.quality = Some(PaidRouteQualityMetrics {
        latency_ms: Some(42),
        jitter_ms: Some(7),
        ..PaidRouteQualityMetrics::default()
    });

    let app = AppConfig::generated();
    let snapshot = paid_exit_status_snapshot_json(&app, Path::new("/tmp/paid-routes.json"), &store);

    assert_eq!(snapshot["counts"]["sessions"].as_u64(), Some(1));
    assert_eq!(snapshot["counts"]["channels"].as_u64(), Some(1));
    assert_eq!(
        snapshot["sessions"][0]["routing"]["state"].as_str(),
        Some("paid")
    );
    assert_eq!(
        snapshot["sessions"][0]["routing"]["shared_internet"].as_str(),
        Some("on: paid")
    );
    assert_eq!(
        snapshot["sessions"][0]["routing"]["amount_due_msat"].as_u64(),
        Some(1_000)
    );
    assert_eq!(
        snapshot["sessions"][0]["realized_exit_ip"].as_str(),
        Some("198.51.100.42")
    );
    assert_eq!(
        snapshot["sessions"][0]["country_claim"]["status"].as_str(),
        Some("match")
    );
    assert_eq!(
        snapshot["sessions"][0]["quality"]["latency_ms"].as_u64(),
        Some(42)
    );
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_record_probe_once_persists_session_measurements() {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, signed_paid_exit_offer_from_config,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-record-probe-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let app = AppConfig::generated();
    app.save(&config_path).expect("save config");

    let seller = Keys::generate();
    let buyer = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 100;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let session = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 1_000,
            now_unix: 125,
        })
        .expect("open buyer session");
    write_paid_route_store(&store_path, &store).expect("write store");

    let result = paid_exit_record_probe_once(PaidExitRecordProbeArgs {
        config: Some(config_path.clone()),
        session: session.session_id.clone(),
        realized_exit_ip: Some("198.51.100.42".to_string()),
        observed_country_code: Some("fi".to_string()),
        observed_asn: Some(14_593),
        latency_ms: Some(42),
        jitter_ms: Some(7),
        packet_loss_ppm: Some(500),
        down_bps: Some(10_000_000),
        up_bps: Some(1_000_000),
        uptime_secs: Some(3600),
        last_seen_unix: Some(130),
        no_reload_daemon: true,
        json: false,
    })
    .expect("record probe");

    assert!(result.probe.changed);
    assert_eq!(
        result.probe.realized_exit_ip.as_deref(),
        Some("198.51.100.42")
    );
    assert_eq!(result.probe.observed_country_code.as_deref(), Some("FI"));
    assert_eq!(
        result
            .probe
            .quality
            .as_ref()
            .and_then(|quality| quality.down_bps),
        Some(10_000_000)
    );

    let store = load_paid_route_store(&store_path).expect("load store");
    let snapshot = paid_exit_status_snapshot_json(&app, &store_path, &store);
    assert_eq!(
        snapshot["sessions"][0]["realized_exit_ip"].as_str(),
        Some("198.51.100.42")
    );
    assert_eq!(
        snapshot["sessions"][0]["observed_country_code"].as_str(),
        Some("FI")
    );
    assert_eq!(
        snapshot["sessions"][0]["quality"]["packet_loss_ppm"].as_u64(),
        Some(500)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_probe_once_measures_and_persists_session() {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, signed_paid_exit_offer_from_config,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-probe-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let app = AppConfig::generated();
    app.save(&config_path).expect("save config");

    let seller = Keys::generate();
    let buyer = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 100;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let session = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 1_000,
            now_unix: 125,
        })
        .expect("open buyer session");
    write_paid_route_store(&store_path, &store).expect("write store");

    let (server_base, server) = spawn_paid_exit_probe_fixture_server(5);
    let result = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(paid_exit_probe_once(PaidExitProbeArgs {
            config: Some(config_path.clone()),
            session: session.session_id.clone(),
            ip_url: Some(format!("{server_base}/ip")),
            stun_servers: Vec::new(),
            no_stun: true,
            geoip_url_template: Some(format!("{server_base}/geo/{{ip}}")),
            no_geoip: false,
            download_url: Some(format!("{server_base}/download")),
            upload_url: Some(format!("{server_base}/upload")),
            bandwidth_bytes: 1_024,
            no_bandwidth: false,
            samples: 2,
            timeout_secs: 2,
            no_reload_daemon: true,
            json: false,
        }))
        .expect("probe");
    server.join().expect("server exits");

    assert!(result.probe.changed);
    assert_eq!(
        result.measurement.realized_exit_ip.as_deref(),
        Some("198.51.100.42")
    );
    assert_eq!(
        result.measurement.observed_country_code.as_deref(),
        Some("FI")
    );
    assert_eq!(result.measurement.observed_asn, Some(14_593));
    assert_eq!(result.measurement.success_count(), 2);
    assert_eq!(result.measurement.failure_count(), 0);
    assert_eq!(result.measurement.quality.packet_loss_ppm, Some(0));
    assert!(result.measurement.quality.down_bps.unwrap_or_default() > 0);
    assert!(result.measurement.quality.up_bps.unwrap_or_default() > 0);
    assert!(
        result.bandwidth_error.is_none(),
        "{:?}",
        result.bandwidth_error
    );

    let store = load_paid_route_store(&store_path).expect("load store");
    let snapshot = paid_exit_status_snapshot_json(&app, &store_path, &store);
    assert_eq!(
        snapshot["sessions"][0]["realized_exit_ip"].as_str(),
        Some("198.51.100.42")
    );
    assert_eq!(
        snapshot["sessions"][0]["observed_country_code"].as_str(),
        Some("FI")
    );
    assert_eq!(
        snapshot["sessions"][0]["observed_asn"].as_u64(),
        Some(14_593)
    );
    assert_eq!(
        snapshot["sessions"][0]["quality"]["packet_loss_ppm"].as_u64(),
        Some(0)
    );
    assert!(
        snapshot["sessions"][0]["quality"]["down_bps"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        snapshot["sessions"][0]["quality"]["up_bps"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_probe_once_uses_stun_for_realized_exit_ip() {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_vpn_core::paid_route_store::{OpenPaidRouteBuyerSessionRequest, PaidRouteStore};
    use nostr_vpn_core::paid_routes::{
        PaidExitConfig, PaidRouteMeter, signed_paid_exit_offer_from_config,
    };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-stun-probe-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let app = AppConfig::generated();
    app.save(&config_path).expect("save config");

    let seller = Keys::generate();
    let buyer = Keys::generate();
    let mut offer_config = PaidExitConfig::default();
    offer_config.enabled = true;
    offer_config.pricing.meter = PaidRouteMeter::Bytes;
    offer_config.pricing.price_msat = 1_000;
    offer_config.pricing.per_units = 100;
    offer_config.channel.accepted_mints = vec!["https://mint.example".to_string()];
    let signed_offer =
        signed_paid_exit_offer_from_config("internet-exit", &seller, &offer_config, None, 123)
            .expect("sign offer");
    let store_path = paid_route_store_file_path(&config_path);
    let mut store = PaidRouteStore::default();
    store
        .upsert_signed_offer(signed_offer, vec!["wss://relay.example".to_string()], 124)
        .expect("store offer");
    let session = store
        .open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: "internet-exit".to_string(),
            buyer_npub: buyer.public_key().to_bech32().expect("buyer npub"),
            mint_url: Some("https://mint.example".to_string()),
            channel_capacity_sat: Some(10),
            initial_paid_msat: 1_000,
            now_unix: 125,
        })
        .expect("open buyer session");
    write_paid_route_store(&store_path, &store).expect("write store");

    let (stun_server, stun_fixture) = spawn_paid_exit_stun_fixture([198, 51, 100, 77]);
    let result = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(paid_exit_probe_once(PaidExitProbeArgs {
            config: Some(config_path.clone()),
            session: session.session_id.clone(),
            ip_url: Some("http://127.0.0.1:9/unused".to_string()),
            stun_servers: vec![stun_server],
            no_stun: false,
            geoip_url_template: None,
            no_geoip: true,
            download_url: None,
            upload_url: None,
            bandwidth_bytes: 0,
            no_bandwidth: true,
            samples: 1,
            timeout_secs: 2,
            no_reload_daemon: true,
            json: false,
        }))
        .expect("probe");
    stun_fixture.join().expect("stun fixture exits");

    assert!(result.probe.changed);
    assert_eq!(
        result.measurement.realized_exit_ip.as_deref(),
        Some("198.51.100.77")
    );
    assert_eq!(result.measurement.success_count(), 1);
    assert_eq!(result.measurement.failure_count(), 0);

    let store = load_paid_route_store(&store_path).expect("load store");
    let snapshot = paid_exit_status_snapshot_json(&app, &store_path, &store);
    assert_eq!(
        snapshot["sessions"][0]["realized_exit_ip"].as_str(),
        Some("198.51.100.77")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
fn spawn_paid_exit_probe_fixture_server(
    expected_requests: usize,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture addr");
    let base = format!("http://{addr}");
    let handle = std::thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().expect("accept fixture request");
            let mut request = [0_u8; 8192];
            let read =
                std::io::Read::read(&mut stream, &mut request).expect("read fixture request");
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let body = if path.starts_with("/geo/") {
                r#"{"country_code":"fi","asn":"AS14593 Example Net"}"#.to_string()
            } else if path.starts_with("/download") {
                "x".repeat(1024)
            } else if path.starts_with("/upload") {
                r#"{"ok":true}"#.to_string()
            } else {
                r#"{"ip":"198.51.100.42"}"#.to_string()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, response.as_bytes())
                .expect("write fixture response");
        }
    });
    (base, handle)
}

#[cfg(feature = "paid-exit")]
fn spawn_paid_exit_stun_fixture(mapped_ip: [u8; 4]) -> (String, std::thread::JoinHandle<()>) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind STUN fixture");
    let addr = socket.local_addr().expect("STUN fixture addr");
    let server = format!("stun:{addr}");
    let handle = std::thread::spawn(move || {
        let mut request = [0_u8; 1500];
        let (len, peer) = socket
            .recv_from(&mut request)
            .expect("receive STUN request");
        assert!(len >= 20, "STUN request was too short");
        assert_eq!(u16::from_be_bytes([request[0], request[1]]), 0x0001);
        assert_eq!(
            u32::from_be_bytes([request[4], request[5], request[6], request[7]]),
            0x2112_A442
        );
        let transaction_id = request[8..20].to_vec();
        let response = paid_exit_stun_fixture_response(&transaction_id, mapped_ip);
        socket
            .send_to(&response, peer)
            .expect("send STUN fixture response");
    });
    (server, handle)
}

#[cfg(feature = "paid-exit")]
fn paid_exit_stun_fixture_response(transaction_id: &[u8], mapped_ip: [u8; 4]) -> Vec<u8> {
    assert_eq!(transaction_id.len(), 12);
    let mut response = Vec::new();
    response.extend_from_slice(&0x0101_u16.to_be_bytes());
    response.extend_from_slice(&12_u16.to_be_bytes());
    response.extend_from_slice(&0x2112_A442_u32.to_be_bytes());
    response.extend_from_slice(transaction_id);
    response.extend_from_slice(&0x0020_u16.to_be_bytes());
    response.extend_from_slice(&8_u16.to_be_bytes());
    response.push(0);
    response.push(0x01);
    let xor_port = 54_321_u16 ^ ((0x2112_A442_u32 >> 16) as u16);
    response.extend_from_slice(&xor_port.to_be_bytes());
    for (octet, mask) in mapped_ip.into_iter().zip(0x2112_A442_u32.to_be_bytes()) {
        response.push(octet ^ mask);
    }
    response
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_run_once_enables_seller_and_stores_offer() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-run-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");

    let args = PaidExitRunArgs {
        config: Some(config_path.clone()),
        offer_id: Some("starlink-fi".to_string()),
        relays: vec!["wss://relay.example".to_string()],
        publish: false,
        no_reload_daemon: true,
        upstream: Some("host-default".to_string()),
        meter: Some("bytes".to_string()),
        price_msat: Some(250),
        per_units: Some("1 MB".to_string()),
        accepted_mints: Some("https://mint.example".to_string()),
        accepted_mint: vec!["https://other-mint.example".to_string()],
        country_code: Some("fi".to_string()),
        region: Some("uusimaa".to_string()),
        asn: Some(12_345),
        network_class: Some("satellite".to_string()),
        ipv4: Some(true),
        ipv6: Some(false),
        max_channel_capacity_sat: Some(250),
        channel_expiry_secs: Some(300),
        free_probe_units: Some("4 KB".to_string()),
        grace_units: Some("1 KB".to_string()),
        json: false,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime
        .block_on(paid_exit_run_once(args))
        .expect("run paid exit once");

    let app = load_or_default_config(&config_path).expect("load saved config");
    let store =
        load_paid_route_store(&paid_route_store_file_path(&config_path)).expect("load store");

    assert!(app.paid_exit.enabled);
    assert_eq!(app.paid_exit.pricing.price_msat, 250);
    assert_eq!(app.paid_exit.pricing.per_units, 1_000_000);
    assert_eq!(app.paid_exit.location.country_code, "FI");
    assert_eq!(
        app.paid_exit.location.network_class,
        ExitNetworkClass::Satellite
    );
    assert_eq!(
        app.paid_exit.channel.accepted_mints,
        vec![
            "https://mint.example".to_string(),
            "https://other-mint.example".to_string()
        ]
    );
    assert_eq!(result.offer.offer_id, "starlink-fi");
    assert_eq!(result.offer.pricing.price_msat, 250);
    assert_eq!(result.offer.access.private_vpn_access.as_str(), "denied");
    assert!(result.publish.is_none());
    assert!(!result.daemon_reload_attempted);
    assert_eq!(store.offers.len(), 1);
    assert_eq!(store.wallet.mints.len(), 2);
    assert_eq!(result.status["counts"]["offers"].as_u64(), Some(1));

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_run_once_rejects_incomplete_wireguard_upstream() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-wg-incomplete-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let error = match runtime.block_on(paid_exit_run_once(PaidExitRunArgs {
        config: Some(config_path.clone()),
        offer_id: Some("wg-exit".to_string()),
        relays: vec![],
        publish: false,
        no_reload_daemon: true,
        upstream: Some("wireguard_exit".to_string()),
        meter: Some("bytes".to_string()),
        price_msat: Some(500),
        per_units: Some("1 MB".to_string()),
        accepted_mints: Some("https://mint.example".to_string()),
        accepted_mint: vec![],
        country_code: Some("fi".to_string()),
        region: None,
        asn: None,
        network_class: Some("residential".to_string()),
        ipv4: Some(true),
        ipv6: Some(false),
        max_channel_capacity_sat: Some(100),
        channel_expiry_secs: Some(600),
        free_probe_units: Some("64 KB".to_string()),
        grace_units: Some("16 KB".to_string()),
        json: false,
    })) {
        Ok(_) => panic!("incomplete WireGuard upstream should not be advertised"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("wireguard_exit is incomplete"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_run_once_enables_configured_wireguard_upstream() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-paid-exit-wg-run-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");
    let mut app = AppConfig::generated();
    app.wireguard_exit.enabled = false;
    app.wireguard_exit.address = "10.200.0.2/32".to_string();
    app.wireguard_exit.private_key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string();
    app.wireguard_exit.peer_public_key = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=".to_string();
    app.wireguard_exit.peer_preshared_key =
        "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=".to_string();
    app.wireguard_exit.endpoint = "198.51.100.20:51820".to_string();
    app.wireguard_exit.allowed_ips = vec!["0.0.0.0/0".to_string()];
    app.save(&config_path).expect("save configured upstream");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime
        .block_on(paid_exit_run_once(PaidExitRunArgs {
            config: Some(config_path.clone()),
            offer_id: Some("wg-exit".to_string()),
            relays: vec![],
            publish: false,
            no_reload_daemon: true,
            upstream: Some("wg".to_string()),
            meter: Some("bytes".to_string()),
            price_msat: Some(500),
            per_units: Some("1 MB".to_string()),
            accepted_mints: Some("https://mint.example".to_string()),
            accepted_mint: vec![],
            country_code: Some("fi".to_string()),
            region: None,
            asn: None,
            network_class: Some("residential".to_string()),
            ipv4: Some(true),
            ipv6: Some(false),
            max_channel_capacity_sat: Some(100),
            channel_expiry_secs: Some(600),
            free_probe_units: Some("64 KB".to_string()),
            grace_units: Some("16 KB".to_string()),
            json: false,
        }))
        .expect("run paid exit with configured WireGuard upstream");

    let saved = load_or_default_config(&config_path).expect("load saved config");
    assert_eq!(
        saved.paid_exit.access.upstream,
        PaidExitUpstream::WireGuardExit
    );
    assert!(saved.wireguard_exit.enabled);
    assert_eq!(
        result.offer.access.upstream,
        PaidExitUpstream::WireGuardExit
    );
    assert_eq!(
        result.status["config"]["upstream"].as_str(),
        Some("wireguard_exit")
    );
    assert_eq!(
        result.status["config"]["private_vpn_access"].as_str(),
        Some("denied")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "paid-exit")]
#[test]
fn paid_exit_advertisable_rejects_disabled_wireguard_upstream() {
    let mut app = AppConfig::generated();
    app.paid_exit.enabled = true;
    app.paid_exit.access.upstream = PaidExitUpstream::WireGuardExit;
    app.wireguard_exit.enabled = false;
    app.wireguard_exit.address = "10.200.0.2/32".to_string();
    app.wireguard_exit.private_key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string();
    app.wireguard_exit.peer_public_key = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=".to_string();
    app.wireguard_exit.endpoint = "198.51.100.20:51820".to_string();

    let error = ensure_paid_exit_advertisable(&app)
        .expect_err("disabled WireGuard upstream should not be advertised");

    assert!(error.to_string().contains("wireguard_exit is disabled"));
}

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

#[test]
fn fips_private_runtime_active_tolerates_no_active_network() {
    let mut app = AppConfig::generated();
    app.fips_host_tunnel_enabled = false;
    for network in &mut app.networks {
        network.listen_for_join_requests = false;
    }

    assert!(app.active_network_opt().is_none());
    assert!(!fips_private_runtime_active(&app, true, 0));

    app.networks[0].listen_for_join_requests = true;
    assert!(fips_private_runtime_active(&app, false, 0));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_roster_publish_attempts_disconnected_recipients() {
    let recipients = vec!["alice".to_string(), "bob".to_string()];

    let (ready, pending) = split_ready_fips_roster_recipients(recipients.clone());

    assert_eq!(ready, recipients);
    assert!(pending.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_share_public_configured_endpoint_with_roster() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "89.27.103.157:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);
    let addrs = hints.into_iter().map(|hint| hint.addr).collect::<Vec<_>>();

    assert_eq!(
        addrs,
        vec![
            "192.168.50.10:51820".to_string(),
            "89.27.103.157:51820".to_string(),
        ]
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_lan_when_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_keep_configured_lan_when_lan_discovery_disabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "192.168.50.22:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(192, 168, 50, 10)]);

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "192.168.50.22:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_cgnat_candidates() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, vec![Ipv4Addr::new(100, 120, 94, 10)]);

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_loopback_when_lan_enabled() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "127.0.0.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, Vec::new());

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_do_not_share_tunnel_endpoint() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "10.44.1.1:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = true;

    let hints = local_fips_endpoint_hints(&app, Vec::new());

    assert!(hints.is_empty());
}

#[cfg(feature = "embedded-fips")]
#[test]
fn local_fips_endpoint_hints_keep_dns_endpoint_and_listen_port() {
    let mut app = AppConfig::generated();
    app.node.endpoint = "peer.example.com:1111".to_string();
    app.node.listen_port = 51820;
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.lan_discovery_enabled = false;

    let hints = local_fips_endpoint_hints(&app, Vec::new());

    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].addr, "peer.example.com:51820");
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_keep_local_non_tunnel_addresses() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(192, 168, 50, 10)), "10.44.1.1/32");

    assert!(candidates.contains(&Ipv4Addr::new(192, 168, 50, 10)));
    assert!(!candidates.contains(&Ipv4Addr::new(10, 44, 1, 1)));
    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn runtime_signal_ipv4_candidates_drop_detected_cgnat_address() {
    let candidates =
        runtime_signal_ipv4_candidates(Some(Ipv4Addr::new(100, 120, 94, 10)), "10.44.1.1/32");

    assert!(!candidates.contains(&Ipv4Addr::new(100, 120, 94, 10)));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn endpoint_hint_recipients_are_active_participants_only() {
    let own = Keys::generate();
    let peer = Keys::generate();
    let admin = Keys::generate();
    let own_pubkey = own.public_key().to_hex();
    let peer_pubkey = peer.public_key().to_hex();
    let admin_pubkey = admin.public_key().to_hex();
    let mut app = AppConfig::generated();
    let network_id = app.networks[0].id.clone();
    app.set_network_enabled(&network_id, true)
        .expect("activate first network");
    app.nostr.secret_key = own.secret_key().to_bech32().expect("own nsec");
    app.nostr.public_key = own_pubkey.clone();
    app.networks[0].devices = vec![own_pubkey.clone(), peer_pubkey.clone()];
    app.networks[0].admins = vec![admin_pubkey.clone()];

    let recipients = desired_fips_endpoint_hint_recipients(&app);

    assert_eq!(recipients, HashSet::from([peer_pubkey]));
    assert!(!recipients.contains(&own_pubkey));
    assert!(!recipients.contains(&admin_pubkey));
}

#[cfg(all(feature = "embedded-fips", feature = "paid-exit"))]
#[test]
fn fips_tunnel_config_carries_paid_route_payment_streaming_inputs() {
    let own = Keys::generate();
    let own_pubkey = own.public_key().to_hex();
    let mut app = AppConfig::generated();
    let network_id = app.networks[0].network_id.clone();
    app.node.tunnel_ip = "10.44.1.1/32".to_string();
    app.nostr.secret_key = own.secret_key().to_bech32().expect("own nsec");
    app.nostr.public_key = own_pubkey.clone();
    app.nostr.relays = vec![
        " wss://relay.example ".to_string(),
        "wss://disabled.example".to_string(),
    ];
    app.nostr.disabled_relays = vec!["wss://disabled.example".to_string()];
    app.paid_exit.enabled = true;
    app.paid_exit.pricing.price_msat = 123;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-fips-paid-route-streaming-{nonce}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let config_path = dir.join("config.toml");

    let config = fips_tunnel_config_from_app(
        &app,
        &config_path,
        &network_id,
        "utun-test",
        Some(&own_pubkey),
        None,
        &[],
    )
    .expect("build fips config");

    assert_eq!(
        config.paid_route_store_path,
        paid_route_store_file_path(&config_path)
    );
    assert_eq!(
        config.paid_route_wallet_data_dir,
        paid_exit_wallet_data_dir(&config_path)
    );
    assert_eq!(
        config.paid_route_payment_relays,
        vec!["wss://relay.example".to_string()]
    );
    assert_eq!(config.paid_exit.pricing.price_msat, 123);
    assert_eq!(config.identity_nsec, app.nostr.secret_key);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_nonzero_pid_rejects_zero_and_invalid_values() {
    assert_eq!(parse_nonzero_pid("4242"), Some(4242));
    assert_eq!(parse_nonzero_pid("0"), None);
    assert_eq!(parse_nonzero_pid("not-a-number"), None);
}

#[test]
fn wall_time_jump_detection_flags_sleep_resume_after_threshold() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        0,
        1_000,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS
    ));
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS - 1),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
    assert!(wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
        observed_at,
        observed_at,
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[test]
fn wall_time_jump_detection_ignores_busy_loop_delays() {
    let observed_at = Instant::now();
    assert!(!wall_time_jump_detected(
        1_000,
        1_000 + MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5,
        observed_at,
        observed_at + Duration::from_secs(MAJOR_LINK_CHANGE_TIME_JUMP_SECS + 5),
        MAJOR_LINK_CHANGE_TIME_JUMP_SECS,
    ));
}

#[test]
fn daemon_network_refresh_cadence_keeps_link_changes_low_latency() {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 15);
        const {
            assert!(DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS <= 1_000);
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    assert_eq!(DAEMON_NETWORK_REFRESH_INTERVAL_SECS, 1);
}

#[test]
fn macos_underlay_route_check_throttles_route_event_storms() {
    assert_eq!(MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS, 5);

    let start = Instant::now();
    let mut last_check_at = start;

    assert!(!macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(1),
    ));
    assert_eq!(last_check_at, start);

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        false,
        start + Duration::from_secs(5),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(5));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        true,
        false,
        start + Duration::from_secs(6),
    ));
    assert_eq!(last_check_at, start + Duration::from_secs(6));

    assert!(macos_underlay_route_check_due(
        &mut last_check_at,
        false,
        true,
        start + Duration::from_secs(7),
    ));
}

#[test]
fn macos_underlay_route_repair_defers_only_for_confirmed_captive_portal() {
    assert!(!macos_underlay_route_repair_allowed(Some(true)));
    assert!(macos_underlay_route_repair_allowed(Some(false)));
    assert!(macos_underlay_route_repair_allowed(None));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_restart_endpoint_for_major_link_changes() {
    assert_eq!(
        fips_link_event_refresh(true, false, false, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, true, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, true),
        FipsLinkEventRefresh::RestartEndpoint
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_link_events_restart_endpoint_for_endpoint_only_changes() {
    assert_eq!(
        fips_link_event_refresh(false, true, false, false),
        FipsLinkEventRefresh::RestartEndpoint
    );
    assert_eq!(
        fips_link_event_refresh(false, false, false, false),
        FipsLinkEventRefresh::None
    );
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_stale_participant_recovery_is_cooldown_gated() {
    let mut last_restart_at = None;

    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000
    ));
    assert_eq!(last_restart_at, Some(1_000));
    assert!(!fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000 + FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS - 1
    ));
    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        1_000 + FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS
    ));
    assert!(fips_stale_participant_restart_due(
        &mut last_restart_at,
        900
    ));
}

#[cfg(feature = "embedded-fips")]
fn pending_fips_peer(pubkey: &str) -> MeshPeerStatus {
    MeshPeerStatus {
        pubkey: pubkey.to_string(),
        connected: false,
        endpoint_npub: format!("npub1{pubkey}"),
        transport_addr: None,
        transport_type: None,
        srtt_ms: None,
        srtt_age_ms: None,
        link_packets_sent: 0,
        link_packets_recv: 0,
        link_bytes_sent: 0,
        link_bytes_recv: 0,
        rekey_in_progress: false,
        rekey_draining: false,
        current_k_bit: None,
        last_outbound_route: None,
        direct_probe_pending: true,
        direct_probe_after_ms: Some(1_234),
        direct_probe_retry_count: 4,
        direct_probe_auto_reconnect: true,
        direct_probe_expires_at_ms: Some(5_678),
        nostr_traversal_consecutive_failures: 1,
        nostr_traversal_in_cooldown: false,
        nostr_traversal_cooldown_until_ms: None,
        nostr_traversal_last_observed_skew_ms: None,
        last_seen_at: None,
        last_control_seen_at: None,
        last_data_seen_at: None,
        tx_bytes: 1024,
        rx_bytes: 0,
        error: Some("fips link pending".to_string()),
    }
}

#[cfg(feature = "embedded-fips")]
fn connected_relay() -> DaemonRelayState {
    DaemonRelayState {
        url: "wss://relay.example".to_string(),
        status: "connected".to_string(),
    }
}

#[cfg(feature = "embedded-fips")]
fn roster_pubkeys(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_waits_for_grace_and_cooldown() {
    let peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let relays = vec![connected_relay()];
    let roster = roster_pubkeys(&["a", "b"]);
    let mut state = FipsPendingRosterRestartState::default();
    let start = 10_000;

    assert!(!fips_pending_roster_restart_due(
        &peers, &relays, &roster, 2, &mut state, start
    ));
    assert!(!fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS - 1
    ));
    assert!(fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
    assert!(!fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS + 1
    ));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_requires_connected_relay_and_all_pending() {
    let mut state = FipsPendingRosterRestartState::default();
    let disconnected_relay = DaemonRelayState {
        url: "wss://relay.example".to_string(),
        status: "disconnected".to_string(),
    };
    let peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let roster = roster_pubkeys(&["a", "b"]);

    assert!(!fips_pending_roster_restart_due(
        &peers,
        &[disconnected_relay],
        &roster,
        2,
        &mut state,
        10_000 + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));

    let mut partly_connected = peers.clone();
    partly_connected[0].connected = true;
    partly_connected[0].error = None;
    assert!(!fips_pending_roster_restart_due(
        &partly_connected,
        &[connected_relay()],
        &roster,
        2,
        &mut state,
        20_000
    ));

    let one_peer_missing_from_snapshot = vec![pending_fips_peer("a")];
    assert!(!fips_pending_roster_restart_due(
        &one_peer_missing_from_snapshot,
        &[connected_relay()],
        &roster,
        2,
        &mut state,
        30_000 + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
}

#[cfg(feature = "embedded-fips")]
#[test]
fn fips_pending_roster_recovery_ignores_connected_non_roster_transit() {
    let mut peers = vec![pending_fips_peer("a"), pending_fips_peer("b")];
    let mut transit = pending_fips_peer("transit");
    transit.connected = true;
    transit.error = None;
    transit.last_seen_at = Some(10_000);
    peers.push(transit);

    let relays = vec![connected_relay()];
    let roster = roster_pubkeys(&["a", "b"]);
    let mut state = FipsPendingRosterRestartState::default();
    let start = 40_000;

    assert!(!fips_pending_roster_restart_due(
        &peers, &relays, &roster, 2, &mut state, start
    ));
    assert!(fips_pending_roster_restart_due(
        &peers,
        &relays,
        &roster,
        2,
        &mut state,
        start + FIPS_PENDING_ROSTER_RESTART_GRACE_SECS
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn runtime_exit_node_routes_do_not_advertise_ipv6_default() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    assert_eq!(runtime_exit_node_default_routes(), vec!["0.0.0.0/0"]);
    assert_eq!(runtime_effective_advertised_routes(&app), vec!["0.0.0.0/0"]);
}

#[test]
fn legacy_macos_exit_cleanup_leaves_global_ipv4_forwarding_alone() {
    let mut app = AppConfig::generated();
    app.node.advertise_exit_node = true;

    let plan = legacy_macos_exit_cleanup_plan(&runtime_effective_advertised_routes(&app));

    assert!(plan.cleanup_pf_nat);
    assert!(!plan.restore_ipv4_forwarding);
}

#[test]
fn macos_exit_node_pf_rules_are_scoped_to_tunnel_source_and_outbound_iface() {
    let rules = crate::macos_network::macos_exit_node_pf_rules("utun42", "en0", "10.44.0.0/16");

    assert_eq!(
        rules,
        concat!(
            "nat on en0 inet from 10.44.0.0/16 to any -> (en0)\n",
            "pass in quick on utun42 inet from 10.44.0.0/16 to any keep state\n",
            "pass out quick on en0 inet from 10.44.0.0/16 to any keep state\n",
        )
    );
    assert!(!rules.contains("net.inet.ip.forwarding"));
    assert!(!rules.contains("pass in quick on en0"));
}

#[test]
fn macos_exit_node_cleanup_flushes_only_nvpn_anchor() {
    assert_eq!(
        crate::macos_network::macos_pf_anchor_flush_args(),
        vec!["-a", "com.apple/to.nostrvpn/exit", "-F", "all"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_underlay_repair_resets_tunnel_runtime() {
    let mut runtime = CliTunnelRuntime::new("utun4");
    runtime.active_listen_port = Some(51820);

    crate::session_runtime::reset_tunnel_runtime_after_macos_underlay_repair(&mut runtime);

    assert!(runtime.active_listen_port.is_none());
}

#[test]
fn macos_connect_privilege_preflight_requires_admin_when_euid_is_not_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(501));

    let error = crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect_err("non-root macOS preflight should fail");
    let message = error.to_string();
    assert!(message.contains("admin privileges"));
    assert!(message.contains("did you run with sudo?"));
    assert!(message.contains("sudo nvpn start --connect"));
    assert!(message.contains("sudo nvpn service install"));

    crate::set_macos_euid_override_for_test(None);
}

#[test]
fn macos_connect_privilege_preflight_allows_root() {
    let _guard = crate::macos_euid_override_lock_for_test()
        .lock()
        .expect("macos euid test lock");
    crate::set_macos_euid_override_for_test(Some(0));

    crate::ensure_macos_connect_privileges(Path::new("/tmp/nvpn.toml"))
        .expect("root macOS preflight should pass");

    crate::set_macos_euid_override_for_test(None);
}
