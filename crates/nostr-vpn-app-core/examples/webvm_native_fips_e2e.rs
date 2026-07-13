use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use nostr_vpn_app_core::HeadlessDirectApprovalRuntime;
use nostr_vpn_app_core::join_approval::prepare_join_approval;
use nostr_vpn_app_core::join_request_link::parse_join_request_qr_code_or_link;
use nostr_vpn_core::config::AppConfig;
use nostr_vpn_core::join_pubsub::queue_direct_join_approval;
use nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY;
use serde_json::json;

const REAL_E2E_GUARD: &str = "NVPN_WEBVM_REAL_E2E";
const ISOLATED_DIRECTORY_PREFIX: &str = "iris-webvm-nvpn-e2e-";
const IMPORT_COMMAND_PREFIX: &str = "import ";

#[derive(Debug)]
struct Args {
    config_path: PathBuf,
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
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--config-path" => config_path = Some(required_arg_value(&mut args)?),
                _ => return Err(HarnessError::new("arguments", "invalid-argument")),
            }
        }
        let config_path = canonical_file(
            &config_path.ok_or_else(|| HarnessError::new("arguments", "missing-config-path"))?,
            false,
        )?;
        require_isolated_config(&config_path)?;
        Ok(Self { config_path })
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

fn prepare_admin_config(path: &Path) -> HarnessResult<(String, String)> {
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
    config.fips_webrtc_enabled = true;
    config.exit_node.clear();
    config.nostr.relays = vec![NOSTR_VPN_JOIN_APPROVAL_RELAY.to_string()];
    config.nostr.disabled_relays.clear();
    config
        .save(path)
        .map_err(|_| HarnessError::new("preflight", "config-save-failed"))?;
    Ok((own_pubkey, network_id))
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
    let (exit_node, network_id) = prepare_admin_config(&args.config_path)?;
    let transport = HeadlessDirectApprovalRuntime::start(&args.config_path)
        .map_err(|_| HarnessError::new("preflight", "headless-fips-start-failed"))?;
    emit_status(&json!({ "status": "ready", "exitNode": exit_node }))?;

    let mut input = io::stdin().lock();
    let command = read_command(&mut input)?
        .ok_or_else(|| HarnessError::new("protocol", "import-command-required"))?;
    let request = command
        .strip_prefix(IMPORT_COMMAND_PREFIX)
        .filter(|request| request.starts_with("nvpn://join-request/"))
        .filter(|request| !request.chars().any(char::is_whitespace))
        .ok_or_else(|| HarnessError::new("import", "invalid-join-request"))?;
    let parsed = parse_join_request_qr_code_or_link(request)
        .map_err(|_| HarnessError::new("import", "join-request-parse-failed"))?;
    let mut config = AppConfig::load(&args.config_path)
        .map_err(|_| HarnessError::new("import", "config-load-failed"))?;
    let before_count = config.participant_pubkeys_hex().len();
    let approved_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HarnessError::new("import", "clock-unavailable"))?
        .as_secs();
    let prepared = prepare_join_approval(&config, &network_id, &parsed.bootstrap, approved_at)
        .map_err(|_| HarnessError::new("import", "approval-prepare-failed"))?;
    config = prepared.updated_config;
    config
        .save(&args.config_path)
        .map_err(|_| HarnessError::new("import", "config-save-failed"))?;
    queue_direct_join_approval(
        &args.config_path,
        &parsed.bootstrap.device_app_key_npub,
        parsed.fips_route_npub.as_deref(),
        &parsed.bootstrap.request_npub,
        &prepared.events,
    )
    .map_err(|_| HarnessError::new("import", "approval-queue-failed"))?;
    if config.participant_pubkeys_hex().len() != before_count + 1 {
        return Err(HarnessError::new("import", "participant-not-added"));
    }
    let direct_events = transport
        .send_queued_approvals(&args.config_path)
        .map_err(|error| {
            eprintln!("direct FIPS approval send failed: {error:#}");
            HarnessError::new("runtime", "direct-fips-send-failed")
        })?;
    emit_status(&json!({
        "status": "imported",
        "participantAdded": true,
        "directEvents": direct_events,
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
