#[cfg(feature = "embedded-fips")]
async fn current_fips_relay_statuses(
    runtime: &Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
) -> Vec<DaemonRelayState> {
    let Some(runtime) = runtime.as_ref() else {
        return Vec::new();
    };
    match runtime.relay_statuses().await {
        Ok(relays) => relays
            .into_iter()
            .map(|relay| DaemonRelayState {
                url: relay.url,
                status: relay.status,
            })
            .collect(),
        Err(error) => {
            eprintln!("fips: relay status snapshot failed: {error}");
            Vec::new()
        }
    }
}

#[cfg(not(feature = "embedded-fips"))]
async fn current_fips_relay_statuses<T>(_runtime: &Option<T>) -> Vec<DaemonRelayState> {
    Vec::new()
}

#[cfg(feature = "embedded-fips")]
pub(crate) const FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS: u64 = 60;
#[cfg(feature = "embedded-fips")]
pub(crate) const FIPS_PENDING_ROSTER_RESTART_GRACE_SECS: u64 = 45;

#[cfg(feature = "embedded-fips")]
macro_rules! current_fips_advertised_routes {
    ($runtime:expr, $app:expr) => {
        $runtime
            .as_ref()
            .map(|runtime| {
                let mut map = std::collections::HashMap::<String, Vec<String>>::new();
                for participant in $app.participant_pubkeys_hex() {
                    let routes = runtime.peer_advertised_routes(&participant);
                    if !routes.is_empty() {
                        map.insert(participant, routes);
                    }
                }
                map
            })
            .unwrap_or_default()
    };
}

#[cfg(not(feature = "embedded-fips"))]
macro_rules! current_fips_advertised_routes {
    ($runtime:expr, $app:expr) => {
        std::collections::HashMap::<String, Vec<String>>::new()
    };
}

#[cfg(feature = "embedded-fips")]
fn fips_peer_count(
    app: &AppConfig,
    own_pubkey: Option<&str>,
    peer_statuses: &[MeshPeerStatus],
) -> usize {
    let participant_pubkeys_list = app.participant_pubkeys_hex();
    let participant_pubkeys = participant_pubkeys_list
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    peer_statuses
        .iter()
        .filter(|status| Some(status.pubkey.as_str()) != own_pubkey)
        .filter(|status| participant_pubkeys.contains(&status.pubkey))
        .filter(|status| status.connected)
        .count()
}

#[cfg(feature = "embedded-fips")]
fn maybe_log_fips_mesh_count(
    app: &AppConfig,
    own_pubkey: Option<&str>,
    peer_statuses: &[MeshPeerStatus],
    expected_peers: usize,
    last_mesh_count: &mut usize,
) {
    let connected = fips_peer_count(app, own_pubkey, peer_statuses);
    if connected != *last_mesh_count {
        println!("mesh: {connected}/{expected_peers} peers connected");
        *last_mesh_count = connected;
    }
}

fn credible_daemon_peer_timestamp(now: u64, timestamp: Option<u64>) -> Option<u64> {
    let timestamp = timestamp?;
    if timestamp > now && timestamp - now > DAEMON_PEER_MAX_FUTURE_SKEW_SECS {
        return None;
    }
    Some(timestamp)
}

#[cfg(feature = "embedded-fips")]
async fn flush_pending_fips_roster_recipients(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    pending_recipients: &mut HashSet<String>,
) {
    if pending_recipients.is_empty() {
        return;
    }
    match publish_fips_active_network_roster(runtime, app, config_path, pending_recipients).await {
        Ok(_) => {}
        Err(error) => eprintln!("fips: queued roster publish failed: {error}"),
    }
}

#[cfg(feature = "embedded-fips")]
type EndpointPeerSignature = Vec<(String, bool, bool, Vec<(String, Option<u64>, u8)>)>;

#[cfg(feature = "embedded-fips")]
struct RecentPeerRefresh<'a> {
    recent_peers: &'a mut nostr_vpn_core::recent_peers::RecentPeerEndpoints,
    recent_peers_path: &'a std::path::Path,
    last_endpoint_peer_signature: &'a mut EndpointPeerSignature,
}

#[cfg(feature = "embedded-fips")]
struct FipsRestartContext<'a> {
    app: &'a nostr_vpn_core::config::AppConfig,
    config_path: &'a std::path::Path,
    network_id: &'a str,
    fallback_iface: &'a str,
    own_pubkey: Option<&'a str>,
    recent_peers: Option<&'a nostr_vpn_core::recent_peers::RecentPeerEndpoints>,
    last_endpoint_peer_signature: &'a mut EndpointPeerSignature,
}

