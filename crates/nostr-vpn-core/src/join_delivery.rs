use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::normalize_nostr_pubkey;
use crate::fips_control::JoinRosterControl;

const JOIN_ROSTER_OUTBOX_VERSION: u8 = 1;
const MAX_QUEUED_JOIN_ROSTERS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedJoinRoster {
    pub version: u8,
    pub recipient_npub: String,
    pub fips_route_npub: Option<String>,
    pub join_roster: JoinRosterControl,
}

pub fn join_roster_outbox_directory(config_path: &Path) -> PathBuf {
    let mut directory_name = config_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.toml".into());
    directory_name.push(".join-roster-outbox");
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(directory_name)
}

pub fn queue_join_roster(
    config_path: &Path,
    recipient_npub: &str,
    fips_route_npub: Option<&str>,
    join_roster: &JoinRosterControl,
) -> Result<PathBuf> {
    let recipient_npub =
        normalize_nostr_pubkey(recipient_npub).context("invalid join roster recipient")?;
    let fips_route_npub = fips_route_npub
        .map(normalize_nostr_pubkey)
        .transpose()
        .context("invalid join roster FIPS route")?;
    join_roster
        .signed_roster
        .verify()
        .context("invalid signed join roster")?;
    let queued = QueuedJoinRoster {
        version: JOIN_ROSTER_OUTBOX_VERSION,
        recipient_npub: recipient_npub.clone(),
        fips_route_npub,
        join_roster: join_roster.clone(),
    };
    let directory = join_roster_outbox_directory(config_path);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let event_id = queued.join_roster.signed_roster.event.id.to_hex();
    let destination = directory.join(format!("{recipient_npub}-{event_id}.json"));
    if destination.exists() {
        return Ok(destination);
    }
    let temporary = directory.join(format!(
        ".{recipient_npub}-{}-{event_id}.tmp",
        std::process::id()
    ));
    let bytes = serde_json::to_vec(&queued).context("failed to encode queued join roster")?;
    write_private_file(&temporary, &bytes)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to queue {}", destination.display()));
    }
    Ok(destination)
}

pub fn load_join_rosters(config_path: &Path) -> Vec<(PathBuf, QueuedJoinRoster)> {
    let directory = join_roster_outbox_directory(config_path);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            eprintln!("failed to scan {}: {error}", directory.display());
            return Vec::new();
        }
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.truncate(MAX_QUEUED_JOIN_ROSTERS);
    paths
        .into_iter()
        .filter_map(|path| {
            match fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<QueuedJoinRoster>(&bytes)
                        .with_context(|| format!("failed to decode {}", path.display()))
                }) {
                Ok(queued)
                    if queued.version == JOIN_ROSTER_OUTBOX_VERSION
                        && queued.join_roster.signed_roster.verify().is_ok()
                        && !queued.join_roster.request_secret.is_empty() =>
                {
                    Some((path, queued))
                }
                Ok(_) => {
                    eprintln!(
                        "discarding unsupported queued join roster {}",
                        path.display()
                    );
                    let _ = fs::remove_file(path);
                    None
                }
                Err(error) => {
                    eprintln!("discarding invalid queued join roster: {error:#}");
                    let _ = fs::remove_file(path);
                    None
                }
            }
        })
        .collect()
}

#[cfg(unix)]
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use nostr_sdk::Keys;

    use super::*;
    use crate::fips_control::{JoinRosterControl, NetworkRoster, SignedRoster};

    #[test]
    fn outbox_stores_exactly_one_signed_roster() {
        let admin = Keys::generate();
        let recipient = Keys::generate().public_key().to_hex();
        let roster = SignedRoster::sign(
            "network",
            NetworkRoster {
                network_name: "Home".to_string(),
                devices: vec![recipient.clone()],
                admins: vec![admin.public_key().to_hex()],
                aliases: HashMap::new(),
                signed_at: 100,
            },
            &admin,
        )
        .expect("sign roster");
        let join_roster =
            JoinRosterControl::new(roster.clone(), "request-secret").expect("join control");
        let config_path = std::env::temp_dir().join(format!(
            "nvpn-join-roster-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));

        let path = queue_join_roster(&config_path, &recipient, None, &join_roster)
            .expect("queue signed roster");
        let queued = load_join_rosters(&config_path);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].1.join_roster.signed_roster, roster);
        assert_eq!(queued[0].1.join_roster.request_secret, "request-secret");
        assert_eq!(queued[0].1.recipient_npub, recipient);

        fs::remove_file(path).expect("remove queued roster");
        fs::remove_dir(join_roster_outbox_directory(&config_path)).expect("remove outbox");
    }
}
