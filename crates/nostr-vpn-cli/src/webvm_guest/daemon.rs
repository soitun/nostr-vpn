use super::*;

pub(crate) fn args_from_daemon(args: &DaemonArgs) -> Result<Option<WebvmGuestArgs>> {
    let Some(ethernet_interface) = args.webvm_ethernet_interface.as_ref() else {
        if args.webvm_discovery_scope.is_some() {
            return Err(anyhow!(
                "--webvm-discovery-scope requires --webvm-ethernet-interface"
            ));
        }
        return Ok(None);
    };
    let discovery_scope = args
        .webvm_discovery_scope
        .as_ref()
        .ok_or_else(|| anyhow!("--webvm-ethernet-interface requires --webvm-discovery-scope"))?;
    Ok(Some(WebvmGuestArgs {
        config: args.config.clone().unwrap_or_else(default_config_path),
        ethernet_interface: ethernet_interface.clone(),
        discovery_scope: discovery_scope.clone(),
        join_pubsub_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        pairing_uri_file: PathBuf::from(DEFAULT_WEBVM_PAIRING_URI_PATH),
        tun_interface: args.iface.clone(),
    }))
}

pub(crate) async fn run_daemon(args: WebvmGuestArgs, service: bool) -> Result<()> {
    validate_args(&args)?;

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (args, service);
        Err(anyhow!("webvm-guest is supported only on Linux"))
    }

    #[cfg(target_os = "linux")]
    run_daemon_linux(args, service).await
}

#[cfg(target_os = "linux")]
async fn run_daemon_linux(args: WebvmGuestArgs, service: bool) -> Result<()> {
    if service && let Err(error) = redirect_stdio_to_daemon_log(&args.config) {
        eprintln!("daemon: failed to redirect WebVM service log: {error}");
    }
    if let Err(error) = compact_daemon_log_if_needed(&args.config) {
        eprintln!("daemon: failed to compact WebVM service log: {error}");
    }
    ensure_no_other_daemon_processes_for_config(&args.config, std::process::id())?;
    let pid_file = daemon_pid_file_path(&args.config);
    write_daemon_pid_record(
        &pid_file,
        &DaemonPidRecord {
            pid: std::process::id(),
            config_path: args.config.display().to_string(),
            started_at: unix_timestamp(),
        },
    )?;
    let _ = fs::remove_file(daemon_control_file_path(&args.config));
    clear_daemon_control_result(&args.config);

    let result = run_linux(args).await;
    if read_daemon_pid_record(&pid_file)
        .ok()
        .flatten()
        .is_some_and(|record| record.pid == std::process::id())
    {
        let _ = fs::remove_file(&pid_file);
    }
    match result {
        Err(error) if error.downcast_ref::<WebvmStop>().is_some() => Ok(()),
        other => other,
    }
}
