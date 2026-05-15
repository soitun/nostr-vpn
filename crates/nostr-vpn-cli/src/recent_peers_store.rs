//! Disk-backed wrapper around [`RecentPeerEndpoints`].
//!
//! Lives in the CLI crate (not in nostr-vpn-core) because file paths and
//! permission bits are CLI-layer concerns; the data model and LAN filter
//! stay in `nostr_vpn_core::recent_peers`.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use nostr_vpn_core::recent_peers::RecentPeerEndpoints;

/// Entries older than this are dropped on load. NAT mappings rarely
/// outlive a week, and stale public IPs become misleading hints — the
/// FIPS retry path will recover via the overlay advert anyway.
pub(crate) const RECENT_PEERS_TTL_SECS: u64 = 7 * 24 * 60 * 60;

pub(crate) fn recent_peers_file_path(config_path: &Path) -> PathBuf {
    let parent = config_path
        .parent()
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from);
    parent.join("daemon.recent-peers.json")
}

pub(crate) fn load_recent_peers(path: &Path, now: u64) -> Result<RecentPeerEndpoints> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(RecentPeerEndpoints::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read recent peers cache {}", path.display()));
        }
    };

    let mut state = match RecentPeerEndpoints::from_json(&raw) {
        Ok(state) => state,
        Err(error) => {
            eprintln!(
                "daemon: discarding unreadable recent peers cache {}: {error}",
                path.display()
            );
            return Ok(RecentPeerEndpoints::default());
        }
    };
    state.prune_stale(now, RECENT_PEERS_TTL_SECS);
    Ok(state)
}

pub(crate) fn write_recent_peers(path: &Path, state: &RecentPeerEndpoints) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = state
        .to_json_pretty()
        .map_err(|error| anyhow::anyhow!("failed to serialize recent peers cache: {error}"))?;
    let mut tmp = path.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("daemon.recent-peers.json"));
    name.push(".tmp");
    tmp.set_file_name(name);

    fs::write(&tmp, raw)
        .with_context(|| format!("failed to write recent peers cache temp {}", tmp.display()))?;
    crate::daemon_runtime::set_private_cache_file_permissions(&tmp)?;

    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "failed to rename recent peers cache {} -> {}: {error}",
                tmp.display(),
                path.display()
            ),
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Single-test scratch directory. RAII drop removes the tree on success
    /// or panic. Mirrors `tempfile::TempDir` minimally to avoid adding a new
    /// dependency for a few file-I/O test cases.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(label: &str) -> Self {
            let now_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let seq = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir()
                .join(format!("nvpn-recent-peers-{label}-{pid}-{now_nanos}-{seq}"));
            fs::create_dir_all(&path).expect("create scratch dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_returns_empty_when_file_missing() {
        let dir = ScratchDir::new("missing");
        let path = dir.path().join("missing.json");
        let state = load_recent_peers(&path, 1_000).unwrap();
        assert!(state.is_empty());
    }

    #[test]
    fn load_discards_garbage_input_without_failing_daemon_startup() {
        let dir = ScratchDir::new("garbage");
        let path = dir.path().join("recent.json");
        fs::write(&path, b"not json at all").unwrap();

        let state = load_recent_peers(&path, 1_000).unwrap();
        assert!(state.is_empty());
    }

    #[test]
    fn load_prunes_entries_past_ttl() {
        let dir = ScratchDir::new("ttl");
        let path = dir.path().join("recent.json");

        let mut state = RecentPeerEndpoints::default();
        let participant = "a".repeat(64);
        state.note_success(&participant, "203.0.113.20:51820", 100);
        write_recent_peers(&path, &state).unwrap();

        let restored = load_recent_peers(&path, 100 + RECENT_PEERS_TTL_SECS + 10).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn write_and_load_round_trip() {
        let dir = ScratchDir::new("round-trip");
        let path = dir.path().join("recent.json");

        let mut state = RecentPeerEndpoints::default();
        let participant = "b".repeat(64);
        state.note_success(&participant, "203.0.113.20:51820", 1_000);
        write_recent_peers(&path, &state).unwrap();

        let restored = load_recent_peers(&path, 1_000).unwrap();
        assert_eq!(
            restored.endpoints_for(&participant),
            vec!["203.0.113.20:51820".to_string()]
        );
    }
}
