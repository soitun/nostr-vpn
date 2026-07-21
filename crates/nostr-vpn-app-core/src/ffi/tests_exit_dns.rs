    #[test]
    fn settings_patch_validates_exit_dns_atomically_and_exposes_saved_policy() {
        let dir = unique_service_test_dir("nvpn-app-core-exit-dns-settings");
        let error = anyhow!("boom");
        let mut runtime = NativeAppRuntime::from_startup_error(&error);
        runtime.startup_error = None;
        runtime.mobile_runtime = true;
        runtime.config_path = dir.join("config.toml");

        runtime.dispatch(NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                exit_dns_mode: Some("through_exit".to_string()),
                exit_dns_through_exit_servers: Some(String::new()),
                ..SettingsPatch::default()
            },
        });
        assert!(runtime.last_error.contains("at least one IP address"));
        assert_eq!(runtime.config.exit_dns.mode, ExitDnsMode::Automatic);

        runtime.dispatch(NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                exit_dns_mode: Some("encrypted".to_string()),
                exit_dns_doh_provider: Some("custom".to_string()),
                exit_dns_custom_doh_url: Some(
                    "https://resolver.example/dns-query".to_string(),
                ),
                exit_dns_custom_doh_bootstrap_ips: Some("192.0.2.53, 192.0.2.54".to_string()),
                ..SettingsPatch::default()
            },
        });
        assert!(runtime.last_error.is_empty(), "{}", runtime.last_error);
        let state = runtime.state();
        assert_eq!(state.exit_dns_mode, "encrypted");
        assert_eq!(state.exit_dns_doh_provider, "custom");
        assert_eq!(
            state.exit_dns_custom_doh_url,
            "https://resolver.example/dns-query"
        );
        assert_eq!(
            state.exit_dns_custom_doh_bootstrap_ips,
            "192.0.2.53, 192.0.2.54"
        );
        let saved = AppConfig::load(&runtime.config_path).expect("load saved config");
        assert_eq!(saved.exit_dns.mode, ExitDnsMode::Encrypted);
        assert_eq!(saved.exit_dns.doh_provider, ExitDohProvider::Custom);

        let _ = fs::remove_dir_all(&dir);
    }
