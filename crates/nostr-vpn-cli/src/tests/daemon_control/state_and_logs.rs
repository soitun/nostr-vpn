fn activate_first_network(config: &mut AppConfig) {
    let network_id = config.networks[0].id.clone();
    config
        .set_network_enabled(&network_id, true)
        .expect("activate first network");
}

#[test]
fn daemon_runtime_state_requires_advertised_routes() {
    let raw = r#"{
  "updated_at": 1773650797,
  "vpn_enabled": true,
  "vpn_active": true,
  "vpn_status": "Connected",
  "expected_peer_count": 1,
  "connected_peer_count": 1,
  "mesh_ready": true,
  "peers": [
{
  "participant_pubkey": "ed91c2fcdf6857157e72497d67be9dad91d865db6407bb0440ca53129e10cb1f",
  "node_id": "6bea57f5-e06b-49d1-83b5-484ab0a3df12",
  "tunnel_ip": "10.44.0.239/32",
  "endpoint": "192.168.178.80:51820",
  "public_key": "+fi3YvMFH0JQFNuQPiPy5xBXNKvpaCKIbbbgrlXT5yw=",
  "last_mesh_seen_at": 1773650779,
  "last_fips_seen_at": 1773650779,
  "reachable": true,
  "last_handshake_at": null,
  "error": null
}
  ]
}"#;

    assert!(serde_json::from_str::<DaemonRuntimeState>(raw).is_err());
}

#[test]
fn read_daemon_state_trims_nul_padding() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-state-trim-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let state_path = dir.join("daemon.state.json");
    let mut raw = br#"{
  "updated_at": 1,
  "binary_version": "test",
  "vpn_enabled": true,
  "vpn_active": true,
  "vpn_status": "Ready",
  "expected_peer_count": 0,
  "connected_peer_count": 0,
  "mesh_ready": false
}"#
    .to_vec();
    raw.extend_from_slice(&[0, 0, b'\n']);
    fs::write(&state_path, raw).expect("write padded daemon state");

    let state = read_daemon_state(&state_path)
        .expect("read daemon state")
        .expect("daemon state should exist");
    assert_eq!(state.vpn_status, "Ready");

    let rewritten = fs::read(&state_path).expect("read rewritten daemon state");
    assert!(!rewritten.contains(&0));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_json_peers_preserves_daemon_reachability() {
    let peer = DaemonPeerState {
        participant_pubkey: Keys::generate().public_key().to_hex(),
        node_id: "peer".to_string(),
        tunnel_ip: "10.44.0.2/32".to_string(),
        endpoint: "fips".to_string(),
        runtime_endpoint: Some("203.0.113.2:9000".to_string()),
        fips_endpoint_npub: String::new(),
        fips_transport_addr: "203.0.113.2:9000".to_string(),
        fips_transport_type: "udp".to_string(),
        fips_srtt_ms: Some(7),
        fips_srtt_age_ms: Some(10),
        fips_packets_sent: 1,
        fips_packets_recv: 2,
        fips_bytes_sent: 3,
        fips_bytes_recv: 4,
        fips_rekey_in_progress: false,
        fips_rekey_draining: false,
        fips_current_k_bit: None,
        fips_last_outbound_route: String::new(),
        direct_probe_pending: false,
        direct_probe_after_ms: None,
        direct_probe_retry_count: 0,
        direct_probe_auto_reconnect: false,
        direct_probe_expires_at_ms: None,
        fips_nostr_traversal_failures: 0,
        fips_nostr_traversal_in_cooldown: false,
        fips_nostr_traversal_cooldown_until_ms: None,
        fips_nostr_traversal_last_observed_skew_ms: None,
        tx_bytes: 5,
        rx_bytes: 6,
        public_key: String::new(),
        advertised_routes: Vec::new(),
        last_mesh_seen_at: 123,
        last_fips_seen_at: Some(123),
        last_fips_control_seen_at: Some(123),
        last_fips_data_seen_at: Some(123),
        reachable: true,
        last_handshake_at: Some(123),
        error: None,
    };

    let value = crate::status_json_peers(Some(std::slice::from_ref(&peer)), &[]);
    let peers = value.as_array().expect("status peers array");
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["reachable"], true);
    assert_eq!(peers[0]["fips_srtt_ms"], 7);
    assert_eq!(peers[0]["fips_transport_type"], "udp");
}

