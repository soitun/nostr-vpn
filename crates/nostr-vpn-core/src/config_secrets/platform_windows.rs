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
