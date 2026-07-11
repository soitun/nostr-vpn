    #[test]
    fn native_state_reports_active_exit_node_when_selected_peer_is_reachable() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let exit_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = true;
        runtime.config.exit_node = exit_pubkey.to_string();
        runtime.config.exit_node_leak_protection = true;
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey];
        runtime.config.networks[0].devices = vec![exit_pubkey.to_string()];
        runtime
            .config
            .set_peer_alias(exit_pubkey, "lab-exit")
            .unwrap();
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: true,
            vpn_active: true,
            expected_peer_count: 1,
            connected_peer_count: 1,
            mesh_ready: true,
            peers: vec![DaemonPeerState {
                participant_pubkey: exit_pubkey.to_string(),
                advertised_routes: vec!["0.0.0.0/0".to_string()],
                reachable: true,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();
        assert!(!state.exit_node_blocked);
        assert!(state.exit_node_active);
        assert_eq!(state.exit_node_status_text, "Exit: lab-exit.nvpn");
    }

    #[test]
    fn native_state_flags_wireguard_exit_blocking_without_advertising_exit() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = false;
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey];
        runtime.config.exit_node_leak_protection = true;
        runtime.config.node.advertise_exit_node = false;
        runtime.config.wireguard_exit.enabled = true;
        runtime.config.wireguard_exit.address = "10.64.70.195/32".to_string();
        runtime.config.wireguard_exit.private_key = "client-private".to_string();
        runtime.config.wireguard_exit.peer_public_key = "provider-public".to_string();
        runtime.config.wireguard_exit.endpoint = "vpn.example.test:51820".to_string();
        runtime.config.wireguard_exit.allowed_ips = vec!["0.0.0.0/0".to_string()];

        let state = runtime.state();
        assert!(state.wireguard_exit_enabled);
        assert!(state.wireguard_exit_configured);
        assert!(!state.advertise_exit_node);
        assert!(state.exit_node_blocked);
        assert!(!state.exit_node_active);
        assert_eq!(
            state.exit_node_status_text,
            "Internet blocked: waiting for WireGuard exit"
        );

        runtime.vpn_active = true;
        let state = runtime.state();
        assert!(!state.exit_node_blocked);
        assert!(state.exit_node_active);
        assert_eq!(state.exit_node_status_text, "Exit: WireGuard upstream");
    }

    #[test]
    fn native_state_reports_routed_fips_peer_latency() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let peer_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = true;
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey];
        runtime.config.networks[0].devices = vec![peer_pubkey.to_string()];
        let now = unix_timestamp();
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: true,
            vpn_active: true,
            expected_peer_count: 1,
            connected_peer_count: 1,
            fips_other_peer_count: 2,
            mesh_ready: true,
            peers: vec![DaemonPeerState {
                participant_pubkey: peer_pubkey.to_string(),
                endpoint: "fips".to_string(),
                runtime_endpoint: Some("fips".to_string()),
                fips_endpoint_npub: "npub1peer".to_string(),
                fips_srtt_ms: Some(112),
                fips_srtt_age_ms: Some(789),
                direct_probe_pending: true,
                direct_probe_after_ms: Some(12_345),
                direct_probe_retry_count: 3,
                direct_probe_auto_reconnect: true,
                direct_probe_expires_at_ms: Some(67_890),
                last_fips_control_seen_at: Some(now.saturating_sub(120)),
                last_fips_data_seen_at: Some(now.saturating_sub(7_200)),
                last_fips_seen_at: Some(now),
                last_handshake_at: Some(now),
                reachable: true,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();
        let peer = state.networks[0]
            .participants
            .iter()
            .find(|participant| participant.pubkey_hex == peer_pubkey)
            .expect("peer participant");

        assert_eq!(state.fips_connected_peer_count, 0);
        assert_eq!(state.fips_roster_peer_count, 1);
        assert_eq!(state.non_fips_roster_peer_count, 2);
        assert_eq!(peer.status_text, "online via mesh, probing direct (112 ms)");
        assert_eq!(peer.fips_srtt_ms, 112);
        assert_eq!(peer.fips_srtt_age_ms, 789);
        assert!(peer.fips_direct_probe_pending);
        assert_eq!(peer.fips_direct_probe_after_ms, 12_345);
        assert_eq!(peer.fips_direct_probe_retry_count, 3);
        assert!(peer.fips_direct_probe_auto_reconnect);
        assert_eq!(peer.fips_direct_probe_expires_at_ms, 67_890);
        assert_eq!(peer.last_fips_control_seen_text, "seen 2m ago");
        assert_eq!(peer.last_fips_data_seen_text, "seen 2h ago");
    }

    #[test]
    fn native_state_rejects_far_future_fips_presence() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let peer_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = true;
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey];
        runtime.config.networks[0].devices = vec![peer_pubkey.to_string()];
        let future_seen_at = unix_timestamp() + 60;
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: true,
            vpn_active: true,
            expected_peer_count: 1,
            connected_peer_count: 0,
            mesh_ready: false,
            peers: vec![DaemonPeerState {
                participant_pubkey: peer_pubkey.to_string(),
                endpoint: "fips".to_string(),
                runtime_endpoint: Some("fips".to_string()),
                fips_endpoint_npub: "npub1peer".to_string(),
                last_fips_control_seen_at: Some(future_seen_at),
                last_fips_data_seen_at: Some(future_seen_at),
                last_mesh_seen_at: future_seen_at,
                last_fips_seen_at: Some(future_seen_at),
                last_handshake_at: Some(future_seen_at),
                reachable: false,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();
        let peer = state.networks[0]
            .participants
            .iter()
            .find(|participant| participant.pubkey_hex == peer_pubkey)
            .expect("peer participant");

        assert!(!peer.reachable);
        assert_eq!(peer.state, "offline");
        assert_eq!(peer.mesh_state, "absent");
        assert_eq!(peer.status_text, "offline");
        assert_eq!(peer.last_fips_control_seen_text, "");
        assert_eq!(peer.last_fips_data_seen_text, "");
        assert_eq!(peer.last_seen_text, "");
    }

    #[test]
    fn native_state_counts_direct_fips_roster_peer() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        let peer_pubkey = "26525c442dd039de4e728b41ee8d7f717b267ab25b7c219d53a3249e1c9174cc";
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = true;
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey];
        runtime.config.networks[0].devices = vec![peer_pubkey.to_string()];
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: true,
            vpn_active: true,
            expected_peer_count: 1,
            connected_peer_count: 1,
            mesh_ready: true,
            peers: vec![DaemonPeerState {
                participant_pubkey: peer_pubkey.to_string(),
                fips_endpoint_npub: "npub1peer".to_string(),
                fips_transport_addr: "203.0.113.9:9000".to_string(),
                fips_transport_type: "udp".to_string(),
                reachable: true,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();

        assert_eq!(state.fips_connected_peer_count, 1);
        assert_eq!(state.fips_roster_peer_count, 1);
        assert_eq!(state.non_fips_roster_peer_count, 0);
    }

    #[test]
    fn invite_import_adopts_network_without_queueing_join_request() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-invite-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let admin_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("admin npub");
        let invite = serde_json::json!({
            "v": 3,
            "networkId": "8d4f34f5425bc50e",
            "inviterEndpoints": ["192.168.50.20:51820"],
            "admins": [admin_npub],
            "relays": ["wss://temp.iris.to"]
        })
        .to_string();

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        runtime
            .import_network_invite(&invite)
            .expect("import invite");

        let network = runtime.config.active_network();
        assert!(network.outbound_join_request.is_none());
        assert!(network.devices.is_empty());
        assert_eq!(
            runtime.config.fips_peer_endpoints.get(&admin_npub),
            Some(&vec!["192.168.50.20:51820".to_string()])
        );
        let state = runtime.state();
        assert_eq!(state.networks.len(), 1);
        assert_eq!(state.networks[0].network_id, "8d4f34f5425bc50e");

        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    #[allow(clippy::too_many_lines)]
    fn compact_join_bootstrap_is_added_by_admin_without_request_event() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let admin_dir =
            std::env::temp_dir().join(format!("nvpn-app-core-admin-join-request-{nonce}"));
        let joiner_dir =
            std::env::temp_dir().join(format!("nvpn-app-core-joiner-request-link-{nonce}"));
        fs::create_dir_all(&admin_dir).expect("create admin test dir");
        fs::create_dir_all(&joiner_dir).expect("create joiner test dir");

        let error = anyhow!("boom");
        let mut admin = NativeAppRuntime::from_startup_error(&error);
        admin.startup_error = None;
        admin.last_error.clear();
        admin.mobile_runtime = true;
        admin.config_path = admin_dir.join("config.toml");
        admin.config.node_name = "Admin Mac".to_string();
        let admin_pubkey = admin
            .config
            .own_nostr_pubkey_hex()
            .expect("admin pubkey");
        let admin_network_id = create_test_network(&mut admin, "Home");
        admin.config.networks[0].network_id = "8d4f34f5425bc50e".to_string();
        admin.config.networks[0].admins = vec![admin_pubkey.clone()];

        let mut joiner = NativeAppRuntime::from_startup_error(&error);
        joiner.startup_error = None;
        joiner.mobile_runtime = true;
        joiner.config_path = joiner_dir.join("config.toml");
        joiner.config.node_name = "Pixel Phone".to_string();
        joiner.config.clear_pending_nostr_join_request();
        let joiner_pubkey = joiner
            .config
            .own_nostr_pubkey_hex()
            .expect("joiner pubkey");
        joiner
            .config
            .ensure_pending_nostr_join_request(1_778_998_000)
            .expect("joiner request");
        let pending = joiner
            .config
            .pending_nostr_join_request
            .as_ref()
            .expect("pending request");
        let request_keys = pending.request_keys().expect("request keys");
        let request_secret = pending.request.request_secret.clone();
        let request_pubkey = pending.request.request_pubkey.clone();
        let bootstrap = nostr_vpn_core::identity_bridge::nostr_identity_device_approval_bootstrap(
            &pending.request,
        )
        .expect("joiner request bootstrap");
        let join_request = joiner
            .config
            .pending_nostr_join_request_link(crate::join_request_link::JOIN_REQUEST_LINK_PREFIX)
            .expect("joiner request link");
        assert!(join_request.starts_with("nvpn://join-request/"));
        let parsed_bootstrap =
            nostr_vpn_core::identity_bridge::parse_nostr_identity_device_approval_bootstrap(
                &join_request,
                &[crate::join_request_link::JOIN_REQUEST_LINK_PREFIX],
            )
            .expect("parse compact join request")
            .expect("join request bootstrap");
        assert_eq!(parsed_bootstrap, bootstrap);
        assert_eq!(
            nostr_vpn_core::config::normalize_nostr_pubkey(&parsed_bootstrap.device_app_key_npub)
                .expect("stable AppKey"),
            joiner_pubkey
        );
        assert_eq!(parsed_bootstrap.request_secret, request_secret);
        assert_ne!(
            nostr_vpn_core::config::normalize_nostr_pubkey(&parsed_bootstrap.request_npub)
                .expect("request key"),
            joiner_pubkey
        );

        admin
            .import_join_request(&join_request)
            .expect("import compact join request");

        assert!(admin.last_error.is_empty(), "{}", admin.last_error);
        assert_eq!(admin.published_join_approval_events.len(), 5);
        let canonical_admin =
            nostr_vpn_core::identity_bridge::parse_roster_app_key_sidecar_event(
                &admin.published_join_approval_events[0],
            )
            .expect("parse canonical admin genesis")
            .expect("canonical admin identity");
        assert_eq!(canonical_admin.facet.pubkey, admin_pubkey);
        assert_eq!(canonical_admin.role, nostr_vpn_core::identity_bridge::RosterAppKeyRole::Admin);
        let roster_identity =
            nostr_vpn_core::identity_bridge::parse_roster_app_key_sidecar_event(
                &admin.published_join_approval_events[1],
            )
            .expect("parse roster sidecar")
            .expect("roster sidecar identity");
        assert_eq!(roster_identity.facet.pubkey, joiner_pubkey);
        let receipt =
            nostr_identity::parse_nostr_identity_device_approval_receipt_event_for_bootstrap(
                &admin.published_join_approval_events[3],
                &request_keys,
                &parsed_bootstrap,
            )
            .expect("decrypt approval receipt");
        assert_eq!(receipt.request_pubkey, request_pubkey);
        assert_eq!(receipt.device_app_key_pubkey, joiner_pubkey);
        assert_eq!(receipt.approved_by_pubkey, admin.config.own_nostr_pubkey_hex().unwrap());
        assert_eq!(receipt.request_secret, request_secret);
        let roster_op_event_id = admin.published_join_approval_events[1].id.to_hex();
        assert_eq!(
            receipt.roster_op_id.as_deref(),
            Some(roster_op_event_id.as_str())
        );
        let receipt_roster_op =
            nostr_vpn_core::identity_bridge::parse_nostr_identity_device_approval_receipt_roster_op(
                &receipt,
            )
            .expect("receipt embeds roster op");
        assert_eq!(receipt_roster_op.op_id, roster_op_event_id);
        assert!(
            !admin.published_join_approval_events[3]
                .content
                .contains(&request_secret)
        );
        let vpn_context =
            nostr_vpn_core::identity_bridge::parse_nostr_vpn_join_approval_context_event(
                &admin.published_join_approval_events[4],
                &request_keys,
            )
            .expect("decrypt Nostr VPN approval context");
        assert_eq!(vpn_context.request_pubkey, request_pubkey);
        assert_eq!(vpn_context.device_app_key_pubkey, joiner_pubkey);
        assert_eq!(
            vpn_context.approved_by_pubkey,
            admin.config.own_nostr_pubkey_hex().unwrap()
        );
        assert_eq!(vpn_context.request_secret, request_secret);
        assert_eq!(vpn_context.mesh_network_id, "8d4f34f5425bc50e");
        assert_eq!(vpn_context.network_name.as_deref(), Some("Home"));
        assert_eq!(vpn_context.roster_op_id.as_deref(), Some(roster_op_event_id.as_str()));
        assert!(
            !admin.published_join_approval_events[4]
                .content
                .contains(&request_secret)
        );
        assert!(
            !admin.published_join_approval_events[4]
                .content
                .contains("8d4f34f5425bc50e")
        );
        assert!(admin.config.networks[0].devices.contains(&joiner_pubkey));
        assert!(admin.config.networks[0].inbound_join_requests.is_empty());
        assert_eq!(
            admin.config.peer_alias(&joiner_pubkey).as_deref(),
            Some("pixel-phone")
        );
        let imported = admin
            .state()
            .networks
            .into_iter()
            .find(|network| network.id == admin_network_id)
            .expect("admin network");
        assert!(imported.inbound_join_requests.is_empty());
        assert!(imported.join_request_qr_code_or_link.is_empty());
        assert!(admin.state().join_request_qr_code_or_link.is_empty());

        assert!(
            joiner
                .apply_fetched_join_approval_events(&admin.published_join_approval_events)
                .expect("apply fetched approval events"),
            "joiner should apply the accepted network"
        );
        assert!(joiner.config.pending_nostr_join_request.is_none());
        assert_eq!(joiner.config.active_network().network_id, "8d4f34f5425bc50e");
        assert!(joiner.config.active_network_has_confirmed_local_identity());
        let persisted_joiner = AppConfig::load(&joiner.config_path).expect("reload joined iPhone");
        assert!(persisted_joiner.pending_nostr_join_request.is_none());
        assert_eq!(
            persisted_joiner.active_network().network_id,
            "8d4f34f5425bc50e"
        );

        let _ = fs::remove_dir_all(&admin_dir);
        let _ = fs::remove_dir_all(&joiner_dir);
    }

    #[test]
    fn unsupported_compact_join_request_is_rejected_without_adding_device() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-compact-join-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let error = anyhow!("boom");
        let mut admin = NativeAppRuntime::from_startup_error(&error);
        admin.startup_error = None;
        admin.mobile_runtime = true;
        admin.config_path = dir.join("config.toml");
        let admin_pubkey = admin
            .config
            .own_nostr_pubkey_hex()
            .expect("admin pubkey");
        create_test_network(&mut admin, "Home");
        admin.config.networks[0].admins = vec![admin_pubkey];

        let joiner_pubkey = Keys::generate().public_key().to_hex();
        let compact = format!("nvpn://join-request?app_key={joiner_pubkey}");

        admin.dispatch(NativeAppAction::ImportJoinRequest { request: compact });

        assert!(
            admin.last_error.contains("unsupported join request link"),
            "{}",
            admin.last_error
        );
        assert!(!admin.config.networks[0].devices.contains(&joiner_pubkey));
        assert!(admin.published_join_approval_events.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invite_import_reuses_inactive_default_network_placeholder() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-placeholder-invite-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let admin_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("admin npub");
        let invite = serde_json::json!({
            "v": 3,
            "networkName": "Network 1",
            "networkId": "8d4f34f5425bc50e",
            "admins": [admin_npub]
        })
        .to_string();

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        runtime.config = AppConfig::generated();

        runtime
            .import_network_invite(&invite)
            .expect("import invite");

        assert_eq!(runtime.config.networks.len(), 1);
        let network = runtime.config.active_network();
        assert_eq!(network.id, "network-1");
        assert_eq!(network.name, "Network 1");
        assert_eq!(network.network_id, "8d4f34f5425bc50e");
        assert!(network.outbound_join_request.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invite_import_creates_new_network_when_active_network_is_named() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-named-active-invite-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let admin_npub = "npub1akgu9lxldpt32lnjf97k005a4kgasewmvsrmkpzqeff398ssev0ssd6t3u";
        let admin_hex = normalize_nostr_pubkey(admin_npub).expect("normalize admin");
        let invite = "nvpn://invite/eyJ2IjozLCJuZXR3b3JrSWQiOiI3YTYwMTQ4MzVkNDA0Y2IwIiwiYWRtaW5zIjpbIm5wdWIxYWtndTlseGxkcHQzMmxuamY5N2swMDVhNGtnYXNld212c3Jta3B6cWVmZjM5OHNzZXYwc3NkNnQzdSJdfQ";

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        let old_network_id = create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].network_id = "5a249444c4254f98".to_string();
        runtime.config.networks[0].admins = vec![admin_hex.clone()];

        runtime
            .import_network_invite(invite)
            .expect("import invite");

        assert_eq!(runtime.config.networks.len(), 2);
        let old_network = runtime
            .config
            .network_by_id(&old_network_id)
            .expect("old network should remain");
        assert_eq!(old_network.network_id, "5a249444c4254f98");
        assert!(!old_network.enabled);

        let network = runtime.config.active_network();
        assert_eq!(network.network_id, "7a6014835d404cb0");
        assert_eq!(network.admins, vec![admin_hex.clone()]);
        assert_eq!(network.invite_inviter, admin_hex);
        assert!(network.outbound_join_request.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_state_marks_reachable_invite_admin_as_pending_until_join_is_accepted() {
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.daemon_running = true;
        runtime.vpn_enabled = true;
        runtime.vpn_active = true;
        create_test_network(&mut runtime, "Home");

        let admin_hex = Keys::generate().public_key().to_hex();
        runtime.config.networks[0].network_id = "mesh-home".to_string();
        runtime.config.networks[0].devices = Vec::new();
        runtime.config.networks[0].admins = vec![admin_hex.clone()];
        runtime.config.networks[0].invite_inviter = admin_hex.clone();
        runtime.config.networks[0].outbound_join_request = Some(PendingOutboundJoinRequest {
            recipient: admin_hex.clone(),
            requested_at: 1_726_000_000,
        });
        runtime.daemon_state = Some(DaemonRuntimeState {
            vpn_enabled: true,
            vpn_active: true,
            expected_peer_count: 1,
            connected_peer_count: 1,
            mesh_ready: true,
            peers: vec![DaemonPeerState {
                participant_pubkey: admin_hex.clone(),
                tunnel_ip: "10.44.135.191".to_string(),
                reachable: true,
                ..DaemonPeerState::default()
            }],
            ..DaemonRuntimeState::default()
        });

        let state = runtime.state();
        let admin = state.networks[0]
            .participants
            .iter()
            .find(|participant| participant.pubkey_hex == admin_hex)
            .expect("admin participant should be visible");

        assert!(admin.reachable);
        assert_eq!(admin.state, "pending");
        assert_eq!(admin.status_text, "join request sent");
    }

    #[test]
    fn manual_add_network_seeds_admin_without_join_request() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-manual-add-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let admin_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("admin npub");
        let admin_hex = normalize_nostr_pubkey(&admin_npub).expect("normalize admin");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        runtime.dispatch(NativeAppAction::ManualAddNetwork {
            admin_npub,
            mesh_network_id: "8d4f34f5425bc50e".to_string(),
        });

        let network = runtime.config.active_network();
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert_eq!(network.network_id, "8d4f34f5425bc50e");
        assert_eq!(network.devices, vec![admin_hex.clone()]);
        assert_eq!(network.admins, vec![admin_hex]);
        assert!(network.outbound_join_request.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lan_pairing_runs_for_fifteen_minutes_until_cancelled() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-lan-pairing-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        runtime.dispatch(NativeAppAction::StartInviteBroadcast);
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        runtime.dispatch(NativeAppAction::StartNearbyDiscovery);
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);

        let state = runtime.state();
        assert!(state.invite_broadcast_active);
        assert!(state.nearby_discovery_active);
        assert!(state.invite_broadcast_remaining_secs <= LAN_PAIRING_DURATION.as_secs());
        assert!(state.invite_broadcast_remaining_secs > LAN_PAIRING_DURATION.as_secs() - 10);
        assert!(state.nearby_discovery_remaining_secs <= LAN_PAIRING_DURATION.as_secs());
        assert!(state.nearby_discovery_remaining_secs > LAN_PAIRING_DURATION.as_secs() - 10);

        runtime.dispatch(NativeAppAction::StopInviteBroadcast);
        let state = runtime.state();
        assert!(!state.invite_broadcast_active);
        assert_eq!(state.invite_broadcast_remaining_secs, 0);
        assert!(
            state.nearby_discovery_active,
            "discovery should keep running"
        );

        runtime.dispatch(NativeAppAction::StopNearbyDiscovery);
        let state = runtime.state();
        assert!(!state.nearby_discovery_active);
        assert_eq!(state.nearby_discovery_remaining_secs, 0);
        assert!(state.lan_peers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn joined_device_does_not_advertise_nearby_join_request() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-broadcast-joins-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        let network_id = create_test_network(&mut runtime, "Home");

        runtime.dispatch(NativeAppAction::StartInviteBroadcast);

        assert!(
            runtime
                .last_error
                .contains("nearby join request advertising is only available"),
            "{}",
            runtime.last_error
        );
        assert!(!runtime.state().invite_broadcast_active);
        assert_eq!(runtime.config.networks[0].id, network_id);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn enabling_join_requests_starts_background_fips_listener() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-join-listener-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");
        let calls_path = dir.join("calls.txt");
        let started_path = dir.join("started");
        let script_path = dir.join("nvpn");
        let calls_literal = calls_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let started_literal = started_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
CALLS="{calls_literal}"
STARTED="{started_literal}"
printf '%s\n' "$*" >> "$CALLS"
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{{"supported":true,"installed":true,"disabled":false,"loaded":true,"running":true,"pid":123,"label":"fi.siriusbusiness.nvpn.test","binary_version":"test"}}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  if [ -f "$STARTED" ]; then
    cat <<'JSON'
{{"daemon":{{"running":true,"state":{{"updated_at":1,"binary_version":"test","local_endpoint":"","advertised_endpoint":"","listen_port":0,"vpn_enabled":true,"vpn_active":false,"vpn_status":"Listening for join requests","expected_peer_count":0,"connected_peer_count":0,"mesh_ready":false,"peers":[]}}}}}}
JSON
  else
    cat <<'JSON'
{{"daemon":{{"running":false,"state":null}}}}
JSON
  fi
  exit 0
fi
if [ "$1" = "start" ]; then
  touch "$STARTED"
  exit 0
fi
if [ "$1" = "resume" ] || [ "$1" = "reload" ]; then
  exit 0
fi
exit 0
"#
        );
        fs::write(&script_path, script).expect("write fake nvpn");
        let mut permissions = fs::metadata(&script_path)
            .expect("fake nvpn metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make fake nvpn executable");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.last_error.clear();
        runtime.mobile_runtime = false;
        runtime.config_path = dir.join("config.toml");
        let network_id = create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].listen_for_join_requests = false;
        runtime
            .config
            .save(&runtime.config_path)
            .expect("save test config");
        runtime.nvpn_bin = Some(script_path);

        runtime.dispatch(NativeAppAction::SetNetworkJoinRequestsEnabled {
            network_id,
            enabled: true,
        });

        let calls = fs::read_to_string(&calls_path).expect("read fake nvpn calls");
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert!(
            started_path.exists(),
            "join listener daemon was not started"
        );
        assert!(calls.contains("start --daemon --connect --config"));
        assert!(runtime.config.networks[0].listen_for_join_requests);
        assert!(runtime.vpn_enabled);
        assert_eq!(runtime.vpn_status, "Listening for join requests");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_state_exposes_inbound_join_requests_for_ui_shells() {
        let requester_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("requester npub");
        let requester_hex = normalize_nostr_pubkey(&requester_npub).expect("normalize requester");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0]
            .inbound_join_requests
            .push(PendingInboundJoinRequest {
                requester: requester_hex.clone(),
                requester_node_name: "iPhone".to_string(),
                requested_at: 1_778_998_000,
            });

        let state = runtime.state();
        let request = state.networks[0]
            .inbound_join_requests
            .first()
            .expect("join request should be visible in native state");

        assert_eq!(request.requester_npub, requester_npub);
        assert_eq!(request.requester_pubkey_hex, requester_hex);
        assert_eq!(request.requester_node_name, "iPhone");
        assert!(!request.requested_at_text.trim().is_empty());

        let json = serde_json::to_value(&state).expect("serialize native state");
        assert_eq!(
            json["networks"][0]["inboundJoinRequests"][0]["requesterNpub"],
            requester_npub
        );
        assert_eq!(
            json["networks"][0]["inboundJoinRequests"][0]["requesterNodeName"],
            "iPhone"
        );
    }

    #[test]
    fn desktop_tick_reloads_roster_edits_from_disk() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-desktop-reload-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");
        let script_path = dir.join("nvpn");

        let script = r#"#!/bin/sh
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{"supported":true,"installed":true,"disabled":false,"loaded":true,"running":true,"pid":123,"label":"fi.siriusbusiness.nvpn.test","binary_version":"test"}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  cat <<'JSON'
{"daemon":{"running":true,"state":{"updated_at":1,"binary_version":"test","local_endpoint":"","advertised_endpoint":"","listen_port":0,"vpn_enabled":true,"vpn_active":true,"vpn_status":"VPN on","expected_peer_count":1,"connected_peer_count":1,"mesh_ready":true,"peers":[]}}}
JSON
  exit 0
