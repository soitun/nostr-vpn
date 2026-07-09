impl NativeAppRuntime {
    fn dispatch(&mut self, action: NativeAppAction) {
        let result = self.apply_action(action);
        match result {
            Ok(()) => self.last_error.clear(),
            Err(error) => self.set_error(error.to_string()),
        }
        self.rev = self.rev.saturating_add(1);
    }

    #[allow(clippy::too_many_lines)]
    fn apply_action(&mut self, action: NativeAppAction) -> Result<()> {
        if self.startup_error.is_some() {
            match &action {
                NativeAppAction::GetState => return Ok(()),
                NativeAppAction::InstallCli
                | NativeAppAction::UninstallCli
                | NativeAppAction::InstallSystemService => {}
                _ => self.recover_from_startup_error().with_context(
                    || "cannot modify VPN config until the config file is readable",
                )?,
            }
        }

        match action {
            NativeAppAction::GetState | NativeAppAction::Tick => {
                if self.mobile_runtime {
                    self.refresh_mobile_status()
                } else {
                    self.refresh_status()
                }
            }
            NativeAppAction::ConnectVpn => self.connect_vpn(),
            NativeAppAction::DisconnectVpn => self.disconnect_vpn(),
            NativeAppAction::InstallCli => {
                let output = self.run_nvpn(["install-cli", "--force"])?;
                ensure_success("nvpn install-cli", &output)
            }
            NativeAppAction::UninstallCli => {
                let output = self.run_nvpn(["uninstall-cli"])?;
                ensure_success("nvpn uninstall-cli", &output)
            }
            NativeAppAction::InstallSystemService => {
                // Preserve "VPN was on" across the service swap: --force tears
                // down the old daemon and starts a fresh one, which by default
                // comes up disconnected. Without restoring, the user sees the
                // VPN switch flip to OFF every time they update the service —
                // doubly bad after an in-app update where they didn't ask to
                // disconnect.
                let was_vpn_on = self.vpn_enabled || self.vpn_active;
                let output = self.run_nvpn_service_action([
                    "service",
                    "install",
                    "--force",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service install", &output)?;
                self.invalidate_service_status();
                self.recover_from_startup_error()?;
                self.refresh_service_status()?;
                // Refresh the daemon state after the service swap before
                // deciding whether to reconnect. Otherwise stale pre-bootout
                // `vpn_active` can make us skip the restore and the next UI
                // tick flips the VPN switch off.
                let _ = self.refresh_status();
                if was_vpn_on && !(self.vpn_enabled || self.vpn_active) {
                    // Best-effort: ignore connect_vpn errors so a transient
                    // race (new daemon not quite ready yet) doesn't surface
                    // as a "service install failed" message — the install
                    // itself succeeded.
                    let _ = self.connect_vpn();
                }
                Ok(())
            }
            NativeAppAction::UninstallSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "uninstall",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service uninstall", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::EnableSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "enable",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service enable", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::DisableSystemService => {
                let output = self.run_nvpn_service_action([
                    "service",
                    "disable",
                    "--config",
                    self.config_path_str()?,
                ])?;
                ensure_success("nvpn service disable", &output)?;
                self.invalidate_service_status();
                self.refresh_service_status()
            }
            NativeAppAction::AddNetwork { name } => {
                self.config.add_owned_network(&name);
                self.save_reload_and_refresh()
            }
            NativeAppAction::RenameNetwork { network_id, name } => {
                self.config.rename_network(&network_id, &name)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::RemoveNetwork { network_id } => {
                self.config.remove_network(&network_id)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkMeshId {
                network_id,
                mesh_id,
            } => {
                self.config.set_network_mesh_id(&network_id, &mesh_id)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkEnabled {
                network_id,
                enabled,
            } => {
                self.config.set_network_enabled(&network_id, enabled)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetNetworkJoinRequestsEnabled {
                network_id,
                enabled,
            } => {
                self.config
                    .set_network_join_requests_enabled(&network_id, enabled)?;
                self.save_reload_refresh_and_maybe_connect_for_join_requests(enabled)
            }
            NativeAppAction::RequestNetworkJoin { network_id } => {
                self.request_network_join(&network_id)
            }
            NativeAppAction::StartInviteBroadcast => self.start_invite_broadcast(),
            NativeAppAction::StopInviteBroadcast => {
                self.stop_invite_broadcast();
                Ok(())
            }
            NativeAppAction::StartNearbyDiscovery => self.start_nearby_discovery(),
            NativeAppAction::StopNearbyDiscovery => {
                self.stop_nearby_discovery();
                Ok(())
            }
            NativeAppAction::AddParticipant {
                network_id,
                npub,
                alias,
            } => {
                let normalized = self.config.add_participant_to_network(&network_id, &npub)?;
                if let Some(alias) = alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    self.config.set_peer_alias(&normalized, alias)?;
                }
                self.save_reload_and_refresh()
            }
            NativeAppAction::AddAdmin { network_id, npub } => {
                self.config.add_admin_to_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::ResetNetworkInvite { network_id } => {
                self.config.reset_network_invite(&network_id)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::ImportNetworkInvite { invite } => {
                self.import_network_invite(&invite)?;
                Ok(())
            }
            NativeAppAction::ImportJoinRequest { request } => self.import_join_request(&request),
            NativeAppAction::ManualAddNetwork {
                admin_npub,
                mesh_network_id,
            } => {
                self.manual_add_network(&admin_npub, &mesh_network_id)?;
                if !self.vpn_enabled {
                    self.connect_vpn()?;
                }
                Ok(())
            }
            NativeAppAction::RemoveParticipant { network_id, npub } => {
                self.config
                    .remove_participant_from_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::RemoveAdmin { network_id, npub } => {
                self.config.remove_admin_from_network(&network_id, &npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::AcceptJoinRequest {
                network_id,
                requester_npub,
            } => self.accept_join_request(&network_id, &requester_npub),
            NativeAppAction::RejectJoinRequest {
                network_id,
                requester_npub,
            } => {
                self.config
                    .reject_inbound_join_request(&network_id, &requester_npub)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetParticipantAlias { npub, alias } => {
                self.config.set_peer_alias(&npub, &alias)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetParticipantEndpointHints {
                npub,
                endpoint_hints,
            } => {
                self.config
                    .set_fips_peer_endpoint_hints(&npub, &endpoint_hints)?;
                self.save_reload_and_refresh()
            }
            NativeAppAction::SetPaidRouteMarketFilter {
                query,
                country_code,
                network_class,
                mint_url,
                require_ipv4,
                require_ipv6,
                sort,
            } => {
                self.paid_route_market_filter = NativePaidRouteMarketFilterState {
                    query: query.trim().to_string(),
                    country_code: country_code.trim().to_ascii_uppercase(),
                    network_class: network_class.trim().to_ascii_lowercase(),
                    mint_url: mint_url.trim().to_string(),
                    require_ipv4,
                    require_ipv6,
                    sort: if sort.trim().is_empty() {
                        "quality".to_string()
                    } else {
                        sort.trim().to_ascii_lowercase()
                    },
                };
                Ok(())
            }
            NativeAppAction::AddPaidRouteWalletMint { url, label } => {
                self.add_paid_route_wallet_mint(&url, label.as_deref())
            }
            NativeAppAction::RemovePaidRouteWalletMint { url } => {
                self.remove_paid_route_wallet_mint(&url)
            }
            NativeAppAction::SetPaidRouteDefaultMint { url } => {
                self.set_paid_route_default_mint(&url)
            }
            NativeAppAction::RefreshPaidRouteWallet { refresh } => {
                self.refresh_paid_route_wallet(refresh)
            }
            NativeAppAction::TopUpPaidRouteWallet {
                mint_url,
                amount_sat,
            } => self.top_up_paid_route_wallet(mint_url.as_deref(), amount_sat),
            NativeAppAction::ReceivePaidRouteWalletToken { token } => {
                self.receive_paid_route_wallet_token(&token)
            }
            NativeAppAction::SendPaidRouteWalletToken {
                mint_url,
                amount_sat,
            } => self.send_paid_route_wallet_token(mint_url.as_deref(), amount_sat),
            NativeAppAction::WithdrawPaidRouteWalletLightning { mint_url, invoice } => {
                self.withdraw_paid_route_wallet_lightning(mint_url.as_deref(), &invoice)
            }
            NativeAppAction::BuyPaidRouteOffer {
                offer_key,
                mint_url,
                channel_capacity_sat,
            } => self.buy_paid_route_offer(&offer_key, mint_url.as_deref(), channel_capacity_sat),
            NativeAppAction::BuyBestPaidRouteOffer {
                mint_url,
                channel_capacity_sat,
            } => self.buy_best_paid_route_offer(mint_url.as_deref(), channel_capacity_sat),
            NativeAppAction::SelectPaidRouteSession {
                session_id,
                connect,
            } => self.select_paid_route_session(&session_id, connect),
            NativeAppAction::ProbePaidRouteSession {
                session_id,
                timeout_secs,
            } => self.probe_paid_route_session(&session_id, timeout_secs),
            NativeAppAction::RecordPaidRouteProbe {
                session_id,
                realized_exit_ip,
                observed_country_code,
                observed_asn,
                latency_ms,
                jitter_ms,
                packet_loss_ppm,
                down_bps,
                up_bps,
                uptime_secs,
                last_seen_unix,
            } => self.record_paid_route_probe(
                &session_id,
                realized_exit_ip.as_deref(),
                observed_country_code.as_deref(),
                observed_asn,
                latency_ms,
                jitter_ms,
                packet_loss_ppm,
                down_bps,
                up_bps,
                uptime_secs,
                last_seen_unix,
            ),
            NativeAppAction::CreatePaidRoutePaymentEnvelope {
                session_id,
                kind,
                payment_json,
                delivered_units,
                paid_msat,
            } => self.create_paid_route_payment_envelope(
                &session_id,
                &kind,
                &payment_json,
                delivered_units,
                paid_msat,
            ),
            NativeAppAction::OpenPaidRouteChannelFromWallet {
                session_id,
                mint_url,
                paid_msat,
                max_amount_per_output,
                keyset_id,
            } => self.open_paid_route_channel_from_wallet(
                &session_id,
                mint_url.as_deref(),
                paid_msat,
                max_amount_per_output,
                keyset_id.as_deref(),
            ),
            NativeAppAction::SignPaidRoutePaymentEnvelopeFromWallet {
                session_id,
                kind,
                delivered_units,
                paid_msat,
            } => self.sign_paid_route_payment_envelope_from_wallet(
                &session_id,
                &kind,
                delivered_units,
                paid_msat,
            ),
            NativeAppAction::ClosePaidRouteChannelFromWallet {
                session_id,
                publish,
            } => self.close_paid_route_channel_from_wallet(&session_id, publish),
            NativeAppAction::ApplyPaidRoutePaymentEnvelope { envelope_json } => {
                self.apply_paid_route_payment_envelope(&envelope_json)
            }
            NativeAppAction::SendPaidRoutePaymentEnvelope { envelope_json } => {
                self.send_paid_route_payment_envelope(&envelope_json)
            }
            NativeAppAction::StreamPaidRoutePayments {
                publish,
                min_increment_msat,
                limit,
            } => self.stream_paid_route_payments(publish, min_increment_msat, limit),
            NativeAppAction::ReceivePaidRoutePayments { duration_secs } => {
                self.receive_paid_route_payments(duration_secs)
            }
            NativeAppAction::CollectPaidExitChannel { channel_id } => {
                self.collect_paid_exit_channel(&channel_id)
            }
            NativeAppAction::CollectDuePaidExitChannels => self.collect_due_paid_exit_channels(),
            NativeAppAction::PublishPaidExitOffer => self.publish_paid_exit_offer(),
            NativeAppAction::DiscoverPaidRouteOffers { duration_secs } => {
                self.discover_paid_route_offers(duration_secs)
            }
            NativeAppAction::UpdateSettings { patch } => {
                self.apply_settings_patch(patch)?;
                self.save_reload_and_refresh()
            }
        }
    }

    fn import_network_invite(&mut self, invite: &str) -> Result<()> {
        let parsed = parse_network_invite(invite)?;
        apply_network_invite_to_active_network(&mut self.config, &parsed)?;
        self.save_reload_and_refresh()
    }

    fn import_join_request(&mut self, request: &str) -> Result<()> {
        let parsed = parse_join_request_qr_code_or_link(request)?;
        let network_id = self.active_admin_network_id()?;
        self.add_join_requester_to_network(
            &network_id,
            &parsed.pubkey_hex,
            parsed.node_name.as_str(),
        )
    }

    fn active_admin_network_id(&self) -> Result<String> {
        let own_pubkey = self.config.own_nostr_pubkey_hex().ok();
        self.config
            .networks
            .iter()
            .find(|network| {
                network.enabled
                    && own_pubkey.as_deref().is_some_and(|own_pubkey| {
                        network.admins.iter().any(|admin| admin == own_pubkey)
                    })
            })
            .map(|network| network.id.clone())
            .ok_or_else(|| anyhow!("active network is not administered by this device"))
    }

    fn add_join_requester_to_network(
        &mut self,
        network_id: &str,
        requester: &str,
        requester_node_name: &str,
    ) -> Result<()> {
        let requester = normalize_nostr_pubkey(requester)?;
        self.config
            .add_participant_to_network(network_id, &requester)?;
        let requester_node_name = requester_node_name.trim();
        if !requester_node_name.is_empty() {
            let _ = self.config.set_peer_alias(&requester, requester_node_name);
        }
        if let Some(network) = self.config.network_by_id_mut(network_id) {
            network
                .inbound_join_requests
                .retain(|pending| pending.requester != requester);
        }
        self.save_reload_and_refresh()?;
        if !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    fn manual_add_network(&mut self, admin_npub: &str, mesh_network_id: &str) -> Result<()> {
        let admin = admin_npub.trim();
        let mesh_id = mesh_network_id.trim();
        if admin.is_empty() {
            return Err(anyhow!("admin device id is empty"));
        }
        if mesh_id.is_empty() {
            return Err(anyhow!("network id is empty"));
        }
        let synthetic = NetworkInvite {
            v: NETWORK_INVITE_VERSION,
            network_name: String::new(),
            network_id: mesh_id.to_string(),
            invite_secret: String::new(),
            inviter_npub: admin.to_string(),
            inviter_node_name: String::new(),
            inviter_endpoints: Vec::new(),
            admins: vec![admin.to_string()],
            participants: Vec::new(),
            relays: Vec::new(),
        };
        let encoded = serde_json::to_string(&synthetic)
            .map_err(|err| anyhow!("failed to encode manual invite: {err}"))?;
        let parsed = parse_network_invite(&encoded)?;
        apply_network_invite_to_active_network(&mut self.config, &parsed)?;
        let network_id = self
            .config
            .active_network_opt()
            .ok_or_else(|| anyhow!("network not found"))?
            .id
            .clone();
        self.config.add_participant_to_network(&network_id, admin)?;
        self.save_reload_and_refresh()
    }

    fn request_network_join(&mut self, network_id: &str) -> Result<()> {
        self.queue_network_join_request(network_id)?;
        self.save_reload_and_refresh()?;
        if !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    fn queue_network_join_request(&mut self, network_id: &str) -> Result<bool> {
        let network = self
            .config
            .network_by_id(network_id)
            .ok_or_else(|| anyhow!("network not found"))?
            .clone();
        if self.network_contains_own_identity(&network) {
            return Ok(false);
        }
        let recipient = preferred_join_request_recipient(&network)
            .ok_or_else(|| anyhow!("this network was not imported from an invite"))?;
        if network
            .outbound_join_request
            .as_ref()
            .is_some_and(|existing| existing.recipient == recipient)
        {
            return Ok(false);
        }

        let _ = self.config.ensure_temporary_self_magic_dns_alias();
        let network = self
            .config
            .network_by_id_mut(network_id)
            .ok_or_else(|| anyhow!("network not found"))?;
        network.outbound_join_request = Some(PendingOutboundJoinRequest {
            recipient,
            requested_at: unix_timestamp(),
        });
        Ok(true)
    }

    fn network_contains_own_identity(&self, network: &NetworkConfig) -> bool {
        let Some(own_pubkey) = self.config.own_nostr_pubkey_hex().ok() else {
            return false;
        };
        network
            .devices
            .iter()
            .chain(network.admins.iter())
            .any(|member| member == &own_pubkey)
    }

    fn accept_join_request(&mut self, network_id: &str, requester_npub: &str) -> Result<()> {
        let requester = normalize_nostr_pubkey(requester_npub)?;
        let network = self
            .config
            .network_by_id(network_id)
            .ok_or_else(|| anyhow!("network not found"))?;
        let requester_node_name = network
            .inbound_join_requests
            .iter()
            .find(|pending| pending.requester == requester)
            .map(|pending| pending.requester_node_name.clone())
            .ok_or_else(|| anyhow!("no pending join request from {requester_npub}"))?;

        self.add_join_requester_to_network(network_id, &requester, &requester_node_name)
    }

    fn start_invite_broadcast(&mut self) -> Result<()> {
        self.refresh_lan_pairing();
        let announcement = self.build_lan_pairing_announcement()?;
        if announcement.invite.trim().is_empty() {
            return Err(anyhow!(
                "nearby join request advertising is only available before this device has joined a network"
            ));
        }
        let expires_at = lan_pairing_deadline();
        self.ensure_lan_pairing_worker(announcement.clone())?;
        if let Some(worker) = self.lan_pairing_worker.as_ref() {
            worker.update_announcement(announcement);
            worker.set_broadcast_until(expires_at);
        }
        self.invite_broadcast_expires_at = Some(expires_at);
        Ok(())
    }

    fn stop_invite_broadcast(&mut self) {
        if let Some(worker) = self.lan_pairing_worker.as_ref() {
            worker.clear_broadcast();
        }
        self.invite_broadcast_expires_at = None;
        self.gc_lan_pairing_worker();
    }

    fn start_nearby_discovery(&mut self) -> Result<()> {
        self.refresh_lan_pairing();
        let announcement = self.build_lan_pairing_announcement()?;
        let expires_at = lan_pairing_deadline();
        self.ensure_lan_pairing_worker(announcement)?;
        if let Some(worker) = self.lan_pairing_worker.as_ref() {
            worker.set_listen_until(expires_at);
        }
        self.nearby_discovery_expires_at = Some(expires_at);
        self.lan_peers.clear();
        Ok(())
    }

    fn stop_nearby_discovery(&mut self) {
        if let Some(worker) = self.lan_pairing_worker.as_ref() {
            worker.clear_listen();
        }
        self.nearby_discovery_expires_at = None;
        self.lan_peers.clear();
        self.gc_lan_pairing_worker();
    }

    fn ensure_lan_pairing_worker(&mut self, announcement: LanPairingAnnouncement) -> Result<()> {
        if self.lan_pairing_worker.is_some() {
            return Ok(());
        }
        let worker = NativeLanPairingWorker::spawn(announcement)?;
        self.lan_pairing_worker = Some(worker);
        Ok(())
    }

    fn gc_lan_pairing_worker(&mut self) {
        if self.invite_broadcast_expires_at.is_none()
            && self.nearby_discovery_expires_at.is_none()
            && let Some(mut worker) = self.lan_pairing_worker.take()
        {
            worker.stop();
        }
    }

    fn build_lan_pairing_announcement(&self) -> Result<LanPairingAnnouncement> {
        let own_npub = to_npub(&self.config.own_nostr_pubkey_hex()?);
        let invite = if self.config.networks.iter().any(|network| network.enabled) {
            String::new()
        } else {
            own_join_request_qr_code_or_link(&self.config).unwrap_or_default()
        };
        let endpoint = self
            .daemon_state
            .as_ref()
            .and_then(|state| non_empty(&state.advertised_endpoint))
            .unwrap_or_else(|| self.config.node.endpoint.clone());
        Ok(LanPairingAnnouncement {
            npub: own_npub,
            node_name: self.config.node_name.clone(),
            endpoint,
            invite,
        })
    }

    fn live_inviter_endpoints(&self) -> Vec<String> {
        let Some(state) = self.daemon_state.as_ref() else {
            return Vec::new();
        };
        let mut endpoints = Vec::new();
        endpoints.push(state.local_endpoint.clone());
        endpoints.push(state.advertised_endpoint.clone());
        if let Some(external_endpoint) = state.port_mapping.external_endpoint.as_ref() {
            endpoints.push(external_endpoint.clone());
        }
        endpoints
    }

    fn refresh_lan_pairing(&mut self) {
        let now = SystemTime::now();
        if self
            .invite_broadcast_expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            self.invite_broadcast_expires_at = None;
            if let Some(worker) = self.lan_pairing_worker.as_ref() {
                worker.clear_broadcast();
            }
        }
        if self
            .nearby_discovery_expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            self.nearby_discovery_expires_at = None;
            if let Some(worker) = self.lan_pairing_worker.as_ref() {
                worker.clear_listen();
            }
            self.lan_peers.clear();
        }
        self.gc_lan_pairing_worker();

        let Some(worker) = &mut self.lan_pairing_worker else {
            return;
        };
        if self.nearby_discovery_expires_at.is_none() {
            // Drain + drop — listen stopped, don't surface stale signals.
            let _ = worker.drain();
            return;
        }
        let signals = worker.drain();
        for signal in signals {
            if self.lan_signal_is_existing_peer(&signal) {
                continue;
            }
            let key = format!("{}:{}", signal.network_id, signal.npub);
            self.lan_peers.insert(
                key,
                LanPeerRecord {
                    signal,
                    last_seen: now,
                },
            );
        }
    }

    fn invite_broadcast_active(&self) -> bool {
        self.lan_pairing_worker.is_some() && self.invite_broadcast_remaining_secs() > 0
    }

    fn invite_broadcast_remaining_secs(&self) -> u64 {
        Self::remaining_secs(self.invite_broadcast_expires_at)
    }

    fn nearby_discovery_active(&self) -> bool {
        self.lan_pairing_worker.is_some() && self.nearby_discovery_remaining_secs() > 0
    }

    fn nearby_discovery_remaining_secs(&self) -> u64 {
        Self::remaining_secs(self.nearby_discovery_expires_at)
    }

    fn remaining_secs(expires_at: Option<SystemTime>) -> u64 {
        expires_at
            .and_then(|expires| expires.duration_since(SystemTime::now()).ok())
            .map_or(0, |remaining| remaining.as_secs())
    }

    fn lan_peer_states(&self) -> Vec<NativeLanPeerState> {
        let mut peers = self
            .lan_peers
            .values()
            .filter(|record| {
                record
                    .last_seen
                    .elapsed()
                    .is_ok_and(|age| age <= LAN_PAIRING_STALE_AFTER)
            })
            .map(|record| NativeLanPeerState {
                npub: record.signal.npub.clone(),
                node_name: record.signal.node_name.clone(),
                endpoint: record.signal.endpoint.clone(),
                network_name: record.signal.network_name.clone(),
                network_id: record.signal.network_id.clone(),
                invite: record.signal.invite.clone(),
                last_seen_text: record.last_seen.elapsed().map_or_else(
                    |_| "just now".to_string(),
                    |age| compact_age_text(age.as_secs()),
                ),
            })
            .collect::<Vec<_>>();
        peers.sort_by(|left, right| {
            left.network_name
                .cmp(&right.network_name)
                .then_with(|| left.node_name.cmp(&right.node_name))
                .then_with(|| left.npub.cmp(&right.npub))
        });
        peers
    }

    fn lan_signal_is_existing_peer(&self, signal: &LanPairingSignal) -> bool {
        let Ok(sender_hex) = normalize_nostr_pubkey(&signal.npub) else {
            return false;
        };
        if lan_signal_is_join_request(signal) {
            return self.config.networks.iter().any(|network| {
                network.admins.iter().any(|admin| admin == &sender_hex)
                    || network
                        .devices
                        .iter()
                        .any(|device| device == &sender_hex)
            });
        }
        let signal_network_id = normalize_runtime_network_id(&signal.network_id);
        self.config.networks.iter().any(|network| {
            normalize_runtime_network_id(&network.network_id) == signal_network_id
                && (network.admins.iter().any(|admin| admin == &sender_hex)
                    || network
                        .devices
                        .iter()
                        .any(|device| device == &sender_hex))
        })
    }
}

fn lan_signal_is_join_request(signal: &LanPairingSignal) -> bool {
    let invite = signal.invite.trim();
    invite
        .get(.."nvpn://join-request?".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("nvpn://join-request?"))
}
