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

        finish_join_roster_delivery(&path, &queued.recipient_npub, delivery);
    }
}

fn finish_join_roster_delivery(path: &Path, recipient: &str, delivery: anyhow::Result<()>) {
    match delivery {
        Ok(()) => {
            consume_join_roster(path);
            eprintln!(
                "delivered and applied one signed join roster over FIPS-TCP to {}",
                recipient
            );
        }
        Err(error) => eprintln!(
            "join roster was not durably applied over FIPS-TCP ({error}); retaining it for retry"
        ),
    }
}

fn consume_join_roster(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        eprintln!("failed to remove join roster {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_join_roster_delivery_keeps_outbox_file_for_retry() {
        let path = std::env::temp_dir().join(format!(
            "nvpn-join-roster-retry-{}-{}",
            std::process::id(),
            crate::unix_timestamp()
        ));
        fs::write(&path, b"queued").expect("write queued roster");

        finish_join_roster_delivery(&path, "recipient", Err(anyhow::anyhow!("offline")));
        assert!(path.exists(), "failed delivery must retain the outbox file");

        finish_join_roster_delivery(&path, "recipient", Ok(()));
        assert!(
            !path.exists(),
            "durable receipt may consume the outbox file"
        );
    }
}
