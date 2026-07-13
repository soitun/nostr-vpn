use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use nostr_vpn_core::join_pubsub::{
    approval_applied_ack_matches_queued, load_direct_join_approvals,
};

use crate::fips_private_mesh::FipsPrivateTunnelRuntime;

const APPROVAL_RETRY_INITIAL: Duration = Duration::from_secs(2);
const APPROVAL_RETRY_MAX: Duration = Duration::from_secs(60);
const APPROVAL_INITIAL_BURST_ATTEMPTS: usize = 3;
const APPROVAL_INITIAL_BURST_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
struct PendingRetry {
    attempts: u32,
    next_send: Instant,
}

#[derive(Debug, Default)]
pub(super) struct DirectJoinApprovalDeliveryState {
    pending: HashMap<PathBuf, PendingRetry>,
}

impl DirectJoinApprovalDeliveryState {
    pub(super) async fn flush(
        &mut self,
        runtime: &mut FipsPrivateTunnelRuntime,
        app: &AppConfig,
        config_path: &Path,
    ) {
        flush_direct_join_approval_outbox(runtime, app, config_path, self).await;
    }

    fn due(&self, path: &Path, now: Instant) -> bool {
        self.pending
            .get(path)
            .is_none_or(|pending| pending.next_send <= now)
    }

    fn record_attempt(&mut self, path: PathBuf, now: Instant) {
        let attempts = self
            .pending
            .get(&path)
            .map_or(1, |pending| pending.attempts.saturating_add(1));
        let shift = attempts.saturating_sub(1).min(5);
        let delay = APPROVAL_RETRY_INITIAL
            .checked_mul(1_u32 << shift)
            .unwrap_or(APPROVAL_RETRY_MAX)
            .min(APPROVAL_RETRY_MAX);
        self.pending.insert(
            path,
            PendingRetry {
                attempts,
                next_send: now + delay,
            },
        );
    }

    fn retain(&mut self, paths: &HashSet<PathBuf>) {
        self.pending.retain(|path, _| paths.contains(path));
    }
}

async fn flush_direct_join_approval_outbox(
    runtime: &mut FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    state: &mut DirectJoinApprovalDeliveryState,
) {
    let mut queued = load_direct_join_approvals(config_path);

    for received in runtime.drain_join_approval_acks() {
        let Ok(source_peer) = normalize_nostr_pubkey(&received.source_peer) else {
            continue;
        };
        for (path, approval) in &queued {
            let expected_source = approval
                .fips_route_npub
                .as_deref()
                .unwrap_or(&approval.recipient_npub);
            if source_peer != expected_source
                || !approval_applied_ack_matches_queued(&received.ack, approval)
            {
                continue;
            }
            if let Err(error) = fs::remove_file(path) {
                eprintln!(
                    "failed to remove acknowledged direct join approval {}: {error}",
                    path.display()
                );
            } else {
                eprintln!(
                    "join approval was durably applied and acknowledged by {}",
                    approval.recipient_npub
                );
            }
        }
    }

    queued.retain(|(path, _)| path.exists());
    let live_paths = queued
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<HashSet<_>>();
    state.retain(&live_paths);

    let participants = app.participant_pubkeys_hex();
    let now = Instant::now();
    for (path, approval) in queued {
        if !state.due(&path, now) {
            continue;
        }
        if !participants.contains(&approval.recipient_npub) {
            eprintln!(
                "direct join approval is pending until recipient {} is in the roster",
                approval.recipient_npub
            );
            continue;
        }
        if let Some(route) = approval.fips_route_npub.as_deref()
            && let Err(error) = runtime.ensure_join_approval_route(route).await
        {
            eprintln!("direct FIPS join approval return route is pending: {error}");
            state.record_attempt(path, now);
            continue;
        }
        let delivery_peer = approval
            .fips_route_npub
            .as_deref()
            .unwrap_or(&approval.recipient_npub);
        let mut sent = false;
        for attempt in 0..APPROVAL_INITIAL_BURST_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(APPROVAL_INITIAL_BURST_INTERVAL).await;
            }
            let mut complete_attempt = true;
            for event in &approval.events {
                if let Err(error) = runtime
                    .send_join_approval_event(
                        delivery_peer,
                        approval
                            .fips_route_npub
                            .as_ref()
                            .map(|_| approval.recipient_npub.as_str()),
                        &approval.request_pubkey,
                        event,
                    )
                    .await
                {
                    complete_attempt = false;
                    eprintln!(
                        "direct FIPS join approval to {} is pending: {error}",
                        approval.recipient_npub
                    );
                    break;
                }
            }
            sent |= complete_attempt;
        }
        state.record_attempt(path, now);
        if sent {
            eprintln!(
                "sent join approval directly over FIPS to {}; awaiting durable apply acknowledgment",
                approval.recipient_npub
            );
        }
    }
}
