#[cfg(target_os = "windows")]
pub fn apply_windows_scoped_host_route(
    interface_index: u32,
    target: IpAddr,
) -> Result<WindowsScopedHostRoute> {
    let target = match target {
        IpAddr::V4(target) => target,
        IpAddr::V6(_) => {
            return Err(anyhow!(
                "Windows scoped WG upstream routes only support IPv4 targets"
            ));
        }
    };
    let route_targets = vec![format!("{target}/32")];
    crate::windows_tunnel::apply_windows_routes(interface_index, &route_targets)?;
    Ok(WindowsScopedHostRoute {
        interface_index,
        route_targets,
        reverted: false,
    })
}

#[cfg(target_os = "windows")]
pub struct WindowsScopedHostRoute {
    interface_index: u32,
    route_targets: Vec<String>,
    reverted: bool,
}

#[cfg(target_os = "windows")]
impl WindowsScopedHostRoute {
    pub fn revert(&mut self) -> Result<()> {
        if self.reverted {
            return Ok(());
        }
        crate::windows_tunnel::remove_windows_routes(self.interface_index, &self.route_targets)?;
        self.reverted = true;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsScopedHostRoute {
    fn drop(&mut self) {
        if let Err(error) = self.revert() {
            eprintln!(
                "wg-upstream: WARNING — Windows scoped host route cleanup failed: {error}. \
                 You may need to run `netsh interface ipv4 delete route <target>/32 \
                 interface={}` manually.",
                self.interface_index
            );
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_checked(command: &mut ProcessCommand) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("spawn {:?}", command.get_program()))?;
    if !status.success() {
        return Err(anyhow!(
            "{:?} {:?} failed: {status}",
            command.get_program(),
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Long-lived holder used by the daemon (macOS-only for now). Owns the tun,
// the userspace WG runtime, and the FullDefaultRoute guard for the lifetime
// of "WireGuard upstream is enabled". Reconciled by FipsPrivateTunnelRuntime
// whenever the config changes.
// ---------------------------------------------------------------------------

// The daemon-side code below keeps the shared WG fingerprint and
// handshake timeout definitions close to the platform routing glue.

#[cfg(target_os = "macos")]
pub struct DaemonWgUpstream {
    pub iface: String,
    pub upstream: SocketAddr,
    runtime: Option<WgUpstreamRuntime>,
    full_route: Option<FullDefaultRoute>,
    // Tun is held to keep the utun fd open for the lifetime of the
    // tunnel; dropping it auto-removes the utun device on macOS.
    _tun: Arc<TunSocket>,
    config_fingerprint: WireGuardExitFingerprint,
}

#[cfg(target_os = "windows")]
pub struct DaemonWgUpstream {
    pub iface: String,
    pub upstream: SocketAddr,
    full_route: Option<WindowsFullDefaultRoute>,
    backend: WindowsWgUpstreamBackend,
    config_fingerprint: WireGuardExitFingerprint,
}

#[cfg(target_os = "windows")]
enum WindowsWgUpstreamBackend {
    Native(WindowsNativeWireGuardTunnel),
    Userspace {
        runtime: Option<WgUpstreamRuntime>,
        // Adapter + session held to keep the WinTun device open for
        // the lifetime of the tunnel; dropping releases the WinTun
        // adapter (which removes its routes too).
        _session: Arc<WintunSession>,
        _adapter: Arc<wintun::Adapter>,
    },
}

#[cfg(target_os = "windows")]
struct WindowsNativeWireGuardTunnel {
    name: String,
    config_path: PathBuf,
    wireguard_exe: PathBuf,
}

/// Bring up the daemon-owned WG upstream tunnel: create utun, run the
/// userspace WG state machine, wait for handshake, and only then swap
/// the default route. If the handshake doesn't complete within
/// `handshake_timeout`, the tunnel is torn down and the routing table
/// is **not** modified.
///
/// This is the "happy path" entry point used by the macOS daemon
/// reconcile loop. The caller stores the returned `DaemonWgUpstream`
/// inside the long-lived runtime; dropping it (or calling `cleanup`)
/// restores the original default route.
#[cfg(target_os = "macos")]
pub async fn apply_daemon_wg_upstream(
    config: &WireGuardExitConfig,
    handshake_timeout: Duration,
) -> Result<DaemonWgUpstream> {
    let fingerprint = WireGuardExitFingerprint::from_config(config);
    let interface_hint =
        if config.interface.trim().is_empty() || !config.interface.starts_with("utun") {
            // Daemon-side: always let the kernel pick the next utunN.
            // The user-facing config's `interface` is just a label.
            "utun".to_string()
        } else {
            config.interface.clone()
        };

    let tun = TunSocket::new(&interface_hint)
        .with_context(|| format!("create utun for WG upstream (hint='{interface_hint}')"))?
        .set_non_blocking()
        .context("set utun non-blocking")?;
    let actual_iface = tun.name().context("read utun name (probably needs root)")?;
    let tun = Arc::new(tun);

    let runtime = start_wg_runtime_with_posix_tun(config, tun.clone())
        .await
        .context("start userspace WG runtime")?;
    let upstream = runtime.upstream();

    // Watchdog: wait up to `handshake_timeout` for the WG handshake to
    // complete. If it doesn't, we never modify the routing table —
    // tear down the tun + runtime and surface the error.
    if !runtime.wait_for_handshake(handshake_timeout).await {
        runtime.shutdown().await;
        return Err(anyhow!(
            "WG upstream handshake to {upstream} did not complete within {}s; \
             routing table NOT modified",
            handshake_timeout.as_secs()
        ));
    }

    let mtu = if config.mtu > 0 { config.mtu } else { 1420 };
    let full_route = match apply_full_default_route(&actual_iface, &config.address, upstream, mtu) {
        Ok(route) => route,
        Err(error) => {
            runtime.shutdown().await;
            return Err(error.context("swap default route via WG upstream"));
        }
    };

    Ok(DaemonWgUpstream {
        iface: actual_iface,
        upstream,
        runtime: Some(runtime),
        full_route: Some(full_route),
        _tun: tun,
        config_fingerprint: fingerprint,
    })
}

#[cfg(target_os = "macos")]
impl DaemonWgUpstream {
    /// Whether the daemon should consider this WG upstream tunnel
    /// equivalent to a fresh apply for `new_config`. Used by the
    /// reconcile loop to short-circuit a teardown+rebuild on every
    /// tick.
    pub fn matches(&self, new_config: &WireGuardExitConfig) -> bool {
        self.config_fingerprint == WireGuardExitFingerprint::from_config(new_config)
    }

    /// Tear down the WG upstream cleanly: remove the two WG split-default
    /// routes while leaving the physical default untouched, then stop the
    /// boringtun pump and drop the tun device.
    pub async fn cleanup(mut self) {
        if let Some(mut full_route) = self.full_route.take() {
            if let Err(error) = full_route.revert() {
                eprintln!(
                    "fips: WG upstream route revert failed: {error}. \
                     Routing table may need manual cleanup."
                );
            }
            // Drop FullDefaultRoute *after* explicit revert; the Drop
            // impl is idempotent, so doing it twice is harmless.
            drop(full_route);
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown().await;
        }
        // self._tun drops here, removing the utun device.
    }
}

#[cfg(target_os = "windows")]
impl DaemonWgUpstream {
    pub fn matches(&self, new_config: &WireGuardExitConfig) -> bool {
        self.config_fingerprint == WireGuardExitFingerprint::from_config(new_config)
    }

    pub async fn cleanup(mut self) {
        if let Some(mut full_route) = self.full_route.take() {
            if let Err(error) = full_route.revert() {
                eprintln!(
                    "fips: WG upstream route revert failed: {error}. \
                     Routing table may need manual cleanup."
                );
            }
            drop(full_route);
        }
        match self.backend {
            WindowsWgUpstreamBackend::Native(mut tunnel) => {
                if let Err(error) = tunnel.cleanup() {
                    eprintln!(
                        "fips: native WireGuardNT tunnel cleanup failed: {error}. \
                         The WireGuardTunnel${} service may need manual removal.",
                        tunnel.name
                    );
                }
            }
            WindowsWgUpstreamBackend::Userspace {
                runtime: Some(runtime),
                ..
            } => {
                runtime.shutdown().await;
            }
            WindowsWgUpstreamBackend::Userspace { runtime: None, .. } => {}
        }
        // Userspace backend session/adapter fields drop here. WinTun
        // removes its adapter when the last reference goes; native
        // backend cleanup uninstalls the official WireGuard tunnel
        // service above.
    }
}

// ---------------------------------------------------------------------------
// Windows full default-route swap. Same shape as FullDefaultRoute on
