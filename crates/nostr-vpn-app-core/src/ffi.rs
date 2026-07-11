use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use nostr_sdk::prelude::Event;
use nostr_vpn_core::config::{
    AppConfig, NetworkConfig, NostrPubsubMode, PendingInboundJoinRequest,
    PendingOutboundJoinRequest, derive_mesh_tunnel_ip, maybe_autoconfigure_node,
    normalize_advertised_route, normalize_magic_dns_label, normalize_nostr_pubkey,
    normalize_relay_urls, normalize_runtime_network_id, parse_wireguard_exit_config,
    wireguard_exit_config_text,
};
use nostr_vpn_core::diagnostics::ProbeStatus;
use nostr_vpn_core::paid_routes::{ExitNetworkClass, PaidExitUpstream, PaidRouteMeter};
use nostr_vpn_core::process_ext::CommandWindowExt;
use serde::Deserialize;

use crate::actions::NativeAppAction;
use crate::invite::{
    NETWORK_INVITE_VERSION, NetworkInvite, active_network_invite_code_with_endpoints,
    apply_network_invite_to_active_network, parse_network_invite, preferred_join_request_recipient,
    to_npub,
};
use crate::join_approval::prepare_join_approval;
use crate::join_request_link::{
    own_join_request_qr_code_or_link, parse_join_request_qr_code_or_link,
};
use crate::lan_pairing::{
    LAN_PAIRING_DURATION, LAN_PAIRING_STALE_AFTER, LanPairingAnnouncement, LanPairingSignal,
};
#[cfg(not(test))]
use crate::lan_pairing::{LanPairingWorker, spawn_lan_pairing_worker};
use crate::native_state::{
    NativeAppState, NativeHealthIssue, NativeInboundJoinRequestState, NativeLanPeerState,
    NativeNetworkState, NativeNetworkSummary, NativeOutboundJoinRequestState,
    NativePaidExitSellerState, NativePaidRouteMarketFilterState, NativePaidRouteMarketState,
    NativePaidRoutePaymentActionState, NativePaidRouteWalletActionState,
    NativePaidRouteWalletState, NativeParticipantState, NativePortMappingStatus, NativeProbeStatus,
    NativeRelayState,
};
use crate::platform::current_runtime_capabilities;
use crate::state::{
    DaemonPeerState, DaemonRuntimeState, HealthIssue, NetworkSummary, PortMappingStatus,
    SettingsPatch,
};

const NVPN_BIN_ENV: &str = "NVPN_CLI_PATH";
const EXTERNAL_DAEMON_ENV: &str = "NVPN_EXTERNAL_DAEMON";
const SERVICE_STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const DAEMON_STARTUP_STATUS_GRACE: Duration = Duration::from_secs(20);
const MOBILE_RUNTIME_STATE_FILE: &str = "mobile-runtime-state.json";
const MOBILE_RUNTIME_STATE_STALE_SECS: u64 = 10;
const MOBILE_RUNTIME_STATE_MAX_FUTURE_SKEW_SECS: u64 = 2;
const PEER_PRESENCE_GRACE_SECS: u64 = 90;
const PEER_PRESENCE_MAX_FUTURE_SKEW_SECS: u64 = 2;

/// Output of running a privileged command from foreign code.
///
/// `cancelled = true` means the user dismissed the elevation dialog (e.g.
/// hit Cancel on the Touch ID / password prompt). Surfaced separately so
/// the UI can avoid showing it as a hard error.
#[derive(uniffi::Record, Debug, Default)]
pub struct PrivilegedCommandOutput {
    pub success: bool,
    pub cancelled: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Foreign-implemented runner for executing the bundled `nvpn` CLI as root.
///
/// The Mac shell implements this with Authorization Services so the
/// elevation prompt can use Touch ID. When no runner is registered, the
/// Rust core falls back to spawning `osascript ... with administrator
/// privileges` (password-only).
#[uniffi::export(with_foreign)]
pub trait PrivilegedCommandRunner: Send + Sync {
    fn run(&self, executable: String, args: Vec<String>) -> PrivilegedCommandOutput;
}

#[derive(uniffi::Object, Debug)]
pub struct FfiApp {
    runtime: Mutex<NativeAppRuntime>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn new(data_dir: String, app_version: String) -> Arc<Self> {
        let runtime = NativeAppRuntime::new(&data_dir, app_version)
            .unwrap_or_else(|error| NativeAppRuntime::from_startup_error(&error));
        Arc::new(Self {
            runtime: Mutex::new(runtime),
        })
    }

