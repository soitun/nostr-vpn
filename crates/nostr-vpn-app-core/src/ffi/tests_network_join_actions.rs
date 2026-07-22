    #[test]
    fn accepting_join_request_uses_requester_node_name_as_alias() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-accept-join-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let requester_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("requester npub");
        let requester_hex = normalize_nostr_pubkey(&requester_npub).expect("normalize requester");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        runtime.dispatch(NativeAppAction::AddNetwork {
            name: "Home".to_string(),
        });
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        let network_id = runtime.config.networks[0].id.clone();
        runtime.config.networks[0]
            .inbound_join_requests
            .push(PendingInboundJoinRequest {
                requester: requester_hex.clone(),
                requester_node_name: "Linux Dev".to_string(),
                requested_at: 1_726_000_000,
            });

        runtime.dispatch(NativeAppAction::AcceptJoinRequest {
            network_id: network_id.clone(),
            requester_npub,
        });

        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert!(
            runtime.config.networks[0].devices
                .contains(&requester_hex)
        );
        assert!(runtime.config.networks[0].inbound_join_requests.is_empty());
        assert_eq!(
            runtime.config.peer_alias(&requester_hex).as_deref(),
            Some("linux-dev")
        );

        let saved = AppConfig::load(&runtime.config_path).expect("load persisted config");
        assert_eq!(
            saved.peer_alias(&requester_hex).as_deref(),
            Some("linux-dev")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adding_a_new_participant_queues_one_receipt_backed_manual_join_roster() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-manual-outbox-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let joiner_keys = Keys::generate();
        let joiner_npub = joiner_keys
            .public_key()
            .to_bech32()
            .expect("joiner npub");
        let joiner_hex = joiner_keys.public_key().to_hex();
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        runtime.dispatch(NativeAppAction::AddNetwork {
            name: "Home".to_string(),
        });
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        let network_entry_id = runtime.config.networks[0].id.clone();
        let mesh_network_id = runtime.config.networks[0].network_id.clone();
        let admin = runtime.config.own_nostr_pubkey_hex().expect("admin pubkey");

        let action = NativeAppAction::AddParticipant {
            network_id: network_entry_id,
            npub: joiner_npub,
            alias: Some("iPhone".to_string()),
        };
        runtime.dispatch(action.clone());
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert_eq!(runtime.queued_join_rosters.len(), 1);
        assert_eq!(
            runtime.queued_join_rosters[0]
                .signed_roster
                .signer_pubkey_hex()
                .expect("manual roster signer"),
            admin
        );
        assert!(
            runtime.queued_join_rosters[0]
                .signed_roster
                .roster()
                .expect("manual roster")
                .devices
                .contains(&joiner_hex)
        );

        runtime.dispatch(action);
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert_eq!(
            runtime.queued_join_rosters.len(),
            1,
            "re-adding an existing participant must not create another delivery"
        );
        assert_eq!(runtime.config.networks[0].network_id, mesh_network_id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepting_join_request_requires_pending_request() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-accept-missing-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let requester_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("requester npub");
        let requester_hex = normalize_nostr_pubkey(&requester_npub).expect("normalize requester");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        create_test_network(&mut runtime, "Home");
        let network_id = runtime.config.networks[0].id.clone();

        runtime.dispatch(NativeAppAction::AcceptJoinRequest {
            network_id,
            requester_npub,
        });

        assert!(
            runtime.last_error.contains("no pending join request"),
            "{}",
            runtime.last_error
        );
        assert!(
            !runtime.config.networks[0].devices
                .contains(&requester_hex)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejecting_join_request_removes_it_without_adding_participant() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nvpn-app-core-reject-join-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");

        let requester_npub = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("requester npub");
        let requester_hex = normalize_nostr_pubkey(&requester_npub).expect("normalize requester");

        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");
        create_test_network(&mut runtime, "Home");
        let network_id = runtime.config.networks[0].id.clone();
        runtime.config.networks[0]
            .inbound_join_requests
            .push(PendingInboundJoinRequest {
                requester: requester_hex.clone(),
                requester_node_name: "Ubuntu Dev".to_string(),
                requested_at: 1_726_000_000,
            });

        runtime.dispatch(NativeAppAction::RejectJoinRequest {
            network_id,
            requester_npub,
        });

        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        assert!(
            !runtime.config.networks[0].devices
                .contains(&requester_hex)
        );
        assert!(runtime.config.networks[0].inbound_join_requests.is_empty());

        let saved = AppConfig::load(&runtime.config_path).expect("load persisted config");
        assert!(saved.networks[0].inbound_join_requests.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
