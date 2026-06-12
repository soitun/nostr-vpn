// ---------------------------------------------------------------------------
// Windows daemon entry point. Mirrors the macOS variant: creates a
// dedicated WinTun adapter for the WG upstream, runs the userspace
// state machine, watchdogs the handshake, and only swaps the default
// route after a successful handshake.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub async fn apply_daemon_wg_upstream(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    match apply_daemon_wg_upstream_native(config, handshake_timeout).await {
        Ok(handle) => return Ok(handle),
        Err(error) => {
            tracing::warn!(
                ?error,
                "wg-upstream: native WireGuardNT backend unavailable; falling back to userspace"
            );
        }
    }
    apply_daemon_wg_upstream_userspace(config, handshake_timeout).await
}

#[cfg(target_os = "windows")]
async fn apply_daemon_wg_upstream_native(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    let tools = resolve_windows_wireguard_tools()?;
    let fingerprint = WireGuardExitFingerprint::from_config(config);
    let tunnel_name = windows_native_wireguard_tunnel_name(config);
    let upstream = resolve_windows_wireguard_endpoint(&config.endpoint).await?;
    let config_path = write_windows_native_wireguard_config(&tunnel_name, config)?;

    let mut tunnel = WindowsNativeWireGuardTunnel {
        name: tunnel_name.clone(),
        config_path,
        wireguard_exe: tools.wireguard_exe.clone(),
    };

    let _ = run_windows_wireguard_command(
        &tools.wireguard_exe,
        &["/uninstalltunnelservice", &tunnel_name],
    );
    let config_path_arg = tunnel.config_path.to_string_lossy().into_owned();
    if let Err(error) = run_windows_wireguard_command(
        &tools.wireguard_exe,
        &["/installtunnelservice", &config_path_arg],
    )
    .with_context(|| {
        format!(
            "install native WireGuardNT tunnel service from {}",
            tunnel.config_path.display()
        )
    }) {
        let _ = std::fs::remove_file(&tunnel.config_path);
        return Err(error);
    }
    // The WireGuard tunnel service receives the config path as its
    // startup argument, so keep the file around while the native
    // service is alive. `WindowsNativeWireGuardTunnel::cleanup` removes
    // it after uninstalling the service.

    if !wait_windows_native_wireguard_handshake(&tools.wg_exe, &tunnel_name, handshake_timeout)
        .await?
    {
        let _ = tunnel.cleanup();
        return Err(anyhow!(
            "native WireGuardNT handshake to {upstream} did not complete within {}s",
            handshake_timeout.as_secs()
        ));
    }

    Ok(DaemonWgUpstream {
        iface: tunnel_name,
        upstream,
        full_route: None,
        backend: WindowsWgUpstreamBackend::Native(tunnel),
        config_fingerprint: fingerprint,
    })
}

#[cfg(target_os = "windows")]
struct WindowsWireGuardTools {
    wireguard_exe: PathBuf,
    wg_exe: PathBuf,
}

#[cfg(target_os = "windows")]
fn resolve_windows_wireguard_tools() -> Result<WindowsWireGuardTools> {
    let wireguard_exe = resolve_windows_wireguard_tool("wireguard.exe")?;
    let wg_exe = wireguard_exe
        .parent()
        .map(|dir| dir.join("wg.exe"))
        .filter(|path| path.is_file())
        .or_else(|| resolve_windows_wireguard_tool("wg.exe").ok())
        .ok_or_else(|| anyhow!("wg.exe not found next to {}", wireguard_exe.display()))?;
    Ok(WindowsWireGuardTools {
        wireguard_exe,
        wg_exe,
    })
}

#[cfg(target_os = "windows")]
fn resolve_windows_wireguard_tool(name: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(name));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("WireGuard").join(name));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        candidates.push(
            PathBuf::from(program_files_x86)
                .join("WireGuard")
                .join(name),
        );
    }
    candidates.push(PathBuf::from(r"C:\Program Files\WireGuard").join(name));

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let output = ProcessCommand::new("where")
        .arg(name)
        .output()
        .with_context(|| format!("search PATH for {name}"))?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(path) = stdout.lines().map(str::trim).find(|line| !line.is_empty()) {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(path);
            }
        }
    }

    Err(anyhow!("{name} not found"))
}

