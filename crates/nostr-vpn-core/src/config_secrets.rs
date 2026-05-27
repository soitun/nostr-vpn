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

#[cfg(target_os = "ios")]
mod platform {
    use std::path::Path;

    use anyhow::{Context, Result, anyhow};
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::{CFGetTypeID, CFRelease, CFTypeRef};
    use core_foundation_sys::string::CFStringRef;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use security_framework_sys::base::{errSecItemNotFound, errSecSuccess};
    use security_framework_sys::item::{
        kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword, kSecMatchLimit,
        kSecMatchLimitAll, kSecReturnAttributes, kSecReturnData, kSecValueData,
    };
    use security_framework_sys::keychain_item::SecItemCopyMatching;

    use super::{
        ConfigSecret, SERVICE, hydrate_config_secret_fields, scoped_account_name,
        stable_account_name,
    };

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-ios-keychain";
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub(super) fn store_name() -> &'static str {
        "the iOS Keychain"
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
        let account = stable_account_name(kind);
        if let Some(value) = read_account(&account, kind)? {
            return Ok(Some(value));
        }

        let legacy_account = scoped_account_name(path, kind);
        if let Some(value) = read_account(&legacy_account, kind)? {
            migrate_legacy_secret(&account, kind, &value);
            return Ok(Some(value));
        }

        recover_legacy_secret(kind)
    }

    pub(super) fn write_secret(_path: &Path, kind: ConfigSecret, value: &str) -> Result<()> {
        let account = stable_account_name(kind);
        set_generic_password(SERVICE, &account, value.as_bytes())
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!(
                    "failed to write {} to the iOS Keychain",
                    kind.display_name()
                )
            })
    }

    pub(super) fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        let account = stable_account_name(kind);
        delete_account(&account, kind)?;

        let legacy_account = scoped_account_name(path, kind);
        if legacy_account != account {
            delete_account(&legacy_account, kind)?;
        }

        Ok(())
    }

    fn read_account(account: &str, kind: ConfigSecret) -> Result<Option<String>> {
        match get_generic_password(SERVICE, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .with_context(|| {
                    format!(
                        "{} in the iOS Keychain is not valid UTF-8",
                        kind.display_name()
                    )
                })
                .map(Some),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to read {} from the iOS Keychain",
                    kind.display_name()
                )
            }),
        }
    }

    fn delete_account(account: &str, kind: ConfigSecret) -> Result<()> {
        match delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to delete {} from the iOS Keychain",
                    kind.display_name()
                )
            }),
        }
    }

    fn recover_legacy_secret(kind: ConfigSecret) -> Result<Option<String>> {
        let candidates = legacy_secret_candidates(kind)?;
        match candidates.as_slice() {
            [] => Ok(None),
            [(account, value)] => {
                let stable_account = stable_account_name(kind);
                migrate_legacy_secret(&stable_account, kind, value);
                tracing::info!(
                    account,
                    secret = kind.account_suffix(),
                    "recovered iOS Keychain config secret from a legacy account"
                );
                Ok(Some(value.clone()))
            }
            _ => Err(anyhow!(
                "{} has multiple legacy iOS Keychain entries; refusing to guess which one to use",
                kind.display_name()
            )),
        }
    }

    fn legacy_secret_candidates(kind: ConfigSecret) -> Result<Vec<(String, String)>> {
        let suffix = format!(":{}", kind.account_suffix());
        let mut candidates = Vec::new();

        for item in query_service_items()? {
            let Some(account) = keychain_string(&item, unsafe { kSecAttrAccount }) else {
                continue;
            };
            if !account.ends_with(&suffix) {
                continue;
            }
            let Some(data) = keychain_data(&item, unsafe { kSecValueData }) else {
                continue;
            };
            let value = String::from_utf8(data).with_context(|| {
                format!(
                    "{} in a legacy iOS Keychain account is not valid UTF-8",
                    kind.display_name()
                )
            })?;
            candidates.push((account, value));
        }

        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup();
        Ok(candidates)
    }

    fn migrate_legacy_secret(stable_account: &str, kind: ConfigSecret, value: &str) {
        if let Err(error) = set_generic_password(SERVICE, stable_account, value.as_bytes()) {
            tracing::warn!(
                error = ?error,
                secret = kind.account_suffix(),
                "failed to migrate legacy iOS Keychain config secret to stable account"
            );
        }
    }

    fn query_service_items() -> Result<Vec<CFDictionary>> {
        let params = vec![
            (
                unsafe { CFString::wrap_under_get_rule(kSecClass) },
                unsafe { CFString::wrap_under_get_rule(kSecClassGenericPassword) }.into_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kSecAttrService) },
                CFString::new(SERVICE).into_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kSecReturnAttributes) },
                CFBoolean::true_value().into_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kSecReturnData) },
                CFBoolean::true_value().into_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kSecMatchLimit) },
                unsafe { CFString::wrap_under_get_rule(kSecMatchLimitAll) }.into_CFType(),
            ),
        ];
        let params = CFDictionary::from_CFType_pairs(&params);
        let mut ret: CFTypeRef = std::ptr::null();
        let status = unsafe { SecItemCopyMatching(params.as_concrete_TypeRef(), &mut ret) };
        if status == errSecItemNotFound {
            return Ok(Vec::new());
        }
        if status != errSecSuccess {
            return Err(anyhow!(security_framework::base::Error::from_code(status)))
                .context("failed to search iOS Keychain config secrets");
        }
        if ret.is_null() {
            return Ok(Vec::new());
        }

        Ok(unsafe { keychain_search_results(ret) })
    }

    unsafe fn keychain_search_results(ret: CFTypeRef) -> Vec<CFDictionary> {
        let type_id = unsafe { CFGetTypeID(ret) };
        if type_id == CFArray::<CFType>::type_id() {
            let array = unsafe { CFArray::<CFType>::wrap_under_create_rule(ret.cast()) };
            return array
                .iter()
                .filter_map(|item| {
                    if unsafe { CFGetTypeID(item.as_CFTypeRef()) }
                        == CFDictionary::<*const std::ffi::c_void, *const std::ffi::c_void>::type_id()
                    {
                        Some(unsafe {
                            CFDictionary::wrap_under_get_rule(item.as_CFTypeRef().cast())
                        })
                    } else {
                        None
                    }
                })
                .collect();
        }

        if type_id == CFDictionary::<*const std::ffi::c_void, *const std::ffi::c_void>::type_id() {
            return vec![unsafe { CFDictionary::wrap_under_create_rule(ret.cast()) }];
        }

        unsafe { CFRelease(ret) };
        Vec::new()
    }

    fn keychain_string(item: &CFDictionary, key: CFStringRef) -> Option<String> {
        let key_name = unsafe { CFString::wrap_under_get_rule(key) }.to_string();
        let (keys, values) = item.get_keys_and_values();
        for (candidate_key, value) in keys.iter().zip(values.iter()) {
            let candidate_name =
                unsafe { CFString::wrap_under_get_rule((*candidate_key).cast()) }.to_string();
            if candidate_name != key_name {
                continue;
            }
            if unsafe { CFGetTypeID(*value) } == CFString::type_id() {
                return Some(unsafe { CFString::wrap_under_get_rule((*value).cast()) }.to_string());
            }
        }
        None
    }

    fn keychain_data(item: &CFDictionary, key: CFStringRef) -> Option<Vec<u8>> {
        let key_name = unsafe { CFString::wrap_under_get_rule(key) }.to_string();
        let (keys, values) = item.get_keys_and_values();
        for (candidate_key, value) in keys.iter().zip(values.iter()) {
            let candidate_name =
                unsafe { CFString::wrap_under_get_rule((*candidate_key).cast()) }.to_string();
            if candidate_name != key_name {
                continue;
            }
            if unsafe { CFGetTypeID(*value) } == CFData::type_id() {
                let data = unsafe { CFData::wrap_under_get_rule((*value).cast()) };
                return Some(data.bytes().to_vec());
            }
        }
        None
    }
}

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

