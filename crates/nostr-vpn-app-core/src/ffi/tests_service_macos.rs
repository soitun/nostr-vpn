    #[cfg(unix)]
    #[test]
    fn connect_vpn_resumes_running_daemon_without_elevated_start() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_service_test_dir("nvpn-app-core-resume");
        let calls_path = dir.join("calls.txt");
        let script_path = dir.join("nvpn");
        let calls_literal = calls_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
CALLS="{calls_literal}"
printf '%s\n' "$*" >> "$CALLS"
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{{"supported":true,"installed":true,"disabled":false,"loaded":true,"running":true,"pid":123,"label":"fi.siriusbusiness.nvpn.test","binary_version":"test"}}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  cat <<'JSON'
{{"daemon":{{"running":true,"state":{{"updated_at":1,"binary_version":"test","local_endpoint":"","advertised_endpoint":"","listen_port":0,"vpn_enabled":false,"vpn_active":false,"vpn_status":"Paused","expected_peer_count":0,"connected_peer_count":0,"mesh_ready":false,"peers":[]}}}}}}
JSON
  exit 0
fi
if [ "$1" = "resume" ]; then
  exit 0
fi
if [ "$1" = "start" ]; then
  echo "unexpected elevated start" >&2
  exit 42
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
        runtime.config_path = dir.join("config.toml");
        create_test_network(&mut runtime, "Home");
        runtime
            .config
            .save(&runtime.config_path)
            .expect("save test config");
        runtime.nvpn_bin = Some(script_path);

        runtime.dispatch(NativeAppAction::ConnectVpn);

        let calls = fs::read_to_string(&calls_path).expect("read fake nvpn calls");
        assert!(calls.contains("resume --config"));
        assert!(!calls.contains("start --daemon --connect"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_connect_without_service_does_not_start_or_prompt() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_service_test_dir("nvpn-app-core-no-service");
        let calls_path = dir.join("calls.txt");
        let script_path = dir.join("nvpn");
        let calls_literal = calls_path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"#!/bin/sh
CALLS="{calls_literal}"
printf '%s\n' "$*" >> "$CALLS"
if [ "$1" = "service" ] && [ "$2" = "status" ]; then
  cat <<'JSON'
{{"supported":true,"installed":false,"disabled":false,"loaded":false,"running":false,"pid":null,"label":"fi.siriusbusiness.nvpn.test","binary_version":""}}
JSON
  exit 0
fi
if [ "$1" = "status" ]; then
  cat <<'JSON'
{{"daemon":{{"running":false,"state":null}}}}
JSON
  exit 0
fi
if [ "$1" = "start" ]; then
  echo "unexpected start" >&2
  exit 42
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
        runtime.config_path = dir.join("config.toml");
        create_test_network(&mut runtime, "Home");
        runtime
            .config
            .save(&runtime.config_path)
            .expect("save test config");
        runtime.nvpn_bin = Some(script_path);

        runtime.dispatch(NativeAppAction::ConnectVpn);

        let calls = fs::read_to_string(&calls_path).expect("read fake nvpn calls");
        assert!(calls.contains("service status --json --skip-binary-version --config"));
        assert!(calls.contains("status --json --discover-secs 0 --config"));
        assert!(!calls.contains("start --daemon --connect"));
        assert_eq!(runtime.last_error, "Install background service first");
        assert!(!runtime.vpn_enabled);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_service_action_shell_command_quotes_bundled_cli_path() {
        let command = macos_service_action_shell_command(
            Path::new("/Applications/Nostr VPN.app/Contents/Resources/nvpn"),
            &["service", "install", "--force"],
        );

        assert!(command.starts_with("'/Applications/Nostr VPN.app/Contents/Resources/nvpn' "));
        assert!(command.contains(" 'service' 'install' '--force'"));
    }
