use std::fs;
use std::path::Path;

use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_pubsub::load_direct_join_approvals;

use crate::fips_private_mesh::FipsPrivateTunnelRuntime;

pub(super) async fn flush_direct_join_approval_outbox(
    runtime: &FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
) {
    let participants = app.participant_pubkeys_hex();
    for (path, queued) in load_direct_join_approvals(config_path) {
        if !participants.contains(&queued.recipient_npub) {
            eprintln!(
                "direct join approval is pending until recipient {} is in the roster",
                queued.recipient_npub
            );
            continue;
        }
        if let Some(route) = queued.fips_route_npub.as_deref()
            && let Err(error) = runtime.ensure_join_approval_route(route).await
        {
            eprintln!("direct FIPS join approval return route is pending: {error}");
            continue;
        }
        let mut sent = true;
        for event in &queued.events {
            if let Err(error) = runtime
                .send_join_approval_event(
                    &queued.recipient_npub,
                    &queued.request_pubkey,
                    event,
                )
                .await
            {
                sent = false;
                eprintln!(
                    "direct FIPS join approval to {} is pending: {error}",
                    queued.recipient_npub
                );
                break;
            }
        }
        if sent {
            if let Err(error) = fs::remove_file(&path) {
                eprintln!(
                    "failed to remove delivered direct join approval {}: {error}",
                    path.display()
                );
            } else {
                eprintln!(
                    "delivered join approval directly over FIPS to {}",
                    queued.recipient_npub
                );
            }
        }
    }
}
