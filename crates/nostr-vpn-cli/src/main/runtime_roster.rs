fn publish_fips_active_network_roster_to(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    extra_recipients: &[String],
    pending_recipients: &mut HashSet<String>,
) -> Result<usize> {
    if app.active_network_opt().is_none() {
        return Ok(0);
    }
    let own_pubkey = match app.own_nostr_pubkey_hex() {
        Ok(pubkey) => pubkey,
        Err(_) => return Ok(0),
    };

    let Some(signed_roster) = active_signed_roster_for_sync(app, config_path, false)? else {
        return Ok(0);
    };
    let mut recipients = app.active_network_signal_pubkeys_hex();
    recipients.extend(extra_recipients.iter().cloned());
    recipients.extend(pending_recipients.drain());
    recipients.retain(|recipient| recipient != &own_pubkey);
    recipients.sort();
    recipients.dedup();

    let (ready_recipients, mut retry) = split_ready_fips_roster_recipients(recipients);
    let mut sent = 0usize;
    for recipient in ready_recipients {
        match runtime.enqueue_roster(&recipient, signed_roster.clone()) {
            Ok(()) => sent += 1,
            Err(error) => {
                eprintln!("fips: roster send to {recipient} failed: {error}");
                retry.insert(recipient);
            }
        }
    }
    *pending_recipients = retry;
    Ok(sent)
}

fn persist_join_roster(
    app: &mut AppConfig,
    config_path: &Path,
    control: &JoinRosterControl,
    vpn_status: &mut String,
) -> Result<Option<String>> {
    let Some(applied) = app.apply_nostr_join_roster(control, unix_timestamp())? else {
        return Ok(None);
    };
    let signed_roster = &control.signed_roster;
    upsert_signed_roster(
        &signed_rosters_file_path(config_path),
        signed_roster.clone(),
    )?;
    maybe_autoconfigure_node(app);
    app.save(config_path)?;
    let network_name = app
        .networks
        .iter()
        .find(|network| {
            normalize_runtime_network_id(&network.network_id)
                == normalize_runtime_network_id(&applied.network_id)
        })
        .map(|network| network.name.clone())
        .unwrap_or(applied.network_id);
    *vpn_status = format!("Join approved for {network_name}.");
    Ok(Some(network_name))
}

fn split_ready_fips_roster_recipients(recipients: Vec<String>) -> (Vec<String>, HashSet<String>) {
    // Do not gate roster sends on nvpn presence. A stale-roster peer may drop
    // Ping/Pong from newly added peers as unknown until this signed roster
    // reaches it, while FIPS can still route/discover the control message.
    (recipients, HashSet::new())
}

include!("runtime_endpoint_helpers.rs");
