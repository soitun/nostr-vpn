use super::*;

const DAEMON_STATE_PERSIST_INTERVAL_SECS: u64 = 5;
const DAEMON_PEER_MAX_FUTURE_SKEW_SECS: u64 = 2;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 300;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 1;
pub(crate) const DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS: u64 = 250;
macro_rules! current_fips_peer_statuses {
    ($runtime:expr) => {
        $runtime
            .as_ref()
            .map(|runtime| runtime.peer_statuses())
            .unwrap_or_default()
    };
}
macro_rules! current_fips_endpoint_peer_states {
    ($signature:expr) => {
        daemon_endpoint_peer_states_from_signature($signature)
    };
}

include!("session_runtime/fips_status_helpers.rs");
include!("session_runtime/connect_vpn.rs");

#[path = "session_runtime/daemon_vpn/heartbeat.rs"]
mod daemon_vpn_heartbeat;
#[path = "session_runtime/daemon_vpn/join_approval.rs"]
mod daemon_vpn_join_approval;
#[cfg(feature = "paid-exit")]
#[path = "session_runtime/daemon_vpn/paid_exit.rs"]
mod daemon_vpn_paid_exit;
#[path = "session_runtime/daemon_vpn/startup.rs"]
mod daemon_vpn_startup;
#[cfg(feature = "paid-exit")]
use daemon_vpn_paid_exit::*;
use {daemon_vpn_heartbeat::*, daemon_vpn_join_approval::*, daemon_vpn_startup::*};

include!("session_runtime/daemon_vpn.rs");
include!("session_runtime/daemon_state.rs");
include!("session_runtime/tests.rs");
