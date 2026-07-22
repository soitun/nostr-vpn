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

    #[test]
    fn settings_patch_persists_every_exit_dns_option() {
        let cases = [
            (
                "automatic",
                SettingsPatch {
                    exit_dns_mode: Some("automatic".to_string()),
                    ..SettingsPatch::default()
                },
                "automatic",
                "cloudflare",
                "",
                "",
                "",
            ),
            (
                "encrypted Cloudflare",
                SettingsPatch {
                    exit_dns_mode: Some("encrypted".to_string()),
                    exit_dns_doh_provider: Some("cloudflare".to_string()),
                    ..SettingsPatch::default()
                },
                "encrypted",
                "cloudflare",
                "",
                "",
                "",
            ),
            (
                "encrypted Quad9",
                SettingsPatch {
                    exit_dns_mode: Some("encrypted".to_string()),
                    exit_dns_doh_provider: Some("quad9".to_string()),
                    ..SettingsPatch::default()
                },
                "encrypted",
                "quad9",
                "",
                "",
                "",
            ),
            (
                "custom encrypted",
                SettingsPatch {
                    exit_dns_mode: Some("encrypted".to_string()),
                    exit_dns_doh_provider: Some("custom".to_string()),
                    exit_dns_custom_doh_url: Some(
                        "https://resolver.example/dns-query".to_string(),
                    ),
                    exit_dns_custom_doh_bootstrap_ips: Some(
                        "192.0.2.53, 192.0.2.54".to_string(),
                    ),
                    ..SettingsPatch::default()
                },
                "encrypted",
                "custom",
                "https://resolver.example/dns-query",
                "192.0.2.53, 192.0.2.54",
                "",
            ),
            (
                "DNS through exit",
                SettingsPatch {
                    exit_dns_mode: Some("through_exit".to_string()),
                    exit_dns_through_exit_servers: Some("9.9.9.9, 149.112.112.112".to_string()),
                    ..SettingsPatch::default()
                },
                "through_exit",
                "cloudflare",
                "",
                "",
                "149.112.112.112, 9.9.9.9",
            ),
        ];

        for (label, patch, mode, provider, custom_url, bootstrap_ips, through_exit_servers) in
            cases
        {
            let dir = unique_service_test_dir("nvpn-app-core-exit-dns-option");
            let error = anyhow!("boom");
            let mut runtime = NativeAppRuntime::from_startup_error(&error);
            runtime.startup_error = None;
            runtime.mobile_runtime = true;
            runtime.config_path = dir.join("config.toml");

            runtime.dispatch(NativeAppAction::UpdateSettings { patch });
            assert!(runtime.last_error.is_empty(), "{label}: {}", runtime.last_error);
            let state = runtime.state();
            assert_eq!(state.exit_dns_mode, mode, "{label}");
            assert_eq!(state.exit_dns_doh_provider, provider, "{label}");
            assert_eq!(state.exit_dns_custom_doh_url, custom_url, "{label}");
            assert_eq!(
                state.exit_dns_custom_doh_bootstrap_ips, bootstrap_ips,
                "{label}"
            );
            assert_eq!(
                state.exit_dns_through_exit_servers, through_exit_servers,
                "{label}"
            );

            let reloaded = AppConfig::load(&runtime.config_path).expect("reload saved DNS option");
            assert_eq!(reloaded.exit_dns.mode.as_str(), mode, "{label}");
            assert_eq!(reloaded.exit_dns.doh_provider.as_str(), provider, "{label}");
            let _ = fs::remove_dir_all(&dir);
        }
    }
