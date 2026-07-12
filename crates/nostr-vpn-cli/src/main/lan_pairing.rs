fn run_invite_broadcast(args: InviteBroadcastArgs) -> Result<()> {
    use nostr_vpn_core::lan_pairing::{
        LAN_PAIRING_DURATION, LanPairingAnnouncement, spawn_lan_pairing_worker,
    };

    let config_path = args.config.unwrap_or_else(default_config_path);
    let app = load_or_default_config(&config_path)?;

    let own_npub_hex = app.own_nostr_pubkey_hex()?;
    let own_npub = nostr_vpn_core::invite::to_npub(&own_npub_hex);
    let invite = active_network_invite_code(&app)?;
    let endpoint = app.node.endpoint.trim().to_string();
    let node_name = app.node_name.trim().to_string();

    let duration = Duration::from_secs(
        args.duration_secs
            .unwrap_or_else(|| LAN_PAIRING_DURATION.as_secs()),
    );
    let expires_at = SystemTime::now()
        .checked_add(duration)
        .ok_or_else(|| anyhow::anyhow!("broadcast duration overflows SystemTime"))?;

    let mut worker = spawn_lan_pairing_worker(
        LanPairingAnnouncement {
            npub: own_npub.clone(),
            node_name,
            endpoint: endpoint.clone(),
            invite: invite.clone(),
        },
        app.nostr_keys()?,
    )?;
    worker.set_broadcast_until(expires_at);

    println!("network_id={}", app.effective_network_id());
    println!("npub={own_npub}");
    println!("endpoint={endpoint}");
    println!("invite={invite}");
    println!(
        "broadcasting on 239.255.73.73:38911 for {} seconds (Ctrl-C to stop)",
        duration.as_secs()
    );

    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
    }
    worker.clear_broadcast();
    worker.stop();
    println!("broadcast stopped");
    Ok(())
}

fn run_discover(args: DiscoverArgs) -> Result<()> {
    use nostr_vpn_core::lan_pairing::{
        LAN_PAIRING_DURATION, LanPairingAnnouncement, LanPairingSignal, spawn_lan_pairing_worker,
    };

    let config_path = args.config.unwrap_or_else(default_config_path);
    let mut app = load_or_default_config(&config_path)?;

    let own_npub_hex = app.own_nostr_pubkey_hex()?;
    let own_npub = nostr_vpn_core::invite::to_npub(&own_npub_hex);
    let endpoint = app.node.endpoint.trim().to_string();
    let node_name = app.node_name.trim().to_string();
    let local_invite = active_network_invite_code(&app).unwrap_or_default();

    let duration = Duration::from_secs(
        args.duration_secs
            .unwrap_or_else(|| LAN_PAIRING_DURATION.as_secs()),
    );
    let expires_at = SystemTime::now()
        .checked_add(duration)
        .ok_or_else(|| anyhow::anyhow!("discover duration overflows SystemTime"))?;

    let mut worker = spawn_lan_pairing_worker(
        LanPairingAnnouncement {
            npub: own_npub,
            node_name,
            endpoint,
            invite: local_invite,
        },
        app.nostr_keys()?,
    )?;
    worker.set_listen_until(expires_at);

    println!(
        "listening on 239.255.73.73:38911 for {} seconds (Ctrl-C to stop)",
        duration.as_secs()
    );

    let mut accepted: Option<LanPairingSignal> = None;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        for signal in worker.drain() {
            if !seen.insert(signal.npub.clone()) {
                continue;
            }
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "npub": signal.npub,
                        "node_name": signal.node_name,
                        "endpoint": signal.endpoint,
                        "network_id": signal.network_id,
                        "network_name": signal.network_name,
                        "invite": signal.invite,
                    }))?
                );
            } else {
                println!(
                    "npub={} node={} endpoint={} network_id={} network_name={}",
                    signal.npub,
                    if signal.node_name.is_empty() {
                        "?"
                    } else {
                        signal.node_name.as_str()
                    },
                    if signal.endpoint.is_empty() {
                        "?"
                    } else {
                        signal.endpoint.as_str()
                    },
                    signal.network_id,
                    if signal.network_name.is_empty() {
                        "-"
                    } else {
                        signal.network_name.as_str()
                    },
                );
            }
            if args.accept {
                accepted = Some(signal);
                break;
            }
        }
        if accepted.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    worker.clear_listen();
    worker.stop();

    if let Some(signal) = accepted {
        let invite = parse_network_invite(&signal.invite)?;
        apply_network_invite_to_active_network(&mut app, &invite)?;
        let join_request_queued = queue_active_network_join_request(&mut app)?;
        app.ensure_defaults();
        maybe_autoconfigure_node(&mut app);
        app.save(&config_path)?;
        maybe_reload_running_daemon(&config_path);
        if args.json {
            println!("{}", serde_json::to_string_pretty(&app)?);
        } else {
            println!("saved {}", config_path.display());
            println!("network_id={}", app.effective_network_id());
            println!("invite_imported={}", app.active_network().name);
            println!("join_request_queued={join_request_queued}");
        }
    }
    Ok(())
}
