use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};

use nostr_vpn_app_core::{FfiApp, NativeAppAction, NativeAppState};
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY;
use serde_json::json;

const REAL_E2E_GUARD: &str = "NVPN_WEBVM_REAL_E2E";
const ISOLATED_DIRECTORY_PREFIX: &str = "iris-webvm-nvpn-e2e-";
const IMPORT_COMMAND_PREFIX: &str = "import ";

#[derive(Debug)]
struct Args {
    config_path: PathBuf,
    nvpn_bin: PathBuf,
}

struct IsolatedAdminDaemon(Child);

impl IsolatedAdminDaemon {
    fn spawn(args: &Args) -> HarnessResult<Self> {
        let child = Command::new(&args.nvpn_bin)
            .arg("daemon")
            .arg("--config")
            .arg(&args.config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| HarnessError::new("preflight", "admin-daemon-start-failed"))?;
        Ok(Self(child))
    }

    fn require_running(&mut self) -> HarnessResult<()> {
        match self.0.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(HarnessError::new("runtime", "admin-daemon-exited")),
            Err(_) => Err(HarnessError::new("runtime", "admin-daemon-status-failed")),
        }
    }
}

impl Drop for IsolatedAdminDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
                "--config-path" => config_path = Some(required_arg_value(&mut args)?),
                "--nvpn-bin" => nvpn_bin = Some(required_arg_value(&mut args)?),
                _ => return Err(HarnessError::new("arguments", "invalid-argument")),
            }
        }
        let config_path = canonical_file(
            &config_path.ok_or_else(|| HarnessError::new("arguments", "missing-config-path"))?,
            false,
        )?;
        require_isolated_config(&config_path)?;
        let nvpn_bin = canonical_file(
            &nvpn_bin.ok_or_else(|| HarnessError::new("arguments", "missing-nvpn-bin"))?,
            true,
        )?;
        Ok(Self {
            config_path,
            nvpn_bin,
        })
    }
}

fn required_arg_value(args: &mut impl Iterator<Item = String>) -> HarnessResult<PathBuf> {
    args.next()
        .filter(|value| !value.starts_with("--") && !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::new("arguments", "missing-argument-value"))
}

fn canonical_file(path: &Path, executable: bool) -> HarnessResult<PathBuf> {
    let canonical =
        fs::canonicalize(path).map_err(|_| HarnessError::new("preflight", "path-unavailable"))?;
    let metadata =
        fs::metadata(&canonical).map_err(|_| HarnessError::new("preflight", "path-unavailable"))?;
    if !metadata.is_file() {
        return Err(HarnessError::new("preflight", "path-is-not-file"));
    }
    #[cfg(unix)]
    if executable && metadata.permissions().mode() & 0o111 == 0 {
        return Err(HarnessError::new("preflight", "nvpn-binary-not-executable"));
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(canonical)
}

fn require_isolated_config(path: &Path) -> HarnessResult<()> {
    let temporary_root = fs::canonicalize(env::temp_dir())
        .map_err(|_| HarnessError::new("preflight", "temp-directory-unavailable"))?;
    let isolated_parent = path
        .parent()
        .filter(|parent| parent.starts_with(&temporary_root))
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(ISOLATED_DIRECTORY_PREFIX));
    if !isolated_parent {
        return Err(HarnessError::new(
            "preflight",
            "non-isolated-config-refused",
        ));
    }
    Ok(())
}

fn emit_status(value: &serde_json::Value) -> HarnessResult<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, value)
        .map_err(|_| HarnessError::new("output", "json-write-failed"))?;
    writeln!(output).map_err(|_| HarnessError::new("output", "stdout-write-failed"))?;
    output
        .flush()
        .map_err(|_| HarnessError::new("output", "stdout-flush-failed"))
}

fn prepare_admin_config(path: &Path) -> HarnessResult<String> {
    let mut config =
        AppConfig::load(path).map_err(|_| HarnessError::new("preflight", "config-load-failed"))?;
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
        .save(path)
        .map_err(|_| HarnessError::new("preflight", "config-save-failed"))?;
    Ok(own_pubkey)
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

fn participant_was_added(before: &NativeAppState, after: &NativeAppState) -> bool {
    let before = participant_keys(before);
    participant_keys(after)
        .into_iter()
        .filter(|participant| !before.contains(participant))
        .count()
        == 1
}

fn read_command(input: &mut impl BufRead) -> HarnessResult<Option<String>> {
    let mut command = String::new();
    if input
        .read_line(&mut command)
        .map_err(|_| HarnessError::new("protocol", "stdin-read-failed"))?
        == 0
    {
        return Ok(None);
    }
    Ok(Some(command.trim_end().to_string()))
}

fn run() -> HarnessResult<()> {
    if env::var(REAL_E2E_GUARD).as_deref() != Ok("1") {
        return Err(HarnessError::new("guard", "real-e2e-not-enabled"));
    }
    let args = Args::parse()?;
    let exit_node = prepare_admin_config(&args.config_path)?;
    let mut daemon = IsolatedAdminDaemon::spawn(&args)?;
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
        return Err(HarnessError::new(
            "preflight",
            "active-admin-network-required",
        ));
    }
    emit_status(&json!({ "status": "ready", "exitNode": exit_node }))?;

    let mut input = io::stdin().lock();
    let command = read_command(&mut input)?
        .ok_or_else(|| HarnessError::new("protocol", "import-command-required"))?;
    let request = command
        .strip_prefix(IMPORT_COMMAND_PREFIX)
        .filter(|request| request.starts_with("nvpn://join-request/"))
        .filter(|request| !request.chars().any(char::is_whitespace))
        .ok_or_else(|| HarnessError::new("import", "invalid-join-request"))?;
    let after = app.dispatch(NativeAppAction::ImportJoinRequest {
        request: request.to_string(),
    });
    if !participant_was_added(&before, &after) {
        return Err(HarnessError::new("import", "participant-not-added"));
    }
    daemon.require_running()?;
    emit_status(&json!({
        "status": "imported",
        "participantAdded": true,
        "postApprovalWarning": (!after.error.is_empty()).then_some(after.error),
        "exitNode": exit_node,
    }))?;
    let _ = read_command(&mut input)?;
    emit_status(&json!({ "status": "cleaned" }))
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