#[cfg(feature = "embedded-fips")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FipsLinkEventRefresh {
    None,
    RefreshPaths,
}

#[cfg(feature = "embedded-fips")]
#[derive(Debug, Default)]
pub(crate) struct FipsPendingRosterRestartState {
    pending_since: Option<u64>,
    last_restart_at: Option<u64>,
}

#[cfg(feature = "embedded-fips")]
pub(crate) fn fips_link_event_refresh(
    network_changed: bool,
    endpoint_changed: bool,
    underlay_repaired: bool,
    resumed_after_sleep: bool,
) -> FipsLinkEventRefresh {
    if network_changed || endpoint_changed || underlay_repaired || resumed_after_sleep {
        FipsLinkEventRefresh::RefreshPaths
    } else {
        FipsLinkEventRefresh::None
    }
}

#[cfg(feature = "embedded-fips")]
pub(crate) fn fips_link_event_should_seed_recent_peers(
    refresh: FipsLinkEventRefresh,
) -> bool {
    matches!(refresh, FipsLinkEventRefresh::None)
}

#[cfg(feature = "embedded-fips")]
pub(crate) fn fips_stale_participant_restart_due(
    last_restart_at: &mut Option<u64>,
    now: u64,
) -> bool {
    let due = last_restart_at.is_none_or(|last_restart_at| {
        now < last_restart_at
            || now.saturating_sub(last_restart_at)
                >= FIPS_STALE_PARTICIPANT_RESTART_COOLDOWN_SECS
    });
    if due {
        *last_restart_at = Some(now);
    }
    due
}

#[cfg(feature = "embedded-fips")]
fn fips_pending_roster_links_detected(
    peer_statuses: &[MeshPeerStatus],
    relay_statuses: &[DaemonRelayState],
    roster_pubkeys: &HashSet<String>,
    expected_peers: usize,
) -> bool {
    if expected_peers == 0
        || roster_pubkeys.is_empty()
        || !relay_statuses
            .iter()
            .any(|relay| relay.status.eq_ignore_ascii_case("connected"))
    {
        return false;
    }
    if peer_statuses
        .iter()
        .any(|status| roster_pubkeys.contains(&status.pubkey) && status.connected)
    {
        return false;
    }
    let pending = peer_statuses
        .iter()
        .filter(|status| {
            roster_pubkeys.contains(&status.pubkey)
                && !status.connected
                && status.last_seen_at.is_none()
                && status.error.as_deref() == Some("fips link pending")
        })
        .count();
    pending >= expected_peers
}

#[cfg(feature = "embedded-fips")]
pub(crate) fn fips_pending_roster_restart_due(
    peer_statuses: &[MeshPeerStatus],
    relay_statuses: &[DaemonRelayState],
    roster_pubkeys: &HashSet<String>,
    expected_peers: usize,
    state: &mut FipsPendingRosterRestartState,
    now: u64,
) -> bool {
    if !fips_pending_roster_links_detected(
        peer_statuses,
        relay_statuses,
        roster_pubkeys,
        expected_peers,
    ) {
        state.pending_since = None;
        return false;
    }
    let pending_since = match state.pending_since {
        Some(pending_since) if now >= pending_since => pending_since,
        _ => {
            state.pending_since = Some(now);
            return false;
        }
    };
    if now.saturating_sub(pending_since) < FIPS_PENDING_ROSTER_RESTART_GRACE_SECS {
        return false;
    }
    if !fips_stale_participant_restart_due(&mut state.last_restart_at, now) {
        return false;
    }
    state.pending_since = None;
    true
}

#[cfg(feature = "embedded-fips")]
fn endpoint_peer_signature(
    endpoint_peers: &[crate::fips_private_mesh::FipsEndpointPeerTransportConfig],
) -> EndpointPeerSignature {
    endpoint_peers
        .iter()
        .map(|peer| {
            let mut addresses = peer
                .addresses
                .iter()
                .map(|hint| (hint.addr.clone(), hint.seen_at_ms, hint.priority))
                .collect::<Vec<_>>();
            addresses.sort();
            addresses.dedup();
            (
                peer.npub.clone(),
                peer.auto_reconnect,
                peer.discovery_fallback_transit,
                addresses,
            )
        })
        .collect()
}

