use super::*;

const DAEMON_STATE_PERSIST_INTERVAL_SECS: u64 = 5;
const DAEMON_PEER_MAX_FUTURE_SKEW_SECS: u64 = 2;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 300;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) const DAEMON_NETWORK_REFRESH_INTERVAL_SECS: u64 = 1;
pub(crate) const DAEMON_NETWORK_EVENT_DEBOUNCE_MILLIS: u64 = 250;
pub(crate) const DAEMON_NETWORK_SETTLE_RECHECK_SECS: u64 = 5;

pub(crate) fn suppressed_platform_network_event_recheck_delay(
    suppressed_until: Option<Instant>,
    now: Instant,
) -> Option<Duration> {
    suppressed_until
        .and_then(|until| until.checked_duration_since(now))
        .filter(|delay| !delay.is_zero())
}

pub(crate) fn reschedule_suppressed_platform_network_event(
    network_interval: &mut tokio::time::Interval,
    suppressed_until: Option<Instant>,
) -> bool {
    let Some(delay) =
        suppressed_platform_network_event_recheck_delay(suppressed_until, Instant::now())
    else {
        return false;
    };
    // Applying utun routes generates notifications of its own. A delayed snapshot is a
    // no-op for those events but still observes Wi-Fi finishing a roam in this window.
    network_interval.reset_after(delay);
    true
}

pub(crate) fn schedule_platform_network_settle_recheck(
    network_interval: &mut tokio::time::Interval,
    platform_network_event: bool,
) -> bool {
    if !platform_network_event {
        return false;
    }
    // Route notifications commonly arrive while an interface is disappearing,
    // before DHCP and the replacement default route are usable. Always sample
    // again after the route burst settles; there may be no later notification.
    network_interval.reset_after(Duration::from_secs(DAEMON_NETWORK_SETTLE_RECHECK_SECS));
    true
}

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
#[path = "session_runtime/daemon_vpn/intervals.rs"]
mod daemon_vpn_intervals;
#[path = "session_runtime/daemon_vpn/join_approval.rs"]
mod daemon_vpn_join_approval;
#[cfg(feature = "paid-exit")]
#[path = "session_runtime/daemon_vpn/paid_exit.rs"]
mod daemon_vpn_paid_exit;
#[path = "session_runtime/daemon_vpn/shutdown.rs"]
mod daemon_vpn_shutdown;
#[path = "session_runtime/daemon_vpn/startup.rs"]
mod daemon_vpn_startup;
#[cfg(feature = "paid-exit")]
use daemon_vpn_paid_exit::*;
use {
    daemon_vpn_heartbeat::*, daemon_vpn_intervals::*, daemon_vpn_join_approval::*,
    daemon_vpn_shutdown::*, daemon_vpn_startup::*,
};

include!("session_runtime/daemon_vpn.rs");
include!("session_runtime/daemon_state.rs");
include!("session_runtime/tests.rs");
