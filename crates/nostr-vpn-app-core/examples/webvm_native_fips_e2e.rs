use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nostr_vpn_app_core::{FfiApp, NativeAppAction, NativeAppState};
use serde_json::json;

const REAL_E2E_GUARD: &str = "NVPN_WEBVM_REAL_E2E";
const IMPORT_COMMAND_PREFIX: &str = "import ";

#[derive(Debug)]
struct Args {
    config_path: PathBuf,
    nvpn_bin: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct HarnessError {
    stage: &'static str,
    code: &'static str,
}

type HarnessResult<T> = Result<T, HarnessError>;

impl HarnessError {
    const fn new(stage: &'static str, code: &'static str) -> Self {
        Self { stage, code }
    }
}

impl Args {
    fn parse() -> HarnessResult<Self> {
        let mut config_path = None;
        let mut nvpn_bin = None;
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--config-path" => {
                    config_path = Some(required_arg_value(&mut args)?);
                }
                "--nvpn-bin" => {
                    nvpn_bin = Some(required_arg_value(&mut args)?);
                }
                _ => return Err(HarnessError::new("arguments", "invalid-argument")),
            }
        }

        let config_path =
            config_path.ok_or_else(|| HarnessError::new("arguments", "missing-config-path"))?;
        let nvpn_bin =
            nvpn_bin.ok_or_else(|| HarnessError::new("arguments", "missing-nvpn-bin"))?;
        Ok(Self {
            config_path: canonical_file(&config_path, false)?,
            nvpn_bin: canonical_file(&nvpn_bin, true)?,
        })
    }
}

fn required_arg_value(args: &mut impl Iterator<Item = String>) -> HarnessResult<PathBuf> {
    args.next()
        .filter(|value| !value.starts_with("--") && !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::new("arguments", "missing-argument-value"))
}

fn canonical_file(path: &Path, require_executable: bool) -> HarnessResult<PathBuf> {
    let canonical =
        fs::canonicalize(path).map_err(|_| HarnessError::new("preflight", "path-unavailable"))?;
    let metadata =
        fs::metadata(&canonical).map_err(|_| HarnessError::new("preflight", "path-unavailable"))?;
    if !metadata.is_file() {
        return Err(HarnessError::new("preflight", "path-is-not-file"));
    }
    #[cfg(unix)]
    if require_executable && metadata.permissions().mode() & 0o111 == 0 {
        return Err(HarnessError::new("preflight", "nvpn-binary-not-executable"));
    }
    #[cfg(not(unix))]
    let _ = require_executable;
    Ok(canonical)
}

fn emit_status(value: &serde_json::Value) -> HarnessResult<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, &value)
        .map_err(|_| HarnessError::new("output", "json-write-failed"))?;
    writeln!(output).map_err(|_| HarnessError::new("output", "stdout-write-failed"))?;
    output
        .flush()
        .map_err(|_| HarnessError::new("output", "stdout-flush-failed"))
}

fn read_command(input: &mut impl BufRead) -> HarnessResult<Option<String>> {
    let mut command = String::new();
    let bytes = input
        .read_line(&mut command)
        .map_err(|_| HarnessError::new("protocol", "stdin-read-failed"))?;
    if bytes == 0 {
        return Ok(None);
    }
    while matches!(command.as_bytes().last(), Some(b'\n' | b'\r')) {
        command.pop();
    }
    Ok(Some(command))
}

fn participant_keys(state: &NativeAppState) -> HashSet<(String, String)> {
    state
        .networks
        .iter()
        .flat_map(|network| {
            network
                .participants
                .iter()
                .map(|participant| (network.id.clone(), participant.npub.clone()))
        })
        .collect()
}

fn added_participant(
    before: &NativeAppState,
    after: &NativeAppState,
) -> HarnessResult<(String, String)> {
    let before_keys = participant_keys(before);
    let mut added = participant_keys(after)
        .into_iter()
        .filter(|key| !before_keys.contains(key));
    let participant = added
        .next()
        .ok_or_else(|| HarnessError::new("import", "participant-not-added"))?;
    if added.next().is_some() {
        return Err(HarnessError::new("import", "ambiguous-participant-change"));
    }
    Ok(participant)
}