#[cfg(feature = "embedded-fips")]
fn endpoint_peers_for_participant_refresh(
    endpoint_peers: &[crate::fips_private_mesh::FipsEndpointPeerTransportConfig],
    participants: &[String],
) -> Vec<crate::fips_private_mesh::FipsEndpointPeerTransportConfig> {
    if participants.is_empty() {
        return Vec::new();
    }

    let participant_keys = participants
        .iter()
        .filter_map(|participant| {
            nostr_sdk::prelude::PublicKey::parse(participant.trim())
                .ok()
                .map(|key| *key.as_bytes())
        })
        .collect::<std::collections::HashSet<_>>();
    if participant_keys.is_empty() {
        return Vec::new();
    }

    endpoint_peers
        .iter()
        .filter(|peer| {
            nostr_sdk::prelude::PublicKey::parse(peer.npub.trim())
                .ok()
                .is_some_and(|key| participant_keys.contains(key.as_bytes()))
        })
        .cloned()
        .collect()
}

/// Snapshot the runtime's authenticated peer transport addresses, update
/// the on-disk recent-peers cache, and hand fips the refreshed peer hint
/// list via `update_peers` so new direct candidates race the existing ones
/// in the next dial cycle without restarting the endpoint. Public (non-LAN)
/// endpoints get rotated into the cache, including authenticated non-roster
/// transit peers; mesh-carried live hints can include LAN endpoints but stay
/// in memory only.
#[cfg(feature = "embedded-fips")]
async fn update_recent_peers_from_runtime(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &nostr_vpn_core::config::AppConfig,
    network_id: &str,
    own_pubkey: Option<&str>,
    refresh: RecentPeerRefresh<'_>,
    now: u64,
) {
    let snapshot = match runtime.authenticated_peer_transport_addrs().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("fips: peer endpoint snapshot failed: {error}");
            Vec::new()
        }
    };
    let mut changed = false;
    for (participant, addr) in snapshot {
        if refresh.recent_peers.note_success(&participant, &addr, now) {
            changed = true;
        }
    }
    if refresh
        .recent_peers
        .prune_stale(now, crate::recent_peers_store::RECENT_PEERS_TTL_SECS)
    {
        changed = true;
    }
    if changed
        && let Err(error) = crate::recent_peers_store::write_recent_peers(
            refresh.recent_peers_path,
            refresh.recent_peers,
        )
    {
        eprintln!(
            "daemon: failed to write recent peers cache {}: {error}",
            refresh.recent_peers_path.display()
        );
    }
    let live_peer_endpoints = runtime.peer_endpoint_hints();
    match crate::fips_private_mesh::FipsPrivateTunnelConfig::from_app(
        app,
        network_id,
        runtime.iface().to_string(),
        own_pubkey,
        Some(refresh.recent_peers),
        &live_peer_endpoints,
    ) {
        Ok(refreshed) => {
            let signature = endpoint_peer_signature(&refreshed.endpoint_peers);
            if signature == *refresh.last_endpoint_peer_signature {
                return;
            }
            if let Err(error) = runtime.update_peers(&refreshed.endpoint_peers).await {
                eprintln!("fips: update_peers (cache refresh) failed: {error}");
            } else {
                *refresh.last_endpoint_peer_signature = signature;
            }
        }
        Err(error) => {
            eprintln!("fips: rebuilding peer hint list failed: {error}");
        }
    }
}

#[cfg(feature = "embedded-fips")]
async fn refresh_fips_tunnel_runtime_after_link_event(
    runtime: &mut Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
    context: FipsRestartContext<'_>,
    reason: &str,
) -> Result<()> {
    let config_iface = runtime
        .as_ref()
        .map(|runtime| runtime.iface().to_string())
        .unwrap_or_else(|| context.fallback_iface.to_string());
    // Link events mean the old runtime's learned endpoint hints may belong to
    // a previous underlay/NAT mapping. Keep the endpoint bound and make fips
    // re-earn direct paths from configured hints and fresh discovery.
    let live_peer_endpoints = Vec::new();
    let config = fips_tunnel_config_from_app(
        context.app,
        context.config_path,
        context.network_id,
        config_iface,
        context.own_pubkey,
        context.recent_peers,
        &live_peer_endpoints,
    )?;
    let endpoint_peer_signature = endpoint_peer_signature(&config.endpoint_peers);
    if let Some(existing) = runtime.as_mut() {
        existing.apply_config(config).await?;
        eprintln!(
            "daemon: refreshed FIPS private mesh paths on {} after {reason}",
            existing.iface()
        );
    } else {
        let started = crate::fips_private_mesh::FipsPrivateTunnelRuntime::start(config).await?;
        eprintln!("daemon: FIPS private mesh on {} after {reason}", started.iface());
        *runtime = Some(started);
    }
    *context.last_endpoint_peer_signature = endpoint_peer_signature;
    Ok(())
}

