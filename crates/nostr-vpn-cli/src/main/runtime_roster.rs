async fn publish_fips_active_network_roster_to(
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
        match runtime.send_roster(&recipient, signed_roster.clone()).await {
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
fn split_ready_fips_roster_recipients(recipients: Vec<String>) -> (Vec<String>, HashSet<String>) {
    // Do not gate roster sends on nvpn presence. A stale-roster peer may drop
    // Ping/Pong from newly added peers as unknown until this signed roster
    // reaches it, while FIPS can still route/discover the control message.
    (recipients, HashSet::new())
}

include!("runtime_endpoint_helpers.rs");
