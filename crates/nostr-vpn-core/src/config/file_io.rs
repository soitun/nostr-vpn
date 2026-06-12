const MAX_SHARED_ROSTER_FUTURE_SECS: u64 = 600;

fn next_shared_roster_updated_at(previous: u64) -> u64 {
    current_unix_timestamp().max(previous.saturating_add(1))
}

#[cfg(unix)]
fn write_config_file(path: &Path, raw: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let existing_owner = fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.uid(), metadata.gid()));
    let parent_owner = fs::metadata(parent)
        .ok()
        .map(|metadata| (metadata.uid(), metadata.gid()));
    let desired_owner = preferred_config_owner(existing_owner, parent_owner);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let mut temp_path = None;
    let mut temp_file = None;
    for attempt in 0..128u32 {
        let candidate = parent.join(format!(
            ".{file_name}.tmp-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => {
                temp_path = Some(candidate);
                temp_file = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let temp_path = temp_path.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "failed to allocate unique config temp file",
        )
    })?;
    let mut file = temp_file.expect("temp file set with temp path");
    if let Err(error) = file.write_all(raw) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = file.sync_all() {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    drop(file);
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Some((uid, gid)) = desired_owner {
        let metadata = fs::metadata(path)?;
        if metadata.uid() != uid || metadata.gid() != gid {
            match std::os::unix::fs::chown(path, Some(uid), Some(gid)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
                Err(error) => return Err(error),
            }
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(unix)]
fn preferred_config_owner(
    existing_owner: Option<(u32, u32)>,
    parent_owner: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    match (existing_owner, parent_owner) {
        (Some((0, _)), Some((parent_uid, parent_gid))) if parent_uid != 0 => {
            Some((parent_uid, parent_gid))
        }
        (Some(owner), _) => Some(owner),
        (None, Some((parent_uid, parent_gid))) if parent_uid != 0 => Some((parent_uid, parent_gid)),
        (None, _) => None,
    }
}

#[cfg(not(unix))]
fn write_config_file(path: &Path, raw: &[u8]) -> std::io::Result<()> {
    fs::write(path, raw)
}
