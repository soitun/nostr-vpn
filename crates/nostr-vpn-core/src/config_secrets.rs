use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
#[cfg(any(target_os = "ios", target_os = "android"))]
use sha2::{Digest as _, Sha256};

use crate::config::{AppConfig, normalize_nostr_pubkey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretPersistence {
    Platform,
    Plaintext,
}

pub(crate) fn hydrate_config_secrets(path: &Path, config: &mut AppConfig) -> Result<()> {
    platform::hydrate_config_secrets(path, config)
}

pub(crate) fn prepare_config_secrets_for_save(
    path: &Path,
    config: &mut AppConfig,
    persistence: SecretPersistence,
) -> Result<()> {
    if persistence == SecretPersistence::Plaintext {
        return Ok(());
    }
    persist_field(path, ConfigSecret::Nostr, &mut config.nostr.secret_key)?;
    persist_field(
        path,
        ConfigSecret::WireGuardExitPrivate,
        &mut config.wireguard_exit.private_key,
    )?;
    persist_field(
        path,
        ConfigSecret::WireGuardExitPeerPreshared,
        &mut config.wireguard_exit.peer_preshared_key,
    )
}

pub(crate) fn delete_config_secrets(path: &Path) -> Result<()> {
    let mut result = Ok(());
    for kind in [
        ConfigSecret::Nostr,
        ConfigSecret::WireGuardExitPrivate,
        ConfigSecret::WireGuardExitPeerPreshared,
    ] {
        if let Err(error) = platform::delete_secret(path, kind) {
            result = Err(error);
        }
    }
    result
}

pub(crate) fn config_file_needs_secret_migration(path: &Path) -> Result<bool> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value: toml::Value = toml::from_str(&raw).context("failed to parse config TOML")?;

    if nostr_secret_needs_migration(&value) {
        return Ok(true);
    }

    for field in ["private_key", "peer_preshared_key"] {
        if plaintext_secret_field(&value, "wireguard_exit", field) {
            return Ok(true);
        }
    }

    Ok(false)
}

#[derive(Debug, Clone, Copy)]
enum ConfigSecret {
    Nostr,
    WireGuardExitPrivate,
    WireGuardExitPeerPreshared,
}

impl ConfigSecret {
    fn account_suffix(self) -> &'static str {
        match self {
            Self::Nostr => "nostr-secret-key",
            Self::WireGuardExitPrivate => "wireguard-exit-private-key",
            Self::WireGuardExitPeerPreshared => "wireguard-exit-peer-preshared-key",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Nostr => "Nostr secret key",
            Self::WireGuardExitPrivate => "WireGuard exit private key",
            Self::WireGuardExitPeerPreshared => "WireGuard exit peer preshared key",
        }
    }
}

const REDACTED_SECRET_MARKERS: &[&str] = &[
    "stored-in-ios-keychain",
    "stored-in-android-keystore",
    "stored-in-windows-dpapi",
    "stored-in-private-secret-file",
];
#[cfg(any(target_os = "ios", target_os = "android"))]
const SERVICE: &str = "to.nostrvpn.nvpn.config-secrets";

fn hydrate_config_secret_fields(path: &Path, config: &mut AppConfig) -> Result<()> {
    if is_redacted_secret(&config.nostr.secret_key) {
        config.nostr.secret_key = read_required_secret(path, ConfigSecret::Nostr)?;
    } else if config.nostr.secret_key.trim().is_empty()
        && normalize_nostr_pubkey(&config.nostr.public_key).is_ok()
        && let Some(value) = platform::read_secret(path, ConfigSecret::Nostr)?
    {
        config.nostr.secret_key = value;
    }
    if is_redacted_secret(&config.wireguard_exit.private_key) {
        config.wireguard_exit.private_key =
            read_required_secret(path, ConfigSecret::WireGuardExitPrivate)?;
    }
    if is_redacted_secret(&config.wireguard_exit.peer_preshared_key) {
        config.wireguard_exit.peer_preshared_key =
            read_required_secret(path, ConfigSecret::WireGuardExitPeerPreshared)?;
    }

    if config.nostr.secret_key.trim().is_empty()
        && normalize_nostr_pubkey(&config.nostr.public_key).is_ok()
    {
        return Err(anyhow!(
            "config {} references a Nostr public key but its secret key is missing from {}",
            path.display(),
            platform::store_name()
        ));
    }

    Ok(())
}

