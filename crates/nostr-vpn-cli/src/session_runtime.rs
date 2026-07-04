use super::*;

const DAEMON_STATE_PERSIST_INTERVAL_SECS: u64 = 1;
const DAEMON_PEER_MAX_FUTURE_SKEW_SECS: u64 = 2;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 15;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 1;
pub(crate) const DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS: u64 = 250;
#[cfg(any(target_os = "macos", test))]
pub(crate) const MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS: u64 = 5;

#[cfg(feature = "embedded-fips")]
macro_rules! current_fips_peer_statuses {
    ($runtime:expr) => {
        $runtime
            .as_ref()
            .map(|runtime| runtime.peer_statuses())
            .unwrap_or_default()
    };
}

#[cfg(not(feature = "embedded-fips"))]
macro_rules! current_fips_peer_statuses {
    ($runtime:expr) => {
        Vec::<MeshPeerStatus>::new()
    };
}

#[cfg(feature = "embedded-fips")]
macro_rules! current_fips_endpoint_peer_states {
    ($signature:expr) => {
        daemon_endpoint_peer_states_from_signature($signature)
    };
}

#[cfg(not(feature = "embedded-fips"))]
macro_rules! current_fips_endpoint_peer_states {
    ($signature:expr) => {
        Vec::<DaemonFipsEndpointPeerState>::new()
    };
}

include!("session_runtime/fips_status_helpers.rs");
include!("session_runtime/connect_vpn.rs");
include!("session_runtime/daemon_vpn.rs");
include!("session_runtime/daemon_state.rs");
include!("session_runtime/tests.rs");
