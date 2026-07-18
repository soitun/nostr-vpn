use std::fs;
use std::path::Path;

use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_delivery::load_join_rosters;

use crate::fips_private_mesh::FipsPrivateTunnelRuntime;

pub(super) async fn send_queued_join_rosters_once(
    runtime: &mut FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
) {
    let participants = app.participant_pubkeys_hex();
    for (path, queued) in load_join_rosters(config_path) {
        let delivery = if !participants.contains(&queued.recipient_npub) {
            Err(anyhow::anyhow!(
                "recipient {} is not in the roster",
                queued.recipient_npub
            ))
        } else {
            runtime
                .send_join_roster(&queued.recipient_npub, queued.join_roster)
                .await
        };

        consume_join_roster(&path);
        match delivery {
            Ok(()) => eprintln!(
                "sent one signed join roster over FIPS-TCP to {}",
                queued.recipient_npub
            ),
            Err(error) => eprintln!(
                "join roster was not delivered over FIPS-TCP ({error}); the joiner must request again"
            ),
        }
    }
}

fn consume_join_roster(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        eprintln!("failed to remove join roster {}: {error}", path.display());
    }
}