#[cfg(feature = "embedded-fips")]
async fn restart_fips_tunnel_runtime_after_stale_participants(
    runtime: &mut Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
    context: FipsRestartContext<'_>,
    last_restart_at: &mut Option<u64>,
    now: u64,
) -> Result<bool> {
    let stale_participants = runtime
        .as_ref()
        .map(|runtime| runtime.stale_participants_needing_path_refresh(now))
        .unwrap_or_default();
    if stale_participants.is_empty() {
        return Ok(false);
    }
    if !fips_stale_participant_restart_due(last_restart_at, now) {
        return Ok(false);
    }
    eprintln!(
        "daemon: refreshing FIPS peer paths after {} participant(s) stopped responding while endpoint paths need refresh",
        stale_participants.len()
    );
    refresh_fips_tunnel_runtime_peer_paths(runtime, context, &stale_participants).await
}

#[cfg(feature = "embedded-fips")]
async fn refresh_fips_tunnel_runtime_peer_paths(
    runtime: &mut Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
    context: FipsRestartContext<'_>,
    stale_participants: &[String],
) -> Result<bool> {
    let Some(current) = runtime.as_ref() else {
        return Ok(false);
    };
    refresh_fips_tunnel_runtime_peer_paths_in_place(
        current,
        context,
        stale_participants,
        "stale participant liveness",
    )
    .await?;
    Ok(false)
}

#[cfg(feature = "embedded-fips")]
async fn refresh_fips_tunnel_runtime_peer_paths_in_place(
    current: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    context: FipsRestartContext<'_>,
    participants: &[String],
    reason: &str,
) -> Result<()> {
    let live_peer_endpoints = current.peer_endpoint_hints();
    let config = fips_tunnel_config_from_app(
        context.app,
        context.config_path,
        context.network_id,
        current.iface().to_string(),
        context.own_pubkey,
        context.recent_peers,
        &live_peer_endpoints,
    )?;
    let endpoint_peer_signature = endpoint_peer_signature(&config.endpoint_peers);
    let outcome = current.update_peers(&config.endpoint_peers).await?;
    let refresh_endpoint_peers =
        endpoint_peers_for_participant_refresh(&config.endpoint_peers, participants);
    if refresh_endpoint_peers.is_empty() {
        eprintln!(
            "daemon: no matching FIPS endpoint peer paths for {} participant(s) after {reason}",
            participants.len()
        );
        *context.last_endpoint_peer_signature = endpoint_peer_signature;
        return Ok(());
    }
    let refreshed = current.refresh_peer_paths(&refresh_endpoint_peers).await?;
    *context.last_endpoint_peer_signature = endpoint_peer_signature;
    eprintln!(
        "daemon: refreshed FIPS endpoint peer paths in place after {reason} (targets={} added={} updated={} unchanged={} removed={} direct_refreshes={})",
        refresh_endpoint_peers.len(),
        outcome.added,
        outcome.updated,
        outcome.unchanged,
        outcome.removed,
        refreshed
    );
    Ok(())
}