fn persist_field(path: &Path, kind: ConfigSecret, value: &mut String) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        let _ = platform::delete_secret(path, kind);
        return Ok(());
    }
    if is_redacted_secret(trimmed) {
        return Ok(());
    }
    if trimmed.starts_with("stored-in-") {
        return Err(anyhow!(
            "{} uses unsupported secret storage marker {trimmed:?}",
            kind.display_name()
        ));
    }

    let secret = trimmed.to_string();
    match platform::write_secret(path, kind, &secret) {
        Ok(()) => {
            *value = platform::REDACTED_SECRET_MARKER.to_string();
            Ok(())
        }
        Err(write_error) => match platform::read_secret(path, kind) {
            Ok(Some(existing)) if existing == secret => {
                *value = platform::REDACTED_SECRET_MARKER.to_string();
                Ok(())
            }
            Ok(Some(_)) => Err(write_error).with_context(|| {
                format!(
                    "{} changed but updating {} failed",
                    kind.display_name(),
                    platform::store_name()
                )
            }),
            Ok(None) | Err(_) if platform::allows_plaintext_fallback() => {
                *value = secret;
                Ok(())
            }
            Ok(None) | Err(_) => Err(write_error).with_context(|| {
                format!(
                    "failed to store {} in {}; refusing to write it to the config file",
                    kind.display_name(),
                    platform::store_name()
                )
            }),
        },
    }
}

fn read_required_secret(path: &Path, kind: ConfigSecret) -> Result<String> {
    platform::read_secret(path, kind)?.ok_or_else(|| {
        anyhow!(
            "{} is marked as stored in {}, but no matching secret exists",
            kind.display_name(),
            platform::store_name()
        )
    })
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn scoped_account_name(path: &Path, kind: ConfigSecret) -> String {
    format!("{}:{}", config_scope(path), kind.account_suffix())
}

#[cfg(target_os = "ios")]
fn stable_account_name(kind: ConfigSecret) -> String {
    kind.account_suffix().to_string()
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn config_scope(path: &Path) -> String {
    let canonical = canonical_config_path(path);
    let mut hasher = Sha256::new();
    hasher.update(config_path_bytes(&canonical));
    hex::encode(hasher.finalize())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn canonical_config_path(path: &Path) -> std::path::PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name())
        && let Ok(parent) = fs::canonicalize(parent)
    {
        return parent.join(file_name);
    }
    path.to_path_buf()
}

#[cfg(all(unix, any(target_os = "ios", target_os = "android")))]
fn config_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(all(windows, any(target_os = "ios", target_os = "android")))]
fn config_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(all(not(any(unix, windows)), any(target_os = "ios", target_os = "android")))]
fn config_path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

fn is_redacted_secret(value: &str) -> bool {
    let value = value.trim();
    REDACTED_SECRET_MARKERS.contains(&value)
}

fn nostr_secret_needs_migration(value: &toml::Value) -> bool {
    let Some(nostr) = value.get("nostr").and_then(toml::Value::as_table) else {
        return true;
    };

    let secret_key = nostr
        .get("secret_key")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .trim();
    let public_key = nostr
        .get("public_key")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .trim();

    secret_key.is_empty() || public_key.is_empty() || !is_redacted_secret(secret_key)
}

fn plaintext_secret_field(value: &toml::Value, table: &str, field: &str) -> bool {
    value
        .get(table)
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get(field))
        .and_then(toml::Value::as_str)
        .is_some_and(|value| {
            let value = value.trim();
            !value.is_empty() && !is_redacted_secret(value)
        })
}

include!("config_secrets/platform_macos.rs");
include!("config_secrets/platform_ios.rs");
include!("config_secrets/platform_android.rs");
include!("config_secrets/platform_windows.rs");
include!("config_secrets/platform_linux.rs");
include!("config_secrets/platform_fallback_and_tests.rs");
