use super::*;

pub(super) struct DaemonVpnIntervals {
    pub(super) state: tokio::time::Interval,
    pub(super) tunnel_heartbeat: tokio::time::Interval,
    pub(super) network: tokio::time::Interval,
    pub(super) runtime_resume_pending: bool,
}

pub(super) fn daemon_vpn_intervals() -> DaemonVpnIntervals {
    let interval = |seconds| {
        let mut timer = tokio::time::interval(Duration::from_secs(seconds));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer
    };
    DaemonVpnIntervals {
        state: interval(1),
        tunnel_heartbeat: interval(2),
        network: interval(DAEMON_NETWORK_REFRESH_INTERVAL_SECS),
        runtime_resume_pending: false,
    }
}