#[cfg(feature = "embedded-fips")]
async fn restart_fips_tunnel_runtime_after_pending_roster_links(
    runtime: &mut Option<crate::fips_private_mesh::FipsPrivateTunnelRuntime>,
    context: FipsRestartContext<'_>,
    expected_peers: usize,
    state: &mut FipsPendingRosterRestartState,
    now: u64,
) -> Result<bool> {
    let Some(current) = runtime.as_ref() else {
        return Ok(false);
    };
    let peer_statuses = current.peer_statuses();
    let relay_statuses = match current.relay_statuses().await {
        Ok(relays) => relays
            .into_iter()
            .map(|relay| DaemonRelayState {
                url: relay.url,
                status: relay.status,
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            eprintln!("fips: relay status snapshot failed during pending roster recovery: {error}");
            Vec::new()
        }
    };
    if !fips_pending_roster_restart_due(
        &peer_statuses,
        &relay_statuses,
        &fips_roster_pubkeys(context.app, context.own_pubkey),
        expected_peers,
        state,
        now,
    ) {
        return Ok(false);
    }
    eprintln!(
        "daemon: refreshing FIPS private mesh paths after all {expected_peers} roster link(s) stayed pending with relay discovery connected"
    );
    refresh_fips_tunnel_runtime_after_link_event(
        runtime,
        context,
        "all FIPS roster links pending",
    )
    .await?;
    Ok(true)
}

#[cfg(feature = "embedded-fips")]
fn fips_roster_pubkeys(app: &AppConfig, own_pubkey: Option<&str>) -> HashSet<String> {
    app.participant_pubkeys_hex()
        .into_iter()
        .filter(|participant| Some(participant.as_str()) != own_pubkey)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn reset_tunnel_runtime_after_macos_underlay_repair(
    tunnel_runtime: &mut CliTunnelRuntime,
) {
    tunnel_runtime.stop();
}

#[cfg(not(any(target_os = "macos", test)))]
pub(crate) fn reset_tunnel_runtime_after_macos_underlay_repair(
    _tunnel_runtime: &mut CliTunnelRuntime,
) {
}

fn prefer_nonself_tunnel_snapshot(
    tunnel_runtime: &CliTunnelRuntime,
    previous: &crate::diagnostics::NetworkSnapshot,
    latest: crate::diagnostics::NetworkSnapshot,
) -> crate::diagnostics::NetworkSnapshot {
    let latest = crate::diagnostics::prefer_nonempty_network_snapshot(previous, latest);
    match latest.default_interface.as_deref() {
        Some(iface) if tunnel_runtime.owns_interface(iface) => previous.clone(),
        _ => latest,
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_underlay_route_check_due(
    last_check_at: &mut Instant,
    network_changed: bool,
    resumed_after_sleep: bool,
    now: Instant,
) -> bool {
    if network_changed
        || resumed_after_sleep
        || now.saturating_duration_since(*last_check_at)
            >= Duration::from_secs(MACOS_UNDERLAY_ROUTE_CHECK_INTERVAL_SECS)
    {
        *last_check_at = now;
        true
    } else {
        false
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn macos_underlay_route_repair_allowed(captive_portal: Option<bool>) -> bool {
    captive_portal != Some(true)
}

async fn capture_network_snapshot_for_daemon() -> crate::diagnostics::NetworkSnapshot {
    match tokio::task::spawn_blocking(capture_network_snapshot).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("daemon: network snapshot task failed: {error}");
            crate::diagnostics::NetworkSnapshot::default()
        }
    }
}

fn spawn_platform_network_change_monitor() -> Option<tokio::sync::mpsc::Receiver<()>> {
    #[cfg(target_os = "linux")]
    {
        crate::linux_network::spawn_linux_route_change_monitor()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_network::spawn_macos_route_change_monitor()
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_network::spawn_windows_route_change_monitor()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

async fn recv_platform_network_change(
    rx: &mut Option<tokio::sync::mpsc::Receiver<()>>,
) -> Option<()> {
    match rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn drain_platform_network_changes(rx: &mut Option<tokio::sync::mpsc::Receiver<()>>) {
    let Some(rx) = rx.as_mut() else {
        return;
    };
    while rx.try_recv().is_ok() {}
}

#[cfg(target_os = "macos")]
async fn ensure_macos_underlay_default_route_for_daemon() -> Result<bool> {
    tokio::task::spawn_blocking(crate::macos_network::ensure_macos_underlay_default_route)
        .await
        .context("macOS underlay route check task failed")?
}

#[cfg(target_os = "macos")]
async fn maybe_ensure_macos_underlay_default_route_for_daemon(
    last_check_at: &mut Instant,
    network_changed: bool,
    resumed_after_sleep: bool,
    now: Instant,
    captive_portal: Option<bool>,
) -> bool {
    if !macos_underlay_route_repair_allowed(captive_portal) {
        if macos_underlay_route_check_due(last_check_at, false, resumed_after_sleep, now) {
            eprintln!(
                "daemon: deferring macOS underlay default route repair while captive portal is detected"
            );
        }
        return false;
    }

    if !macos_underlay_route_check_due(last_check_at, network_changed, resumed_after_sleep, now) {
        return false;
    }

    match ensure_macos_underlay_default_route_for_daemon().await {
        Ok(true) => {
            eprintln!("daemon: restored missing macOS underlay default route");
            true
        }
        Ok(false) => false,
        Err(error) => {
            eprintln!("daemon: failed to ensure macOS underlay default route: {error}");
            false
        }
    }
}