fi
exit 0
"#;
        fs::write(&script_path, script).expect("write fake nvpn");
        let mut permissions = fs::metadata(&script_path)
            .expect("fake nvpn metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make fake nvpn executable");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.last_error.clear();
        runtime.mobile_runtime = false;
        runtime.config_path = dir.join("config.toml");
        runtime.nvpn_bin = Some(script_path);
        let own_pubkey = runtime
            .config
            .own_nostr_pubkey_hex()
            .expect("generated config should have own pubkey");
        create_test_network(&mut runtime, "Home");
        runtime.config.networks[0].admins = vec![own_pubkey.clone()];
        runtime
            .config
            .save(&runtime.config_path)
            .expect("save initial config");
        assert_eq!(runtime.state().networks[0].participants.len(), 1);

        let peer_pubkey = Keys::generate().public_key().to_hex();
        let mut persisted = runtime.config.clone();
        persisted.networks[0].devices = vec![peer_pubkey.clone()];
        persisted
            .save(&runtime.config_path)
            .expect("save external roster edit");

        runtime.dispatch(NativeAppAction::Tick);
        let state = runtime.state();
        let network = &state.networks[0];

        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert_eq!(network.participants.len(), 2);
        assert!(network
            .participants
            .iter()
            .any(|participant| participant.pubkey_hex == peer_pubkey));
        assert_eq!(network.expected_count, 2);

        let _ = fs::remove_dir_all(&dir);
    }
