async fn bind_fips_private_mesh(
    config: &FipsPrivateTunnelConfig,
) -> Result<Arc<FipsPrivateMeshRuntime>> {
    let scope = config
        .ethernet_underlay
        .is_none()
        .then(|| {
            config
                .nostr_discovery_enabled
                .then(|| fips_lan_discovery_scope(&config.network_id))
        })
        .flatten();
    let transport = FipsEndpointTransportConfig {
        listen_port: config.listen_port,
        advertised_endpoint: config.advertised_endpoint.clone(),
        advertise_public_endpoint: config.advertise_public_endpoint,
        nostr_discovery_enabled: config.nostr_discovery_enabled,
        webrtc_enabled: config.webrtc_enabled,
        stun_servers: config.stun_servers.clone(),
        nostr_relays: config.nostr_relays.clone(),
        websocket: config.websocket.clone(),
        share_local_candidates: config.share_local_candidates,
    };
    let endpoint_config = match config.ethernet_underlay.as_ref() {
        Some(ethernet) => fips_endpoint_config_for_ethernet(
            &config.endpoint_peers,
            Some(&transport),
            ethernet,
            config.mesh_mtu,
            config.nostr_discovery_policy,
            config.open_discovery_max_pending,
        ),
        None => fips_endpoint_config_with_open_discovery_limit(
            &config.endpoint_peers,
            Some(&transport),
            config.mesh_mtu,
            config.nostr_discovery_policy,
            config.open_discovery_max_pending,
        ),
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let local_tunnel_ips = config.local_tunnel_ips();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let local_tunnel_ips = Vec::new();

    Ok(Arc::new(
        FipsPrivateMeshRuntime::bind_with_config_scoped(
            config.identity_nsec.clone(),
            scope,
            config.peers.clone(),
            endpoint_config,
            config.local_allowed_ips(),
            local_tunnel_ips,
            config.paid_route_admissions.clone(),
        )
        .await?,
    ))
}

/// A small second endpoint used only while an administrator has a durable
/// join-roster delivery pending. Unjoined devices intentionally use the empty
/// network discovery scope; the administrator's ordinary tunnel cannot reach
/// that scope once it belongs to a network.
pub(crate) struct FipsJoinApprovalRuntime {
    mesh: Arc<FipsPrivateMeshRuntime>,
    state_control: FipsControlTcpRuntime,
    _control_pubsub: Option<crate::control_pubsub_runtime::ControlPubsubFipsRuntime>,
    recipients: HashSet<String>,
}

impl FipsJoinApprovalRuntime {
    pub(crate) async fn start(app: &AppConfig, recipients: &HashSet<String>) -> Result<Self> {
        let config = fips_join_approval_config(app, recipients)?;
        let mesh = bind_fips_private_mesh(&config).await?;
        let target_peer_npubs = config
            .peers
            .iter()
            .map(|peer| peer.endpoint_npub.clone())
            .collect::<Vec<_>>();
        let control_pubsub = crate::control_pubsub_runtime::ControlPubsubFipsRuntime::start_for_peers(
            Arc::clone(mesh.endpoint()),
            config.nostr_pubsub.clone(),
            config.nostr_relays.clone(),
            None,
            &target_peer_npubs,
        )
        .await?;
        let state_control = FipsControlTcpRuntime::start(Arc::clone(mesh.endpoint())).await?;
        Ok(Self {
            mesh,
            state_control,
            _control_pubsub: control_pubsub,
            recipients: recipients.clone(),
        })
    }

    pub(crate) fn matches_recipients(&self, recipients: &HashSet<String>) -> bool {
        self.recipients == *recipients
    }

    pub(crate) async fn send_join_roster(
        &self,
        participant: &str,
        join_roster: JoinRosterControl,
    ) -> Result<()> {
        self.mesh
            .send_join_roster(&self.state_control, participant, join_roster)
            .await
    }
}

fn fips_join_approval_config(
    app: &AppConfig,
    recipients: &HashSet<String>,
) -> Result<FipsPrivateTunnelConfig> {
    if recipients.is_empty() {
        return Err(anyhow!("join approval runtime requires a recipient"));
    }
    let own_pubkey = app.own_nostr_pubkey_hex()?;
    let mut approval_app = app.clone();
    approval_app.connect_to_non_roster_fips_peers = true;
    let active_endpoint_npubs = app
        .active_network_signal_pubkeys_hex()
        .into_iter()
        .filter_map(|participant| {
            FipsMeshPeerConfig::from_participant_pubkey(participant, Vec::new())
                .ok()
                .map(|peer| peer.endpoint_npub)
        })
        .collect::<HashSet<_>>();
    let recipient_endpoint_npubs = recipients
        .iter()
        .map(|participant| {
            FipsMeshPeerConfig::from_participant_pubkey(participant, Vec::new())
                .map(|peer| peer.endpoint_npub)
        })
        .collect::<Result<HashSet<_>>>()?;

    let mut config = FipsPrivateTunnelConfig::from_app(
        &approval_app,
        "",
        "",
        Some(&own_pubkey),
        None,
        &[],
    )?;
    config
        .peers
        .retain(|peer| recipients.contains(&peer.participant_pubkey));
    for recipient in recipients {
        if !config
            .peers
            .iter()
            .any(|peer| peer.participant_pubkey == *recipient)
        {
            config.peers.push(FipsMeshPeerConfig::from_participant_pubkey(
                recipient,
                Vec::new(),
            )?);
        }
    }
    config.endpoint_peers.retain(|peer| {
        recipient_endpoint_npubs.contains(&peer.npub)
            || !active_endpoint_npubs.contains(&peer.npub)
    });
    for recipient in recipients {
        config.endpoint_peers =
            prioritize_fips_control_recipient(config.endpoint_peers, recipient)?;
    }
    config.network_id.clear();
    config.iface.clear();
    config.listen_port = 0;
    config.advertised_endpoint.clear();
    config.advertise_public_endpoint = false;
    config.ethernet_underlay = None;
    config.route_targets.clear();
    config.secure_dns_requested = false;
    config.magic_dns_records.clear();
    config.local_advertised_routes.clear();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        config.fips_host = None;
        config.local_exit_forwarding_routes.clear();
    }
    config.paid_route_admissions.clear();
    #[cfg(feature = "paid-exit")]
    config.paid_route_accounting_peers.clear();
    config.exit_node_leak_protection = false;
    config.nostr_discovery_policy = NostrDiscoveryPolicy::Open;
    config.open_discovery_max_pending = FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING;
    Ok(config)
}

