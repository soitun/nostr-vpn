#[cfg(target_os = "windows")]
fn windows_install_service(
    executable: &Path,
    config_path: &Path,
    iface: &str,
    mesh_refresh_interval_secs: u64,
    force: bool,
) -> Result<()> {
    stop_daemon(StopArgs {
        config: Some(config_path.to_path_buf()),
        timeout_secs: 5,
        force: true,
    })?;

    if force {
        windows_stop_service(true)?;
        windows_delete_service(true)?;
        windows_wait_for_service_deleted(Duration::from_secs(10))?;
    } else if windows_service_query()?.is_some() {
        return Err(anyhow!(
            "system service already installed (pass --force to reinstall)"
        ));
    }

    let bin_path = windows_service_bin_path(
        executable,
        config_path,
        iface,
        mesh_refresh_interval_secs.max(1),
    );
    run_sc_checked(
        &[
            "create",
            WINDOWS_SERVICE_NAME,
            "type=",
            "own",
            "start=",
            "auto",
            "binPath=",
            &bin_path,
            "DisplayName=",
            WINDOWS_SERVICE_DISPLAY_NAME,
        ],
        "create service",
    )?;
    run_sc_checked(
        &[
            "description",
            WINDOWS_SERVICE_NAME,
            WINDOWS_SERVICE_DESCRIPTION,
        ],
        "set service description",
    )?;
    windows_start_service_and_wait(false, Duration::from_secs(10))?;
    println!("installed system service: {}", WINDOWS_SERVICE_NAME);
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_uninstall_service() -> Result<()> {
    windows_stop_service(true)?;
    windows_delete_service(true)?;
    println!("removed system service: {}", WINDOWS_SERVICE_NAME);
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_enable_service() -> Result<()> {
    let status = windows_query_service_status(true)?;
    if !status.installed {
        return Err(anyhow!("system service is not installed"));
    }

    run_sc_checked(
        &["config", WINDOWS_SERVICE_NAME, "start=", "auto"],
        "configure service start type",
    )?;
    windows_start_service_and_wait(true, Duration::from_secs(10))?;
    println!("enabled system service: {}", WINDOWS_SERVICE_NAME);
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_disable_service() -> Result<()> {
    let status = windows_query_service_status(true)?;
    if !status.installed {
        return Err(anyhow!("system service is not installed"));
    }

    windows_stop_service(true)?;
    run_sc_checked(
        &["config", WINDOWS_SERVICE_NAME, "start=", "disabled"],
        "configure service start type",
    )?;
    println!("disabled system service: {}", WINDOWS_SERVICE_NAME);
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_should_apply_config_via_service(status: &ServiceStatusView) -> bool {
    status.installed && !status.disabled
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_query_service_status(
    include_binary_version: bool,
) -> Result<ServiceStatusView> {
    let query = windows_service_query()?;
    let Some(query) = query else {
        return Ok(ServiceStatusView {
            supported: true,
            installed: false,
            disabled: false,
            loaded: false,
            running: false,
            pid: None,
            label: WINDOWS_SERVICE_NAME.to_string(),
            plist_path: WINDOWS_SERVICE_NAME.to_string(),
            binary_path: String::new(),
            binary_version: String::new(),
        });
    };

    let config = windows_service_config_query()?
        .ok_or_else(|| anyhow!("service query succeeded but service config lookup failed"))?;
    let query_text = String::from_utf8_lossy(&query.stdout);
    let config_text = String::from_utf8_lossy(&config.stdout);
    let (running, pid) = windows_service_status_from_query_output(&query_text);
    let disabled = windows_service_disabled_from_qc_output(&config_text);
    let service_binary = windows_service_binary_path_from_sc_qc_output(&config_text);
    let binary_version = if include_binary_version {
        service_binary
            .as_ref()
            .and_then(|path| query_binary_version(path))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(ServiceStatusView {
        supported: true,
        installed: true,
        disabled,
        loaded: !disabled,
        running,
        pid,
        label: WINDOWS_SERVICE_NAME.to_string(),
        plist_path: WINDOWS_SERVICE_NAME.to_string(),
        binary_path: service_binary
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        binary_version,
    })
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_service_binary_path_from_sc_qc_output(output: &str) -> Option<PathBuf> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with("BINARY_PATH_NAME"))?;
    let (_, command) = line.split_once(':')?;
    let command = command.trim();
    if let Some(quoted) = command.strip_prefix('"') {
        let executable = quoted.split_once('"')?.0;
        if executable.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(executable))
        }
    } else {
        let executable = command.split_whitespace().next()?.trim();
        if executable.is_empty() {
            None
        } else {
            Some(PathBuf::from(executable))
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_service_query() -> Result<Option<std::process::Output>> {
    let output = run_sc_raw(&["queryex", WINDOWS_SERVICE_NAME], "query service")?;
    if output.status.success() {
        return Ok(Some(output));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim());
    if windows_service_missing_message(&details) {
        return Ok(None);
    }

    Err(anyhow!(
        "sc query failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_service_config_query() -> Result<Option<std::process::Output>> {
    let output = run_sc_raw(&["qc", WINDOWS_SERVICE_NAME], "query service config")?;
    if output.status.success() {
        return Ok(Some(output));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim());
    if windows_service_missing_message(&details) {
        return Ok(None);
    }

    Err(anyhow!(
        "sc qc failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn windows_start_service(ignore_already_running: bool) -> Result<()> {
    let output = run_sc_raw(&["start", WINDOWS_SERVICE_NAME], "start service")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim());
    if ignore_already_running && windows_service_already_running_message(&details) {
        return Ok(());
    }
    Err(anyhow!(
        "sc start failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_start_service_and_wait(
    ignore_already_running: bool,
    timeout: Duration,
) -> Result<()> {
    windows_start_service(ignore_already_running)?;
    windows_wait_for_service_running(timeout)
}

#[cfg(target_os = "windows")]
fn windows_wait_for_service_running(timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if windows_query_service_status(true)?.running {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }

    Err(anyhow!(
        "system service did not reach running state within {}s",
        timeout.as_secs()
    ))
}

#[cfg(target_os = "windows")]
fn windows_stop_service(ignore_missing: bool) -> Result<()> {
    let output = run_sc_raw(&["stop", WINDOWS_SERVICE_NAME], "stop service")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim());
    if ignore_missing
        && (windows_service_missing_message(&details)
            || windows_service_not_active_message(&details))
    {
        return Ok(());
    }
    Err(anyhow!(
        "sc stop failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn windows_delete_service(ignore_missing: bool) -> Result<()> {
    let output = run_sc_raw(&["delete", WINDOWS_SERVICE_NAME], "delete service")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{}\n{}", stdout.trim(), stderr.trim());
    if ignore_missing && windows_service_missing_message(&details) {
        return Ok(());
    }
    Err(anyhow!(
        "sc delete failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn windows_wait_for_service_deleted(timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if windows_service_query()?.is_none() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }

    Err(anyhow!(
        "system service did not finish deletion within {}s",
        timeout.as_secs()
    ))
}

#[cfg(target_os = "windows")]
fn run_sc_checked(args: &[&str], context: &str) -> Result<std::process::Output> {
    let output = run_sc_raw(args, context)?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(
        "sc {context} failed\nstdout: {}\nstderr: {}",
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn run_sc_raw(args: &[&str], context: &str) -> Result<std::process::Output> {
    ProcessCommand::new("sc.exe")
        .args(args)
        .output()
        .with_context(|| format!("failed to sc.exe {context}"))
}

#[cfg(target_os = "windows")]
fn windows_service_missing_message(details: &str) -> bool {
    let lowered = details.to_ascii_lowercase();
    lowered.contains("failed 1060") || lowered.contains("does not exist as an installed service")
}

#[cfg(target_os = "windows")]
fn windows_service_not_active_message(details: &str) -> bool {
    let lowered = details.to_ascii_lowercase();
    lowered.contains("failed 1062") || lowered.contains("service has not been started")
}

#[cfg(target_os = "windows")]
fn windows_service_already_running_message(details: &str) -> bool {
    let lowered = details.to_ascii_lowercase();
    lowered.contains("failed 1056")
        || lowered.contains("instance of the service is already running")
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_service_status_from_query_output(output: &str) -> (bool, Option<u32>) {
    let mut running = false;
    let mut pid = None;

    for line in output.lines().map(str::trim) {
        if line.contains("STATE") && line.to_ascii_uppercase().contains("RUNNING") {
            running = true;
        } else if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case("PID")
        {
            pid = parse_nonzero_pid(value);
        }
    }

    (running, pid)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_service_disabled_from_qc_output(output: &str) -> bool {
    output.lines().map(str::trim).any(|line| {
        line.to_ascii_uppercase().contains("START_TYPE")
            && line.to_ascii_uppercase().contains("DISABLED")
    })
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_service_bin_path(
    executable: &Path,
    config_path: &Path,
    iface: &str,
    mesh_refresh_interval_secs: u64,
) -> String {
    [
        windows_command_line_quote(&executable.display().to_string()),
        "daemon".to_string(),
        "--service".to_string(),
        "--config".to_string(),
        windows_command_line_quote(&config_path.display().to_string()),
        "--iface".to_string(),
        windows_command_line_quote(iface),
        "--mesh-refresh-interval-secs".to_string(),
        mesh_refresh_interval_secs.max(1).to_string(),
    ]
    .join(" ")
}