#[cfg(any(test, target_os = "windows"))]
fn windows_native_wireguard_tunnel_name(config: &WireGuardExitConfig) -> String {
    let raw = if config.interface.trim().is_empty() {
        "nvpn-wg-upstream"
    } else {
        config.interface.trim()
    };
    let mut name = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            name.push(ch);
        } else {
            name.push('-');
        }
    }
    let name = name.trim_matches('-');
    if name.is_empty() {
        "nvpn-wg-upstream".to_string()
    } else {
        name.chars().take(64).collect()
    }
}

#[cfg(target_os = "windows")]
fn write_windows_native_wireguard_config(
    tunnel_name: &str,
    config: &WireGuardExitConfig,
) -> Result<PathBuf> {
    let root = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("nostr-vpn")
        .join("wireguard");
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create native WireGuard config dir {}", root.display()))?;
    let path = root.join(format!("{tunnel_name}.conf"));
    let config_text = nostr_vpn_core::config::wireguard_exit_config_text(config);
    std::fs::write(&path, config_text)
        .with_context(|| format!("write native WireGuard config {}", path.display()))?;
    restrict_windows_native_wireguard_config_acl(&path);
    Ok(path)
}

#[cfg(target_os = "windows")]
fn restrict_windows_native_wireguard_config_acl(path: &Path) {
    let output = ProcessCommand::new("icacls")
        .arg(path)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:F",
            "*S-1-5-32-544:F",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                status = %output.status,
                stdout = %stdout.trim(),
                stderr = %stderr.trim(),
                "wg-upstream: failed to restrict native WireGuard config ACL"
            );
        }
        Err(error) => {
            tracing::warn!(
                ?error,
                "wg-upstream: failed to run icacls for native WireGuard config"
            );
        }
    }
}

#[cfg(target_os = "windows")]
async fn resolve_windows_wireguard_endpoint(endpoint: &str) -> Result<SocketAddr> {
    let mut addrs = tokio::net::lookup_host(endpoint.trim())
        .await
        .with_context(|| format!("resolve WireGuard endpoint {endpoint}"))?;
    addrs
        .next()
        .ok_or_else(|| anyhow!("WireGuard endpoint {endpoint} resolved no addresses"))
}

#[cfg(target_os = "windows")]
async fn wait_windows_native_wireguard_handshake(
    wg_exe: &Path,
    tunnel_name: &str,
    timeout: Duration,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if windows_native_wireguard_has_handshake(wg_exe, tunnel_name)? {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(500))).await;
    }
}