#[cfg(test)]
mod join_approval_runtime_tests {
    use super::*;
    use nostr_sdk::prelude::Keys;

    #[test]
    fn approval_config_uses_unjoined_scope_and_only_queued_roster_peers() {
        let admin = Keys::generate();
        let recipient = Keys::generate();
        let other = Keys::generate();
        let admin_pubkey = admin.public_key().to_hex();
        let recipient_pubkey = recipient.public_key().to_hex();
        let other_pubkey = other.public_key().to_hex();
        let mut app = AppConfig::default();
        app.nostr.secret_key = admin.secret_key().to_bech32().expect("admin nsec");
        app.networks[0].enabled = true;
        app.networks[0].network_id = "joined-network".to_string();
        app.networks[0].devices = vec![recipient_pubkey.clone(), other_pubkey.clone()];
        app.networks[0].admins = vec![admin_pubkey];

        let recipients = HashSet::from([recipient_pubkey.clone()]);
        let config = fips_join_approval_config(&app, &recipients).expect("approval config");

        assert!(config.network_id.is_empty());
        assert_eq!(config.listen_port, 0);
        assert!(config.route_targets.is_empty());
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].participant_pubkey, recipient_pubkey);
        assert!(!config
            .endpoint_peers
            .iter()
            .any(|peer| peer.npub == other.public_key().to_bech32().expect("other npub")));
        assert_eq!(
            fips_lan_discovery_scope(&config.network_id),
            fips_lan_discovery_scope("")
        );
    }
}
