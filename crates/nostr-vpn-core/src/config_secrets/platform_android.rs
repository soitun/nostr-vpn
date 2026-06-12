#[cfg(target_os = "android")]
mod platform {
    use std::collections::HashMap;
    use std::path::Path;

    use android_native_keyring_store::Store;
    use anyhow::{Context, Result, anyhow};
    use keyring_core::{Entry, Error as KeyringError, api::CredentialStoreApi};

    use super::{ConfigSecret, SERVICE, hydrate_config_secret_fields, scoped_account_name};

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-android-keystore";

    pub(super) fn store_name() -> &'static str {
        "the Android Keystore"
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
        match entry(path, kind)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to read {} from the Android Keystore",
                    kind.display_name()
                )
            }),
        }
    }

    pub(super) fn write_secret(path: &Path, kind: ConfigSecret, value: &str) -> Result<()> {
        entry(path, kind)?
            .set_password(value)
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!(
                    "failed to write {} to the Android Keystore",
                    kind.display_name()
                )
            })
    }

    pub(super) fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        match entry(path, kind)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to delete {} from the Android Keystore",
                    kind.display_name()
                )
            }),
        }
    }

    fn entry(path: &Path, kind: ConfigSecret) -> Result<Entry> {
        let configuration = HashMap::from([
            ("name", "nostr-vpn-config-secrets"),
            ("filename", "nostr-vpn-config-secrets"),
        ]);
        let store = Store::new_with_configuration(&configuration)
            .map_err(anyhow::Error::from)
            .context("Android context is not initialized for secret storage")?;
        store
            .build(SERVICE, &scoped_account_name(path, kind), None)
            .map_err(anyhow::Error::from)
            .context("failed to create Android Keystore entry")
    }
}