#[cfg(target_os = "windows")]
fn windows_native_wireguard_has_handshake(wg_exe: &Path, tunnel_name: &str) -> Result<bool> {
    let output = ProcessCommand::new(wg_exe)
        .args(["show", tunnel_name, "latest-handshakes"])
        .output()
        .with_context(|| format!("query native WireGuard handshakes for {tunnel_name}"))?;
    if output.status.success()
        && parse_windows_wireguard_latest_handshakes(&String::from_utf8_lossy(&output.stdout))
    {
        return Ok(true);
    }

    let output = ProcessCommand::new(wg_exe)
        .args(["show", "all", "latest-handshakes"])
        .output()
        .with_context(|| "query native WireGuard handshakes for all tunnels")?;
    if output.status.success()
        && parse_windows_wireguard_latest_handshakes_for_tunnel(
            &String::from_utf8_lossy(&output.stdout),
            tunnel_name,
        )
    {
        return Ok(true);
    }

    if output.status.success()
        && parse_windows_wireguard_latest_handshakes_for_single_active_tunnel(
            &String::from_utf8_lossy(&output.stdout),
        )
    {
        return Ok(true);
    }

    let output = ProcessCommand::new(wg_exe)
        .args(["show", tunnel_name])
        .output()
        .with_context(|| format!("query native WireGuard tunnel status for {tunnel_name}"))?;
    if output.status.success()
        && parse_windows_wireguard_show_handshake(
            &String::from_utf8_lossy(&output.stdout),
            tunnel_name,
        )
    {
        return Ok(true);
    }

    let output = ProcessCommand::new(wg_exe)
        .args(["show", "all"])
        .output()
        .with_context(|| "query native WireGuard status for all tunnels")?;
    if output.status.success()
        && parse_windows_wireguard_show_handshake(
            &String::from_utf8_lossy(&output.stdout),
            tunnel_name,
        )
    {
        return Ok(true);
    }

    Ok(false)
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_wireguard_latest_handshakes(output: &str) -> bool {
    output.lines().any(|line| {
        line.split_whitespace()
            .last()
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|timestamp| timestamp > 0)
    })
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_wireguard_latest_handshakes_for_tunnel(output: &str, tunnel_name: &str) -> bool {
    output.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            return false;
        };
        if !name.eq_ignore_ascii_case(tunnel_name) {
            return false;
        }
        parts
            .last()
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|timestamp| timestamp > 0)
    })
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_wireguard_latest_handshakes_for_single_active_tunnel(output: &str) -> bool {
    let mut active = 0usize;
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        if parts.next().is_none() {
            continue;
        }
        let Some(timestamp) = parts.last().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        if timestamp > 0 {
            active += 1;
            if active > 1 {
                return false;
            }
        }
    }
    active == 1
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_wireguard_show_handshake(output: &str, tunnel_name: &str) -> bool {
    let mut saw_interface = false;
    let mut in_target_interface = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("interface:") {
            saw_interface = true;
            in_target_interface = name.trim().eq_ignore_ascii_case(tunnel_name);
            continue;
        }
        let Some(value) = trimmed.strip_prefix("latest handshake:") else {
            continue;
        };
        if saw_interface && !in_target_interface {
            continue;
        }
        let value = value.trim();
        if !value.is_empty() && !value.eq_ignore_ascii_case("never") {
            return true;
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn run_windows_wireguard_command(exe: &Path, args: &[&str]) -> Result<()> {
    let output = ProcessCommand::new(exe)
        .args(args)
        .output()
        .with_context(|| format!("spawn {} {}", exe.display(), args.join(" ")))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{} {} failed with {}\nstdout: {}\nstderr: {}",
            exe.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
impl WindowsNativeWireGuardTunnel {
    fn cleanup(&mut self) -> Result<()> {
        let result = run_windows_wireguard_command(
            &self.wireguard_exe,
            &["/uninstalltunnelservice", &self.name],
        );
        let _ = std::fs::remove_file(&self.config_path);
        result
    }
}

#[cfg(target_os = "windows")]
async fn apply_daemon_wg_upstream_userspace(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    let fingerprint = WireGuardExitFingerprint::from_config(config);
    let adapter_name = if config.interface.trim().is_empty() {
        "nvpn-wg-upstream".to_string()
    } else {
        config.interface.clone()
    };

    let wintun = nostr_vpn_wintun::load_wintun().context("load wintun.dll for WG upstream")?;
    let adapter = wintun::Adapter::open(&wintun, &adapter_name)
        .or_else(|_| wintun::Adapter::create(&wintun, &adapter_name, "NostrVPN", None))
        .with_context(|| format!("open or create wintun adapter {adapter_name}"))?;

    let mtu = if config.mtu > 0 { config.mtu } else { 1420 };
    adapter
        .set_mtu(mtu as usize)
        .with_context(|| format!("set MTU on wintun adapter {adapter_name}"))?;
    let parsed_address = crate::windows_tunnel::windows_interface_address(&config.address)?;
    adapter
        .set_network_addresses_tuple(
            parsed_address.address.into(),
            parsed_address.mask.into(),
            None,
        )
        .with_context(|| format!("set address on wintun adapter {adapter_name}"))?;
    let interface_index = adapter
        .get_adapter_index()
        .with_context(|| format!("read interface index for {adapter_name}"))?;
    let session = Arc::new(
        adapter
            .start_session(wintun::MAX_RING_CAPACITY)
            .with_context(|| format!("start wintun session for {adapter_name}"))?,
    );

    let runtime = start_wg_runtime_with_wintun(config, session.clone())
        .await
        .context("start userspace WG runtime on wintun")?;
    let upstream = runtime.upstream();

    if !runtime.wait_for_handshake(handshake_timeout).await {
        runtime.shutdown().await;
        // Adapter and session drop here, removing the WinTun device.
        return Err(anyhow!(
            "WG upstream handshake to {upstream} did not complete within {}s; \
             routing table NOT modified",
            handshake_timeout.as_secs()
        ));
    }

    let full_route = match apply_windows_full_default_route(interface_index, upstream) {
        Ok(route) => route,
        Err(error) => {
            runtime.shutdown().await;
            return Err(error.context("swap Windows default route via WG upstream"));
        }
    };

    Ok(DaemonWgUpstream {
        iface: adapter_name,
        upstream,
        full_route: Some(full_route),
        backend: WindowsWgUpstreamBackend::Userspace {
            runtime: Some(runtime),
            _session: session,
            _adapter: adapter,
        },
        config_fingerprint: fingerprint,
    })
}
