use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use nostr_vpn_core::config::{AppConfig, normalize_nostr_pubkey};
use serde_json::json;

const NETWORK_NAME: &str = "Roster GUI e2e";
const REQUESTER_NAME: &str = "e2e-phone";

fn required_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf> {
    args.next()
        .filter(|value| !value.trim().is_empty() && !value.starts_with("--"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("{flag} requires a path"))
}

fn parse_paths() -> Result<(String, PathBuf, PathBuf)> {
    let mut args = env::args().skip(1);
    let command = args.next().context("expected prepare or verify")?;
    let mut data_dir = None;
    let mut result = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--data-dir" => data_dir = Some(required_path(&mut args, &flag)?),
            "--result" => result = Some(required_path(&mut args, &flag)?),
            _ => bail!("unknown argument: {flag}"),
        }
    }
    Ok((
        command,
        data_dir.context("--data-dir is required")?,
        result.context("--result is required")?,
    ))
}

fn requester_npub(config: &AppConfig) -> Result<String> {
    let hex = config.own_nostr_pubkey_hex()?;
    Ok(PublicKey::from_hex(&hex)?.to_bech32()?)
}

fn prepare(data_dir: &Path, result_path: &Path) -> Result<()> {
    if data_dir.exists() {
        fs::remove_dir_all(data_dir)
            .with_context(|| format!("remove old fixture {}", data_dir.display()))?;
    }
    fs::create_dir_all(data_dir)
        .with_context(|| format!("create fixture directory {}", data_dir.display()))?;

    let requester = AppConfig::generated_without_networks();
    let requester_npub = requester_npub(&requester)?;

    let mut admin = AppConfig::generated_without_networks();
    let network_id = admin.add_owned_network(NETWORK_NAME);
    admin.set_network_enabled(&network_id, true)?;
    admin.set_network_join_requests_enabled(&network_id, true)?;
    let network = admin
        .network_by_id(&network_id)
        .context("fixture network missing")?;
    let mesh_network_id = network.network_id.clone();
    let invite_secret = network.invite_secret.clone();
    let requested_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let recorded = admin.record_inbound_join_request(
        &mesh_network_id,
        &invite_secret,
        &requester_npub,
        REQUESTER_NAME,
        requested_at,
    )?;
    if recorded.is_none() {
        bail!("fixture join request was not recorded");
    }
    let config_path = data_dir.join("config.toml");
    admin.save(&config_path)?;

    let debug_url =
        format!("nvpn://debug/accept-join?networkId={network_id}&requesterNpub={requester_npub}");
    write_result(
        result_path,
        json!({
            "ok": true,
            "phase": "prepared",
            "configPath": config_path,
            "networkId": network_id,
            "requesterNpub": requester_npub,
            "requesterName": REQUESTER_NAME,
            "debugUrl": debug_url,
        }),
    )
}

fn verify(data_dir: &Path, result_path: &Path) -> Result<()> {
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(result_path).with_context(|| format!("read {}", result_path.display()))?,
    )?;
    let requester_npub = metadata["requesterNpub"]
        .as_str()
        .context("fixture result has no requesterNpub")?;
    let requester = normalize_nostr_pubkey(requester_npub)?;
    let config_path = data_dir.join("config.toml");
    let admin = AppConfig::load(&config_path)?;
    let network = admin
        .active_network_opt()
        .context("active network missing")?;
    if !network.devices.iter().any(|device| device == &requester) {
        bail!("desktop GUI did not add the requester to the roster");
    }
    if network
        .inbound_join_requests
        .iter()
        .any(|pending| pending.requester == requester)
    {
        bail!("desktop GUI left the accepted join request pending");
    }
    if admin.peer_alias(&requester).as_deref() != Some(REQUESTER_NAME) {
        bail!("desktop GUI did not retain the requester's device name");
    }
    write_result(
        result_path,
        json!({
            "ok": true,
            "phase": "verified",
            "configPath": config_path,
            "networkId": network.id,
            "requesterNpub": requester_npub,
            "requesterName": REQUESTER_NAME,
            "participantPersisted": true,
            "pendingRequestRemoved": true,
            "aliasPersisted": true,
        }),
    )
}

fn write_result(path: &Path, value: serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn run() -> Result<()> {
    let (command, data_dir, result_path) = parse_paths()?;
    match command.as_str() {
        "prepare" => prepare(&data_dir, &result_path),
        "verify" => verify(&data_dir, &result_path),
        _ => bail!("expected prepare or verify"),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("desktop roster e2e fixture failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}
