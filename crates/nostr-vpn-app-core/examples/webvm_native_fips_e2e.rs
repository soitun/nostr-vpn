use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use nostr_vpn_app_core::{FfiApp, NativeAppAction, NativeAppState};
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY;
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

#[derive(Debug)]
struct HostRestore {
    config_path: PathBuf,
    journal_path: PathBuf,
    nvpn_bin: PathBuf,
}

impl HostRestore {
    fn new(args: &Args) -> Self {
        let mut journal_name = args
            .config_path
            .file_name()
            .unwrap_or_default()
            .to_os_string();
        journal_name.push(".webvm-e2e-restore");
        Self {
            config_path: args.config_path.clone(),
            journal_path: args.config_path.with_file_name(journal_name),
            nvpn_bin: args.nvpn_bin.clone(),
        }
    }
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

fn reload_daemon(restore: &HostRestore) -> bool {
    Command::new(&restore.nvpn_bin)
        .args(["reload", "--config"])
        .arg(&restore.config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn write_restore_journal(restore: &HostRestore) -> HarnessResult<()> {
    if restore.journal_path.exists() {
        return Err(HarnessError::new("preflight", "stale-restore-journal"));
    }
    fs::copy(&restore.config_path, &restore.journal_path)
        .map_err(|_| HarnessError::new("preflight", "restore-journal-write-failed"))?;
    #[cfg(unix)]
    fs::set_permissions(&restore.journal_path, fs::Permissions::from_mode(0o600))
        .map_err(|_| HarnessError::new("preflight", "restore-journal-permissions-failed"))?;
    Ok(())
}

fn restore_host(restore: &HostRestore) -> bool {
    if !restore.journal_path.is_file() {
        return true;
    }
    let mut temporary_path = restore.config_path.clone();
    temporary_path.set_extension("toml.webvm-e2e-restoring");
    if fs::copy(&restore.journal_path, &temporary_path).is_err()
        || fs::rename(&temporary_path, &restore.config_path).is_err()
        || !reload_daemon(restore)
    {
        let _ = fs::remove_file(&temporary_path);
        return false;
    }
    fs::remove_file(&restore.journal_path).is_ok()
}

fn cleanup(participant: Option<&(String, String)>, restore: &HostRestore) -> HarnessResult<()> {
    if !restore_host(restore) {
        return Err(HarnessError::new("cleanup", "host-restore-failed"));
    }

    emit_status(&json!({
        "status": "cleaned",
        "participantRemoved": participant.is_some(),
        "hostConfigRestored": true,
    }))
}

fn prepare_host_config(restore: &HostRestore) -> HarnessResult<String> {
    write_restore_journal(restore)?;
    let mut config = AppConfig::load(&restore.config_path)
        .map_err(|_| HarnessError::new("preflight", "config-load-failed"))?;
    let own_pubkey = config
        .own_nostr_pubkey_hex()
        .map_err(|_| HarnessError::new("preflight", "host-identity-unavailable"))?;
    let network_id = config.add_owned_network("WebVM e2e");
    config
        .set_network_enabled(&network_id, true)
        .map_err(|_| HarnessError::new("preflight", "temporary-network-enable-failed"))?;
    config.node.advertise_exit_node = true;
    config.connect_to_non_roster_fips_peers = true;
    config.exit_node.clear();
    config.nostr.relays = vec![NOSTR_VPN_JOIN_APPROVAL_RELAY.to_string()];
    config.nostr.disabled_relays.clear();
    config
        .save(&restore.config_path)
        .map_err(|_| HarnessError::new("preflight", "config-save-failed"))?;
    if !reload_daemon(restore) {
        return Err(HarnessError::new("preflight", "daemon-reload-failed"));
    }
    Ok(own_pubkey)
}

fn sanitized_import_error(error: &str) -> &'static str {
    let error = error.to_ascii_lowercase();
    if error.contains("not administered") {
        "active-admin-network-required"
    } else if error.contains("timestamp is in the future") {
        "join-request-from-future"
    } else if error.contains("has expired") {
        "join-request-expired"
    } else if error.contains("must name exactly one approval relay") {
        "approval-relay-count-invalid"
    } else if error.contains("invalid join request approval relay") {
        "approval-relay-resource-invalid"
    } else if error.contains("failed to initialize join approval pubsub provider") {
        "approval-relay-connect-failed"
    } else if error.contains("approval relay") {
        "invalid-approval-relay"
    } else if error.contains("request type") {
        "invalid-request-type"
    } else if error.contains("separate ephemeral") {
        "invalid-request-key"
    } else if error.contains("different admin device") {
        "wrong-admin-device"
    } else if error.contains("failed to parse join request") {
        "join-request-parse-failed"
    } else if error.contains("no nostr relays") {
        "approval-relays-unavailable"
    } else if error.contains("failed to add nostr relay") {
        "approval-relay-add-failed"
    } else if error.contains("failed to publish join request approval") {
        "approval-publish-failed"
    } else if error.contains("pubsub batch timed out") {
        "approval-publish-timeout"
    } else if error.contains("not accepted by any relay") {
        "approval-publish-rejected"
    } else if error.contains("approval request") || error.contains("join request") {
        "invalid-join-request"
    } else {
        "ffi-import-failed"
    }
}

fn run_prepared_session(
    app: &FfiApp,
    before: &NativeAppState,
    restore: &HostRestore,
    exit_node: &str,
) -> HarnessResult<()> {
    emit_status(&json!({
        "status": "ready",
        "exitNode": exit_node,
    }))?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let Some(mut command) = read_command(&mut input)? else {
        return cleanup(None, restore);
    };
    if command == "cleanup" {
        return cleanup(None, restore);
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
    if !after.error.is_empty() {
        return Err(HarnessError::new(
            "import",
            sanitized_import_error(&after.error),
        ));
    }
    let participant = added_participant(before, &after)?;
    emit_status(&json!({
        "status": "imported",
        "participantAdded": true,
        "exitNode": exit_node,
    }))?;

    match read_command(&mut input)? {
        None => cleanup(Some(&participant), restore),
        Some(command) if command == "cleanup" => cleanup(Some(&participant), restore),
        Some(_) => {
            cleanup(Some(&participant), restore)?;
            Err(HarnessError::new("protocol", "unexpected-command"))
        }
    }
}

fn run() -> HarnessResult<()> {
    if env::var(REAL_E2E_GUARD).as_deref() != Ok("1") {
        return Err(HarnessError::new("guard", "real-e2e-not-enabled"));
    }

    let args = Args::parse()?;
    let restore = HostRestore::new(&args);
    if restore.journal_path.exists() && !restore_host(&restore) {
        return Err(HarnessError::new("preflight", "stale-restore-failed"));
    }
    let initial_app = FfiApp::new_with_config_path(
        args.config_path.clone(),
        env!("CARGO_PKG_VERSION").to_string(),
        Some(args.nvpn_bin.clone()),
    );
    let initial = initial_app.state();
    if !initial.error.is_empty() {
        return Err(HarnessError::new("startup", "ffi-app-startup-failed"));
    }
    drop(initial_app);
    let expected_exit = match prepare_host_config(&restore) {
        Ok(expected_exit) => expected_exit,
        Err(error) => {
            let _ = restore_host(&restore);
            return Err(error);
        }
    };
    let app = FfiApp::new_with_config_path(
        args.config_path,
        env!("CARGO_PKG_VERSION").to_string(),
        Some(args.nvpn_bin),
    );
    let before = app.state();
    if !before.error.is_empty()
        || !before
            .networks
            .iter()
            .any(|network| network.enabled && network.local_is_admin)
    {
        let _ = restore_host(&restore);
        return Err(HarnessError::new(
            "preflight",
            "active-admin-network-required",
        ));
    }
    match run_prepared_session(&app, &before, &restore, &expected_exit) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = restore_host(&restore);
            Err(error)
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
