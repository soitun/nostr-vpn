#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::CString;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

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
        match fs::read_to_string(&secret_path) {
            Ok(value) => return Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", secret_path.display()));
            }
        }

        Ok(None)
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
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .with_context(|| format!("failed to open {}", secret_path.display()))?;
        file.write_all(value.as_bytes())
            .with_context(|| format!("failed to write {}", secret_path.display()))?;
        set_secret_file_owner(path, &secret_path)?;
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

    fn set_secret_file_owner(config_path: &Path, secret_path: &Path) -> Result<()> {
        if current_euid() != 0 {
            return Ok(());
        }

        let Some((uid, gid)) = preferred_secret_owner(config_path, secret_path) else {
            return Ok(());
        };
        chown_path(secret_path, uid, gid)
    }

    fn preferred_secret_owner(config_path: &Path, secret_path: &Path) -> Option<(u32, u32)> {
        owner(config_path)
            .filter(|(uid, _)| *uid != 0)
            .or_else(|| {
                config_path
                    .parent()
                    .and_then(owner)
                    .filter(|(uid, _)| *uid != 0)
            })
            .or_else(|| owner(secret_path))
    }

    fn owner(path: &Path) -> Option<(u32, u32)> {
        fs::metadata(path)
            .ok()
            .map(|metadata| (metadata.uid(), metadata.gid()))
    }

    fn chown_path(path: &Path, uid: u32, gid: u32) -> Result<()> {
        let raw = CString::new(path.as_os_str().as_bytes())
            .with_context(|| format!("path contains an interior NUL: {}", path.display()))?;
        let rc = unsafe { chown(raw.as_ptr(), uid, gid) };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to chown {}", path.display()))
        }
    }

    fn current_euid() -> u32 {
        unsafe { geteuid() }
    }

    unsafe extern "C" {
        fn chown(path: *const std::ffi::c_char, owner: u32, group: u32) -> i32;
        fn geteuid() -> u32;
    }
}