    #[must_use]
    pub fn state(&self) -> NativeAppState {
        self.with_runtime(|runtime| runtime.state())
    }

    #[must_use]
    pub fn refresh(&self) -> NativeAppState {
        self.dispatch(NativeAppAction::Tick)
    }

    #[must_use]
    pub fn dispatch(&self, action: NativeAppAction) -> NativeAppState {
        self.with_runtime(|runtime| {
            runtime.dispatch(action);
            runtime.state()
        })
    }

    pub fn set_privileged_command_runner(&self, runner: Arc<dyn PrivilegedCommandRunner>) {
        self.with_runtime(|runtime| {
            #[cfg(target_os = "macos")]
            {
                runtime.privileged_command_runner = Some(PrivilegedCommandRunnerHandle(runner));
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = runner;
            }
            runtime.state()
        });
    }
}

impl FfiApp {
    #[must_use]
    pub fn new_with_config_path(
        config_path: PathBuf,
        app_version: String,
        nvpn_bin: Option<PathBuf>,
    ) -> Arc<Self> {
        let runtime = NativeAppRuntime::new_with_config_path(config_path, app_version, nvpn_bin)
            .unwrap_or_else(|error| NativeAppRuntime::from_startup_error(&error));
        Arc::new(Self {
            runtime: Mutex::new(runtime),
        })
    }
}

impl FfiApp {
    fn with_runtime(
        &self,
        f: impl FnOnce(&mut NativeAppRuntime) -> NativeAppState,
    ) -> NativeAppState {
        match self.runtime.lock() {
            Ok(mut runtime) => f(&mut runtime),
            Err(poisoned) => {
                let mut runtime = poisoned.into_inner();
                runtime.set_error("native app state lock was poisoned");
                f(&mut runtime)
            }
        }
    }
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct NativeAppRuntime {
    rev: u64,
    app_version: String,
    config_path: PathBuf,
    config: AppConfig,
    nvpn_bin: Option<PathBuf>,
    mobile_runtime: bool,
    startup_error: Option<String>,
    last_error: String,
    daemon_running: bool,
    vpn_enabled: bool,
    vpn_active: bool,
    vpn_status: String,
    daemon_state: Option<DaemonRuntimeState>,
    service_supported: bool,
    service_enablement_supported: bool,
    service_installed: bool,
    service_disabled: bool,
    service_running: bool,
    service_status_detail: String,
    service_binary_version: String,
    expected_service_binary_version: String,
    daemon_status_grace_until: Option<Instant>,
    last_service_status_refresh_at: Option<Instant>,
    lan_pairing_worker: Option<NativeLanPairingWorker>,
    invite_broadcast_expires_at: Option<SystemTime>,
    nearby_discovery_expires_at: Option<SystemTime>,
    lan_peers: HashMap<String, LanPeerRecord>,
    paid_route_market_filter: NativePaidRouteMarketFilterState,
    paid_route_wallet_last_action: NativePaidRouteWalletActionState,
    paid_route_payment_last_action: NativePaidRoutePaymentActionState,
    #[cfg(test)]
    published_join_approval_events: Vec<Event>,
    #[cfg(target_os = "macos")]
    privileged_command_runner: Option<PrivilegedCommandRunnerHandle>,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
struct PrivilegedCommandRunnerHandle(Arc<dyn PrivilegedCommandRunner>);

#[cfg(target_os = "macos")]
impl std::fmt::Debug for PrivilegedCommandRunnerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivilegedCommandRunnerHandle(<foreign>)")
    }
}

#[derive(Debug, Clone)]
struct LanPeerRecord {
    signal: LanPairingSignal,
    last_seen: SystemTime,
}

#[cfg(not(test))]
#[derive(Debug)]
struct NativeLanPairingWorker(LanPairingWorker);

#[cfg(test)]
#[derive(Debug)]
struct NativeLanPairingWorker;

impl NativeLanPairingWorker {
    #[cfg(not(test))]
    fn spawn(announcement: LanPairingAnnouncement) -> Result<Self> {
        Ok(Self(spawn_lan_pairing_worker(announcement)?))
    }