#[cfg(unix)]
#[test]
fn atomic_runtime_write_creates_private_file() {
    use std::os::unix::fs::PermissionsExt;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-runtime-mode-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("staged-config.toml");

    write_runtime_file_atomically(&path, b"secret = true\n").expect("write runtime file");
    let mode = fs::metadata(&path)
        .expect("runtime metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_request_projects_desired_vpn_state_immediately() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-project-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let state_path = daemon_state_file_path(&config_path);
    let state = DaemonRuntimeState {
        updated_at: 1,
        vpn_enabled: true,
        vpn_active: true,
        vpn_status: "VPN on".to_string(),
        expected_peer_count: 1,
        connected_peer_count: 1,
        mesh_ready: true,
        ..DaemonRuntimeState::default()
    };
    write_daemon_state(&state_path, &state).expect("write daemon state");

    write_daemon_control_request(&config_path, DaemonControlRequest::Pause)
        .expect("write pause control request");
    let paused = read_daemon_state(&state_path)
        .expect("read projected daemon state")
        .expect("daemon state should exist");
    assert!(!paused.vpn_enabled);
    assert!(paused.vpn_active);
    assert_eq!(paused.vpn_status, "Turning VPN off");

    write_daemon_control_request(&config_path, DaemonControlRequest::Resume)
        .expect("write resume control request");
    let resumed = read_daemon_state(&state_path)
        .expect("read projected daemon state")
        .expect("daemon state should exist");
    assert!(resumed.vpn_enabled);
    assert_eq!(resumed.vpn_status, "VPN on");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_control_request_persists_desired_vpn_state() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-control-persist-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    let mut config = AppConfig::generated();
    config.autoconnect = true;
    config.save(&config_path).expect("save config");

    write_daemon_control_request(&config_path, DaemonControlRequest::Pause)
        .expect("write pause control request");
    let paused = AppConfig::load(&config_path).expect("load paused config");
    assert!(!paused.autoconnect);

    write_daemon_control_request(&config_path, DaemonControlRequest::Resume)
        .expect("write resume control request");
    let resumed = AppConfig::load(&config_path).expect("load resumed config");
    assert!(resumed.autoconnect);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_start_honors_persisted_desired_vpn_state() {
    let mut config = AppConfig::generated();

    config.autoconnect = true;
    assert!(daemon_start_vpn_enabled(&config, false));
    assert!(!daemon_start_vpn_enabled(&config, true));

    config.autoconnect = false;
    assert!(!daemon_start_vpn_enabled(&config, false));
    assert!(!daemon_start_vpn_enabled(&config, true));
}

#[test]
fn daemon_state_freshness_allows_pid_namespace_status() {
    let state = DaemonRuntimeState {
        updated_at: 100,
        ..DaemonRuntimeState::default()
    };

    assert!(daemon_state_is_fresh(&state, 105, 10));
    assert!(!daemon_state_is_fresh(&state, 111, 10));
    assert!(daemon_state_is_fresh(&state, 99, 10));
    assert!(!daemon_state_is_fresh(&state, 50, 10));
    assert!(!daemon_state_is_fresh(
        &DaemonRuntimeState::default(),
        105,
        10
    ));
}

#[test]
fn daemon_log_compaction_leaves_small_log_untouched() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-log-small-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let log_path = dir.join("daemon.log");
    fs::write(&log_path, "short\n").expect("write daemon log");

    assert!(
        !compact_log_file_if_needed(&log_path, 64, 16).expect("compact daemon log"),
        "small logs should not be compacted"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read daemon log"),
        "short\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_log_compaction_keeps_line_aligned_tail() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-log-compact-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let log_path = dir.join("daemon.log");
    fs::write(&log_path, "old-0\nold-1\nold-2\nkeep-1\nkeep-2\n").expect("write daemon log");

    assert!(
        compact_log_file_if_needed(&log_path, 20, 14).expect("compact daemon log"),
        "oversized logs should be compacted"
    );
    let compacted = fs::read_to_string(&log_path).expect("read compacted daemon log");
    assert!(compacted.starts_with("[nvpn] daemon log compacted at "));
    assert!(!compacted.contains("old-0"));
    assert!(!compacted.contains("keep-1"));
    assert!(compacted.ends_with("keep-2\n"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_status_ignores_and_quarantines_corrupt_daemon_state() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-status-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, "").expect("write config placeholder");
    let state_path = dir.join("daemon.state.json");
    fs::write(&state_path, vec![0; 64]).expect("write corrupt daemon state");

    let status = crate::daemon_status(&config_path).expect("daemon status should succeed");
    assert!(status.state.is_none());
    assert!(!state_path.exists());

    let quarantined: Vec<_> = fs::read_dir(&dir)
        .expect("list temp dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("daemon.state.json.corrupt-"))
        })
        .collect();
    assert_eq!(quarantined.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn daemon_status_does_not_repair_network_state_when_daemon_is_stopped() {
    let _guard = crate::repair_saved_network_state_call_lock_for_test()
        .lock()
        .expect("repair call test lock");
    crate::reset_repair_saved_network_state_call_count_for_test();

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-status-pure-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, "").expect("write config placeholder");

    let status = crate::daemon_status(&config_path).expect("daemon status should succeed");
    assert!(!status.running);
    assert_eq!(crate::repair_saved_network_state_call_count_for_test(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persist_daemon_runtime_state_marks_vpn_on_as_active() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nvpn-daemon-state-test-{nonce}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let state_path = dir.join("daemon.state.json");

    let mut config = AppConfig::generated();
    config.networks[0].devices = vec!["11".repeat(32)];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    crate::persist_daemon_runtime_state(
        &state_path,
        &config,
        true,
        1,
        &tunnel_runtime,
        &[],
        &[crate::DaemonRelayState {
            url: "wss://relay.example".to_string(),
            status: "connected".to_string(),
        }],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    )
    .expect("persist daemon runtime state");

    let state = read_daemon_state(&state_path)
        .expect("read daemon state")
        .expect("daemon state should exist");
    assert!(state.vpn_active);
    assert_eq!(state.vpn_status, "VPN on");
    assert_eq!(state.relays.len(), 1);
    assert_eq!(state.relays[0].status, "connected");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fips_runtime_state_is_ready_without_waiting_for_every_peer() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    config.networks[0].devices = vec!["11".repeat(32), "22".repeat(32)];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");

    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        2,
        &tunnel_runtime,
        &[],
        &[],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert_eq!(state.connected_peer_count, 0);
    assert!(state.vpn_enabled);
    assert!(state.mesh_ready);
    assert_eq!(state.vpn_status, "VPN on");
}

#[test]
fn fips_runtime_state_rejects_far_future_peer_timestamps() {
    let mut config = AppConfig::generated();
    activate_first_network(&mut config);
    let peer_pubkey = Keys::generate().public_key().to_hex();
    config.networks[0].devices = vec![peer_pubkey.clone()];
    let tunnel_runtime = crate::CliTunnelRuntime::new("utun100");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_secs();

    let state = crate::build_daemon_runtime_state(
        &config,
        true,
        true,
        1,
        &tunnel_runtime,
        &[MeshPeerStatus {
            pubkey: peer_pubkey,
            connected: false,
            endpoint_npub: "npub1endpoint".to_string(),
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
            direct_probe_pending: false,
            direct_probe_after_ms: None,
            direct_probe_retry_count: 0,
            direct_probe_auto_reconnect: false,
            direct_probe_expires_at_ms: None,
            nostr_traversal_consecutive_failures: 0,
            nostr_traversal_in_cooldown: false,
            nostr_traversal_cooldown_until_ms: None,
            nostr_traversal_last_observed_skew_ms: None,
            last_seen_at: Some(now + 60),
            last_control_seen_at: Some(now + 60),
            last_data_seen_at: Some(now + 60),
            tx_bytes: 0,
            rx_bytes: 0,
            error: Some("fips link pending".to_string()),
        }],
        &[],
        &std::collections::HashMap::new(),
        "VPN on",
        &nostr_vpn_core::diagnostics::NetworkSummary::default(),
        &nostr_vpn_core::diagnostics::PortMappingStatus::default(),
    );

    assert_eq!(state.peers.len(), 1);
    assert_eq!(state.peers[0].last_mesh_seen_at, 0);
    assert_eq!(state.peers[0].last_fips_seen_at, None);
    assert_eq!(state.peers[0].last_fips_control_seen_at, None);
    assert_eq!(state.peers[0].last_fips_data_seen_at, None);
    assert_eq!(state.peers[0].last_handshake_at, None);
}
