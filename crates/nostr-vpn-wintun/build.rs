use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Ok(source) = find_wintun_dll() else {
        println!("cargo:warning=failed to locate wintun.dll in cargo registry source");
        return;
    };

    println!("cargo:rerun-if-changed={}", source.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let profile = env::var("PROFILE").expect("PROFILE");
    let Some(target_dir) = out_dir
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new(&profile)))
        .map(Path::to_path_buf)
    else {
        println!("cargo:warning=failed to derive target profile dir from OUT_DIR");
        return;
    };

    let target_dll = target_dir.join("wintun.dll");
    let already_current = files_identical(&source, &target_dll).unwrap_or(false);
    if !already_current {
        fs::copy(&source, &target_dll).unwrap_or_else(|error| {
            panic!(
                "failed to copy wintun.dll from {} to {}: {error}",
                source.display(),
                target_dll.display()
            )
        });
    }

    println!(
        "cargo:rustc-env=NOSTR_VPN_WINTUN_DLL_SOURCE={}",
        target_dll.display()
    );
}

fn files_identical(left: &Path, right: &Path) -> io::Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    Ok(fs::read(left)? == fs::read(right)?)
}

fn find_wintun_dll() -> Result<PathBuf, String> {
    let arch_dir = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("x86_64") => "amd64",
        Ok("arm") => "arm",
        Ok("aarch64") => "arm64",
        Ok(other) => return Err(format!("unsupported windows arch {other}")),
        Err(error) => return Err(error.to_string()),
    };

    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(default_cargo_home)
        .ok_or_else(|| "unable to resolve cargo home".to_string())?;
    let registry_src = cargo_home.join("registry").join("src");
    let registries = fs::read_dir(&registry_src).map_err(|error| {
        format!(
            "failed to read cargo registry source root {}: {error}",
            registry_src.display()
        )
    })?;

    for registry in registries {
        let registry = registry.map_err(|error| error.to_string())?;
        let packages = fs::read_dir(registry.path()).map_err(|error| error.to_string())?;
        for package in packages {
            let package = package.map_err(|error| error.to_string())?;
            let name = package.file_name();
            if !name.to_string_lossy().starts_with("wintun-") {
                continue;
            }

            let candidate = package
                .path()
                .join("wintun")
                .join("bin")
                .join(arch_dir)
                .join("wintun.dll");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(format!(
        "wintun.dll for architecture {arch_dir} not found under {}",
        registry_src.display()
    ))
}

fn default_cargo_home() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .map(|home| home.join(".cargo"))
}