#[cfg(target_os = "windows")]
mod platform {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result, anyhow};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CryptProtectData, CryptUnprotectData,
    };

    use super::{ConfigSecret, hydrate_config_secret_fields};

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-windows-dpapi";

    pub(super) fn store_name() -> &'static str {
        "a Windows DPAPI-protected sidecar"
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
        let path = secret_path(path, kind);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let plaintext = dpapi_unprotect(&bytes)
            .with_context(|| format!("failed to decrypt {}", path.display()))?;
        String::from_utf8(plaintext)
            .with_context(|| {
                format!(
                    "{} in {} is not valid UTF-8",
                    kind.display_name(),
                    path.display()
                )
            })
            .map(Some)
    }

    pub(super) fn write_secret(path: &Path, kind: ConfigSecret, value: &str) -> Result<()> {
        let secret_path = secret_path(path, kind);
        if let Some(parent) = secret_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let encrypted = dpapi_protect(value.as_bytes())?;
        fs::write(&secret_path, encrypted)
            .with_context(|| format!("failed to write {}", secret_path.display()))
    }

    pub(super) fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        match fs::remove_file(secret_path(path, kind)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to delete Windows secret sidecar"),
        }
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
        parent.join(format!(".{file_name}.{}.dpapi", kind.account_suffix()))
    }

    fn dpapi_protect(plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(plaintext.len()).context("secret is too large for DPAPI")?,
            pbData: plaintext.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_LOCAL_MACHINE,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(anyhow!(std::io::Error::last_os_error()));
        }
        let bytes = unsafe { blob_to_vec_and_free(output) };
        Ok(bytes)
    }

    fn dpapi_unprotect(ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(ciphertext.len()).context("secret is too large for DPAPI")?,
            pbData: ciphertext.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(anyhow!(std::io::Error::last_os_error()));
        }
        let bytes = unsafe { blob_to_vec_and_free(output) };
        Ok(bytes)
    }

    unsafe fn blob_to_vec_and_free(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes =
            unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(blob.pbData.cast());
        }
        bytes
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    use super::{ConfigSecret, hydrate_config_secret_fields};

    pub(super) const REDACTED_SECRET_MARKER: &str = "stored-in-private-secret-file";

    pub(super) fn store_name() -> &'static str {
        "a private Linux secret sidecar"
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
        let path = secret_path(path, kind);
        match fs::read_to_string(&path) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
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
            .with_context(|| format!("failed to write {}", secret_path.display()))
    }

    pub(super) fn delete_secret(path: &Path, kind: ConfigSecret) -> Result<()> {
        match fs::remove_file(secret_path(path, kind)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to delete Linux secret sidecar"),
        }
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
}

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
