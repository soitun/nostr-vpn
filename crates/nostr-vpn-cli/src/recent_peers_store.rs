//! Disk-backed wrapper around [`RecentPeerEndpoints`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fips_endpoint::{RecentPeersFileError, RecentPeersFileStore};

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

pub(crate) fn load_recent_peers(
    path: &Path,
    local_npub: &str,
    scope: &str,
    now: u64,
) -> Result<RecentPeerEndpoints> {
    let store = RecentPeersFileStore::new(path, local_npub, scope)
        .with_context(|| format!("invalid recent peers cache context for {}", path.display()))?;
    let mut state = match store.load() {
        Ok(recent_peers) => RecentPeerEndpoints::from_recent_peers(recent_peers),
        Err(RecentPeersFileError::Model { source, .. }) => {
            eprintln!(
                "daemon: discarding unreadable recent peers cache {}: {error}",
                path.display(),
                error = source,
            );
            RecentPeerEndpoints::new(local_npub, scope).with_context(|| {
                format!("invalid recent peers cache context for {}", path.display())
            })?
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read recent peers cache {}", path.display()));
        }
    };
    state.prune_stale(now, RECENT_PEERS_TTL_SECS);
    Ok(state)
}

pub(crate) fn write_recent_peers(path: &Path, state: &RecentPeerEndpoints) -> Result<()> {
    RecentPeersFileStore::new(path, state.local_npub(), state.scope())
        .with_context(|| format!("invalid recent peers cache context for {}", path.display()))?
        .save(state.as_recent_peers())
        .with_context(|| format!("failed to write recent peers cache {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fips_core::Identity;
    use nostr_vpn_core::recent_peers::recent_peers_scope;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn context() -> (String, String) {
        (
            Identity::generate().npub(),
            recent_peers_scope("store-tests"),
        )
    }

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
        let (local, scope) = context();
        let state = load_recent_peers(&path, &local, &scope, 1_000).unwrap();
        assert!(state.is_empty());
        assert_eq!(state.local_npub(), local);
        assert_eq!(state.scope(), scope);
    }

    #[test]
    fn load_discards_garbage_input_without_failing_daemon_startup() {
        let dir = ScratchDir::new("garbage");
        let path = dir.path().join("recent.json");
        fs::write(&path, b"not json at all").unwrap();
        let (local, scope) = context();

        let state = load_recent_peers(&path, &local, &scope, 1_000).unwrap();
        assert!(state.is_empty());
    }

    #[test]
    fn load_prunes_entries_past_ttl() {
        let dir = ScratchDir::new("ttl");
        let path = dir.path().join("recent.json");

        let (local, scope) = context();
        let mut state = RecentPeerEndpoints::new(&local, &scope).unwrap();
        let participant = Identity::generate().npub();
        state.note_success(&participant, "203.0.113.20:51820", 100);
        write_recent_peers(&path, &state).unwrap();

        let restored =
            load_recent_peers(&path, &local, &scope, 100 + RECENT_PEERS_TTL_SECS + 10).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn write_and_load_round_trip() {
        let dir = ScratchDir::new("round-trip");
        let path = dir.path().join("recent.json");

        let (local, scope) = context();
        let mut state = RecentPeerEndpoints::new(&local, &scope).unwrap();
        let participant = Identity::generate().npub();
        state.note_success(&participant, "203.0.113.20:51820", 1_000);
        write_recent_peers(&path, &state).unwrap();

        let restored = load_recent_peers(&path, &local, &scope, 1_000).unwrap();
        assert_eq!(
            restored.endpoints_for(&participant),
            vec!["203.0.113.20:51820".to_string()]
        );
    }
}
