use super::*;

#[cfg(target_os = "linux")]
const APPROVAL_ACK_FILE_SUFFIX: &str = ".join-approval-ack";

#[cfg(target_os = "linux")]
fn approval_ack_path(config_path: &Path) -> PathBuf {
    let mut name = config_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.toml".into());
    name.push(APPROVAL_ACK_FILE_SUFFIX);
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name)
}

#[cfg(target_os = "linux")]
pub(super) fn persist_approval_ack(
    config_path: &Path,
    datagram: &NostrJoinFipsPubsubDatagram,
) -> Result<()> {
    parse_approval_applied_ack_datagram(datagram)?;
    let path = approval_ack_path(config_path);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("join-approval-ack");
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true).mode(0o600);
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("failed to create {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(&datagram.payload)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)
            .with_context(|| format!("failed to replace {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "linux")]
pub(super) fn load_approval_ack(config_path: &Path) -> Result<Option<NostrJoinFipsPubsubDatagram>> {
    let path = approval_ack_path(config_path);
    let payload = match fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let datagram = NostrJoinFipsPubsubDatagram {
        source_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        destination_port: NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT,
        payload,
    };
    parse_approval_applied_ack_datagram(&datagram)
        .with_context(|| format!("invalid persisted approval ack {}", path.display()))?;
    Ok(Some(datagram))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn write_pairing_uri(path: &Path, uri: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("pairing-uri");
    let temp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temp)
        .with_context(|| format!("failed to create {}", temp.display()))?;
    let write_result = (|| -> Result<()> {
        file.write_all(uri.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
            .with_context(|| format!("failed to replace pairing URI file {}", path.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn remove_pairing_uri(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove pairing URI file {}", path.display())),
    }
}
