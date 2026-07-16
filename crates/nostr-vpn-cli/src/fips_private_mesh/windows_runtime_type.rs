#[cfg(target_os = "windows")]
pub(crate) struct FipsPrivateTunnelRuntime {
    iface: String,
    mesh: Arc<FipsPrivateMeshRuntime>,
    control_pubsub: Option<crate::control_pubsub_runtime::ControlPubsubFipsRuntime>,
    join_approval_ack: Option<DirectJoinApprovalAckRuntime>,
    nostr_relay_adapter: Option<NostrRelayAdapter>,
    secure_dns: Option<crate::secure_dns_runtime::SecureDnsRuntime>,
    config: FipsPrivateTunnelConfig,
    session: Arc<Session>,
    stop: Arc<AtomicBool>,
    tun_read_thread: ThreadJoinHandle<()>,
    mesh_recv_task: JoinHandle<()>,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    interface_index: u32,
    route_targets: Vec<String>,
    endpoint_bypass_underlay: Option<crate::wg_upstream_runtime::WindowsDefaultRoute>,
    endpoint_bypass_routes: Vec<String>,
    /// Same shape as the macOS variant: a userspace WG upstream
    /// tunnel (boringtun + a *separate* WinTun adapter, distinct from
    /// the FIPS adapter above) that the daemon reconciles whenever
    /// `wireguard_exit` changes.
    wg_upstream: Option<crate::wg_upstream_runtime::DaemonWgUpstream>,
}
