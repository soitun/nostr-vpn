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