    #[cfg(test)]
    fn spawn(announcement: LanPairingAnnouncement) -> Result<Self> {
        let LanPairingAnnouncement {
            npub,
            node_name,
            endpoint,
            invite,
        } = announcement;
        anyhow::ensure!(!npub.trim().is_empty(), "LAN pairing npub is missing");
        let _ = (node_name, endpoint, invite);
        Ok(Self)
    }

    #[cfg(not(test))]
    fn drain(&mut self) -> Vec<LanPairingSignal> {
        self.0.drain()
    }

    #[cfg(test)]
    fn drain(&mut self) -> Vec<LanPairingSignal> {
        let _ = self;
        Vec::new()
    }

    #[cfg(not(test))]
    fn set_broadcast_until(&self, expires_at: SystemTime) {
        self.0.set_broadcast_until(expires_at);
    }

    #[cfg(test)]
    fn set_broadcast_until(&self, expires_at: SystemTime) {
        let _ = (self, expires_at);
    }

    #[cfg(not(test))]
    fn set_listen_until(&self, expires_at: SystemTime) {
        self.0.set_listen_until(expires_at);
    }

    #[cfg(test)]
    fn set_listen_until(&self, expires_at: SystemTime) {
        let _ = (self, expires_at);
    }

    #[cfg(not(test))]
    fn clear_broadcast(&self) {
        self.0.clear_broadcast();
    }

    #[cfg(test)]
    fn clear_broadcast(&self) {
        let _ = self;
    }

    #[cfg(not(test))]
    fn clear_listen(&self) {
        self.0.clear_listen();
    }

    #[cfg(test)]
    fn clear_listen(&self) {
        let _ = self;
    }

    #[cfg(not(test))]
    fn update_announcement(&self, announcement: LanPairingAnnouncement) {
        self.0.update_announcement(announcement);
    }

    #[cfg(test)]
    fn update_announcement(&self, announcement: LanPairingAnnouncement) {
        let _ = (self, announcement);
    }

    #[cfg(not(test))]
    fn stop(&mut self) {
        self.0.stop();
    }

    #[cfg(test)]
    fn stop(&mut self) {
        let _ = self;
    }
}

#[derive(Debug, Deserialize)]
struct CliStatusResponse {
    daemon: CliDaemonStatus,
}

#[derive(Debug, Deserialize)]
struct CliDaemonStatus {
    running: bool,
    state: Option<DaemonRuntimeState>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(clippy::struct_excessive_bools)]
struct CliServiceStatusResponse {
    supported: bool,
    installed: bool,
    #[serde(default)]
    disabled: bool,
    loaded: bool,
    running: bool,
    pid: Option<u32>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    plist_path: String,
    #[serde(default)]
    binary_version: String,
}

#[derive(Debug, Clone, Default)]
struct ExitNodeUiStatus {
    active: bool,
    blocked: bool,
    text: String,
}

include!("ffi/runtime_lifecycle.rs");
include!("ffi/runtime_paid_exit.rs");
include!("ffi/runtime_actions.rs");
include!("ffi/runtime_config.rs");
include!("ffi/runtime_network.rs");
include!("ffi/runtime_service.rs");
include!("ffi/helpers.rs");

#[cfg(test)]
mod tests {
    include!("ffi/tests_core.rs");
    include!("ffi/tests_network.rs");
    include!("ffi/tests_network_join_actions.rs");
    include!("ffi/tests_service.rs");
    include!("ffi/tests_service_macos.rs");
}