fn participant_exists(state: &NativeAppState, network_id: &str, npub: &str) -> bool {
    state.networks.iter().any(|network| {
        network.id == network_id
            && network
                .participants
                .iter()
                .any(|participant| participant.npub == npub)
    })
}

fn cleanup(
    app: &FfiApp,
    participant: Option<&(String, String)>,
    restore_disconnected: bool,
) -> HarnessResult<()> {
    let mut participant_removed = false;
    if let Some((network_id, npub)) = participant {
        let state = app.dispatch(NativeAppAction::RemoveParticipant {
            network_id: network_id.clone(),
            npub: npub.clone(),
        });
        if !state.error.is_empty() || participant_exists(&state, network_id, npub) {
            return Err(HarnessError::new("cleanup", "participant-remove-failed"));
        }
        participant_removed = true;
    }

    let mut vpn_restored = false;
    let state = app.state();
    if restore_disconnected && (state.vpn_enabled || state.vpn_active) {
        let state = app.dispatch(NativeAppAction::DisconnectVpn);
        if !state.error.is_empty() {
            return Err(HarnessError::new("cleanup", "vpn-restore-failed"));
        }
        vpn_restored = true;
    }

    emit_status(&json!({
        "status": "cleaned",
        "participantRemoved": participant_removed,
        "vpnRestored": vpn_restored,
    }))
}

fn run() -> HarnessResult<()> {
    if env::var(REAL_E2E_GUARD).as_deref() != Ok("1") {
        return Err(HarnessError::new("guard", "real-e2e-not-enabled"));
    }

    let args = Args::parse()?;
    let app = FfiApp::new_with_config_path(
        args.config_path,
        env!("CARGO_PKG_VERSION").to_string(),
        Some(args.nvpn_bin),
    );
    let before = app.state();
    if !before.error.is_empty() {
        return Err(HarnessError::new("startup", "ffi-app-startup-failed"));
    }
    if !before
        .networks
        .iter()
        .any(|network| network.enabled && network.local_is_admin)
    {
        return Err(HarnessError::new(
            "preflight",
            "active-admin-network-required",
        ));
    }
    let restore_disconnected = !before.vpn_enabled && !before.vpn_active;
    emit_status(&json!({ "status": "ready" }))?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let Some(mut command) = read_command(&mut input)? else {
        return cleanup(&app, None, false);
    };
    if command == "cleanup" {
        return cleanup(&app, None, false);
    }
    let mut request = command
        .strip_prefix(IMPORT_COMMAND_PREFIX)
        .ok_or_else(|| HarnessError::new("protocol", "import-command-required"))?
        .to_string();
    command.clear();
    if !request.starts_with("nvpn://join-request/") || request.chars().any(char::is_whitespace) {
        request.clear();
        return Err(HarnessError::new("import", "invalid-join-request"));
    }

    let after = app.dispatch(NativeAppAction::ImportJoinRequest {
        request: std::mem::take(&mut request),
    });
    let participant = added_participant(&before, &after)?;
    if !after.error.is_empty() {
        cleanup(&app, Some(&participant), restore_disconnected)?;
        return Err(HarnessError::new("import", "ffi-import-failed"));
    }
    emit_status(&json!({
        "status": "imported",
        "participantAdded": true,
    }))?;

    match read_command(&mut input)? {
        None => cleanup(&app, Some(&participant), restore_disconnected),
        Some(command) if command == "cleanup" => {
            cleanup(&app, Some(&participant), restore_disconnected)
        }
        Some(_) => {
            cleanup(&app, Some(&participant), restore_disconnected)?;
            Err(HarnessError::new("protocol", "unexpected-command"))
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = emit_status(&json!({
                "status": "error",
                "stage": error.stage,
                "code": error.code,
            }));
            ExitCode::FAILURE
        }
    }
}
