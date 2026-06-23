#[cfg(target_os = "macos")]
mod platform {
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result, anyhow};

    use super::{ConfigSecret, hydrate_config_secret_fields};

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-private-secret-file";

    pub(super) fn store_name() -> &'static str {
        "a private macOS secret sidecar"
    }

    pub(super) fn allows_plaintext_fallback() -> bool {
        false
    }

    pub(super) fn hydrate_config_secrets(
        path: &Path,
        config: &mut crate::config::AppConfig,
    ) -> Result<()> {
        hydrate_config_secret_fields(path, config)
    }

    pub(super) fn read_secret(path: &Path, kind: ConfigSecret) -> Result<Option<String>> {
        let secret_path = secret_path(path, kind);
        let Some(mut file) = open_secret_for_read(&secret_path)? else {
            return Ok(None);
        };
        let mut value = String::new();
        file.read_to_string(&mut value)
            .with_context(|| format!("failed to read {}", secret_path.display()))?;
        Ok(Some(value))
    }

    pub(super) fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        let secret_path = secret_path(path, kind);
        match fs::remove_file(&secret_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to delete {}", secret_path.display()))
            }
        }
    }

    pub(super) fn write_secret(path: &Path, kind: ConfigSecret, value: &str) -> Result<()> {
        let secret_path = secret_path(path, kind);
        if let Some(parent) = secret_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut file = open_secret_for_write(&secret_path)?;
        file.write_all(value.as_bytes())
            .with_context(|| format!("failed to write {}", secret_path.display()))?;
        set_secret_file_owner(path, &file)?;
        Ok(())
    }

    fn secret_path(path: &Path, kind: ConfigSecret) -> PathBuf {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("config.toml");
        parent.join(format!(".{file_name}.{}.secret", kind.account_suffix()))
    }

    fn set_secret_file_owner(config_path: &Path, secret_file: &File) -> Result<()> {
        if current_euid() != 0 {
            return Ok(());
        }

        let Some((uid, gid)) = preferred_secret_owner(config_path, secret_file) else {
            return Ok(());
        };
        fchown_file(secret_file, uid, gid)
    }

    fn preferred_secret_owner(config_path: &Path, secret_file: &File) -> Option<(u32, u32)> {
        owner(config_path)
            .filter(|(uid, _)| *uid != 0)
            .or_else(|| {
                config_path
                    .parent()
                    .and_then(owner)
                    .filter(|(uid, _)| *uid != 0)
            })
            .or_else(|| owner_file(secret_file))
    }

    fn owner(path: &Path) -> Option<(u32, u32)> {
        fs::metadata(path)
            .ok()
            .map(|metadata| (metadata.uid(), metadata.gid()))
    }

    fn fchown_file(file: &File, uid: u32, gid: u32) -> Result<()> {
        let rc = unsafe { fchown(file.as_raw_fd(), uid, gid) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error()).context("failed to chown macOS secret sidecar")
        }
    }

    fn owner_file(file: &File) -> Option<(u32, u32)> {
        file.metadata()
            .ok()
            .map(|metadata| (metadata.uid(), metadata.gid()))
    }

    fn open_secret_for_read(path: &Path) -> Result<Option<File>> {
        match validate_existing_secret_path(path)? {
            false => Ok(None),
            true => OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map(Some)
                .with_context(|| format!("failed to open {}", path.display())),
        }
    }

    fn open_secret_for_write(path: &Path) -> Result<File> {
        validate_existing_secret_path(path)?;
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))
    }

    fn validate_existing_secret_path(path: &Path) -> Result<bool> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(anyhow!(
                "refusing to use non-regular macOS secret sidecar {}",
                path.display()
            ));
        }
        Ok(true)
    }

    fn current_euid() -> u32 {
        unsafe { geteuid() }
    }

    unsafe extern "C" {
        fn fchown(fd: i32, owner: u32, group: u32) -> i32;
        fn geteuid() -> u32;
    }
}
