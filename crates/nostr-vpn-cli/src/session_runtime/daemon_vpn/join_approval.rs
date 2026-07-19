use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_delivery::{load_join_rosters, record_join_roster_attempt};
use nostr_vpn_core::join_requests::MAX_NOSTR_JOIN_ROSTER_AGE_SECS;

use crate::fips_private_mesh::FipsJoinApprovalRuntime;
use crate::unix_timestamp;

const JOIN_ROSTER_RETRY_MAX_SECS: u64 = 60 * 60;

pub(super) async fn send_queued_join_rosters_once(
    runtime: &mut Option<FipsJoinApprovalRuntime>,
    app: &AppConfig,
    config_path: &Path,
) {
    let participants = app.participant_pubkeys_hex();
    let now = unix_timestamp();
    let mut latest = HashMap::new();
    for (path, queued) in load_join_rosters(config_path) {
        let signed_at = queued.join_roster.signed_roster.signed_at();
        match latest.entry(queued.recipient_npub.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((path, queued));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let current_signed_at = entry.get().1.join_roster.signed_roster.signed_at();
                if signed_at > current_signed_at {
                    let (superseded_path, _) = entry.insert((path, queued));
                    eprintln!("discarding superseded queued join roster");
                    consume_join_roster(&superseded_path);
                } else {
                    eprintln!("discarding superseded queued join roster");
                    consume_join_roster(&path);
                }
            }
        }
    }
    let mut pending = Vec::new();
    for (path, queued) in latest.into_values() {
        if !participants.contains(&queued.recipient_npub) {
            eprintln!(
                "discarding queued join roster because recipient {} is no longer in the roster",
                queued.recipient_npub
            );
            consume_join_roster(&path);
            continue;
        }
        if now.saturating_sub(queued.join_roster.signed_roster.signed_at())
            > MAX_NOSTR_JOIN_ROSTER_AGE_SECS
        {
            eprintln!("discarding expired queued join roster");
            consume_join_roster(&path);
            continue;
        }
        pending.push((path, queued));
    }
    if pending.is_empty() {
        *runtime = None;
        return;
    }

    let recipients = pending
        .iter()
        .map(|(_, queued)| queued.recipient_npub.clone())
        .collect::<HashSet<_>>();
    let due = pending
        .iter()
        .any(|(_, queued)| join_roster_retry_due(queued.attempts, queued.last_attempt_at, now));
    if !due {
        return;
    }
    if runtime
        .as_ref()
        .is_none_or(|runtime| !runtime.matches_recipients(&recipients))
    {
        match FipsJoinApprovalRuntime::start(app, &recipients).await {
            Ok(started) => *runtime = Some(started),
            Err(error) => {
                eprintln!(
                    "join roster approval endpoint is pending ({error}); retaining delivery for retry"
                );
                for (path, mut queued) in pending {
                    if join_roster_retry_due(queued.attempts, queued.last_attempt_at, now)
                        && let Err(error) = record_join_roster_attempt(&path, &mut queued, now)
                    {
                        eprintln!("failed to persist join roster delivery attempt: {error}");
                    }
                }
                return;
            }
        }
    }

    let mut transport_acknowledged = false;
    for (path, mut queued) in pending {
        if !join_roster_retry_due(queued.attempts, queued.last_attempt_at, now) {
            continue;
        }

        let result = runtime
            .as_ref()
            .expect("approval runtime initialized")
            .send_join_roster(&queued.recipient_npub, queued.join_roster.clone())
            .await;
        if let Err(error) = record_join_roster_attempt(&path, &mut queued, now) {
            eprintln!("failed to persist join roster delivery attempt: {error}");
        }
        match result {
            Ok(()) => {
                transport_acknowledged = true;
                eprintln!(
                    "sent one signed join roster over the unjoined FIPS approval scope to {}; retaining it for interruption-safe retry",
                    queued.recipient_npub
                );
            }
            Err(error) => eprintln!(
                "join roster delivery over FIPS-TCP is pending ({error}); retaining it for retry"
            ),
        }
    }
    if transport_acknowledged {
        // The durable file remains until expiry, but the second endpoint should
        // not compete with the ordinary joined-network endpoint between retries.
        *runtime = None;
    }
}

fn join_roster_retry_due(attempts: u32, last_attempt_at: u64, now: u64) -> bool {
    if attempts == 0 {
        return true;
    }
    let exponent = attempts.min(12);
    let retry_secs = 1_u64
        .checked_shl(exponent)
        .unwrap_or(JOIN_ROSTER_RETRY_MAX_SECS)
        .min(JOIN_ROSTER_RETRY_MAX_SECS);
    now.saturating_sub(last_attempt_at) >= retry_secs
}

fn consume_join_roster(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        eprintln!("failed to remove join roster {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::join_roster_retry_due;

    #[test]
    fn join_roster_retry_backoff_is_immediate_then_bounded() {
        assert!(join_roster_retry_due(0, 0, 100));
        assert!(!join_roster_retry_due(1, 100, 101));
        assert!(join_roster_retry_due(1, 100, 102));
        assert!(!join_roster_retry_due(20, 100, 3_699));
        assert!(join_roster_retry_due(20, 100, 3_700));
    }
}
