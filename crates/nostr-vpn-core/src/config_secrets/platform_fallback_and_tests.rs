#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "windows",
    target_os = "linux"
)))]
mod platform {
    use std::path::Path;

    use anyhow::Result;

    use super::{ConfigSecret, hydrate_config_secret_fields};

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-private-secret-file";

    pub(super) fn store_name() -> &'static str {
        "the platform secret store"
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

    pub(super) fn read_secret(_path: &Path, _kind: ConfigSecret) -> Result<Option<String>> {
        Ok(None)
    }

    pub(super) fn write_secret(_path: &Path, _kind: ConfigSecret, _value: &str) -> Result<()> {
        anyhow::bail!("platform secret storage is not available")
    }

    pub(super) fn delete_secret(_path: &Path, _kind: ConfigSecret) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::is_redacted_secret;

    #[test]
    fn recognizes_all_secret_markers() {
        for marker in [
            "stored-in-ios-keychain",
            "stored-in-android-keystore",
            "stored-in-windows-dpapi",
            "stored-in-private-secret-file",
        ] {
            assert!(is_redacted_secret(marker));
        }
    }
}
