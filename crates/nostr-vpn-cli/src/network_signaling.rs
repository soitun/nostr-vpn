use std::path::Path;

use anyhow::{Result, anyhow};
use nostr_vpn_core::config::{maybe_autoconfigure_node, normalize_nostr_pubkey};
use serde_json::json;

use super::{
    DaemonControlRequest, UpdateRosterArgs, clear_daemon_control_result, daemon_status,
    default_config_path, load_or_default_config, request_daemon_reload,
    wait_for_daemon_control_ack, wait_for_daemon_control_result,
};

pub(crate) fn maybe_reload_running_daemon(config_path: &Path) {
    let status = match daemon_status(config_path) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("config: failed to inspect daemon status after save: {error}");
            return;
        }
    };
    if !status.running {
        return;
    }
    clear_daemon_control_result(config_path);
    if let Err(error) = request_daemon_reload(config_path) {
        eprintln!("config: failed to request daemon reload after save: {error}");
        return;
    }
    if let Err(error) = wait_for_daemon_control_ack(
        config_path,
        crate::daemon_control_ack_timeout(DaemonControlRequest::Reload),
    ) {
        eprintln!("config: daemon did not acknowledge reload after save: {error}");
        return;
    }
    if let Err(error) = wait_for_daemon_control_result(
        config_path,
        DaemonControlRequest::Reload,
        crate::daemon_control_result_timeout(DaemonControlRequest::Reload),
    ) {
        eprintln!("config: daemon reload after save failed: {error}");
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RosterEditAction {
    AddDevice,
    RemoveDevice,
    AddAdmin,
    RemoveAdmin,
}

pub(crate) async fn update_active_network_roster(
    args: UpdateRosterArgs,
    action: RosterEditAction,
) -> Result<()> {
    let config_path = args.config.unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;
    if let Some(network_id) = args.network_id {
        app.set_active_network_id(&network_id)?;
    }
    let active_network_id = app
        .active_network_opt()
        .ok_or_else(|| anyhow!("create or join a network first"))?
        .id
        .clone();

    let mut changed = Vec::new();
    for device in &args.devices {
        let normalized = match action {
            RosterEditAction::AddDevice => app.add_device_to_network(&active_network_id, device)?,
            RosterEditAction::RemoveDevice => {
                let normalized = normalize_nostr_pubkey(device)?;
                app.remove_device_from_network(&active_network_id, device)?;
                normalized
            }
            RosterEditAction::AddAdmin => app.add_admin_to_network(&active_network_id, device)?,
            RosterEditAction::RemoveAdmin => {
                let normalized = normalize_nostr_pubkey(device)?;
                app.remove_admin_from_network(&active_network_id, device)?;
                normalized
            }
        };
        changed.push(normalized);
    }

    app.ensure_defaults();
    maybe_autoconfigure_node(&mut app);
    app.save(&config_path)?;
    maybe_reload_running_daemon(&config_path);

    let published = 0usize;

    if args.json {
        let active_network = app
            .active_network_opt()
            .ok_or_else(|| anyhow!("create or join a network first"))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                        "network_id": app.effective_network_id(),
                        "devices": active_network.devices,
                        "participants": active_network.devices,
                        "admins": active_network.admins,
                "changed": changed,
                "published_recipients": published,
                "published": args.publish,
            }))?
        );
    } else {
        println!("saved {}", config_path.display());
        println!("network_id={}", app.effective_network_id());
        println!("changed={}", changed.join(","));
        if args.publish {
            println!("published_recipients={published}");
        }
    }

    Ok(())
}
