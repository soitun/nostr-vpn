impl NativeAppRuntime {
    #[allow(clippy::too_many_lines)]
    fn apply_settings_patch(&mut self, patch: SettingsPatch) -> Result<()> {
        let parsed_wireguard_exit_config = patch
            .wireguard_exit_config
            .as_deref()
            .map(parse_wireguard_exit_config)
            .transpose()?;

        if let Some(value) = patch.node_name {
            self.config.node_name = value.trim().to_string();
        }
        if let Some(value) = patch.endpoint {
            self.config.node.endpoint = value.trim().to_string();
        }
        if let Some(value) = patch.tunnel_ip {
            self.config.node.tunnel_ip = value.trim().to_string();
        }
        if let Some(value) = patch.listen_port {
            self.config.node.listen_port = value;
        }
        self.apply_relay_settings_patch(patch.relays, patch.disabled_relays);
        if let Some(value) = patch.nostr_pubsub_mode {
            self.config.nostr.pubsub.mode =
                value.parse::<NostrPubsubMode>().map_err(|error| anyhow!(error))?;
        }
        if let Some(value) = patch.nostr_pubsub_fanout {
            self.config.nostr.pubsub.fanout = value as usize;
        }
        if let Some(value) = patch.nostr_pubsub_max_hops {
            self.config.nostr.pubsub.max_hops = value;
        }
        if let Some(value) = patch.nostr_pubsub_max_event_bytes {
            self.config.nostr.pubsub.max_event_bytes = value as usize;
        }
        // Exit-node selection is mutually exclusive: at most one of
        // (peer exit_node, WireGuard upstream) can be active at a
        // time. The daemon enforces this so every UI / CLI client
        // can push a single field and get the right end state —
        // they don't have to remember to clear the other side. When
        // both fields are in the same patch, both are applied as
        // sent (so the patch fully specifies the intent).
        let exit_node_in_patch = patch.exit_node.is_some();
        let wg_enabled_in_patch = patch.wireguard_exit_enabled.is_some();
        if let Some(value) = patch.exit_node {
            self.config.exit_node = if value.trim().is_empty() {
                String::new()
            } else {
                normalize_nostr_pubkey(&value)?
            };
        }
        if let Some(value) = patch.exit_node_leak_protection {
            self.config.exit_node_leak_protection = value;
        }
        if let Some(value) = patch.advertise_exit_node {
            self.config.node.advertise_exit_node = value;
        }
        if let Some(value) = patch.advertised_routes {
            self.config.node.advertised_routes = parse_advertised_routes(&value);
        }
        if let Some(value) = patch.wireguard_exit_enabled {
            self.config.wireguard_exit.enabled = value;
        }
        // Mutual-exclusion guard. After both fields have landed,
        // resolve any conflict by preferring the field the patch
        // *explicitly* set. If both are set explicitly, the patch
        // already declared the full intent — trust it. If only one
        // is in the patch, clear the other side when the new value
        // would otherwise leave both "selected".
        let peer_set = !self.config.exit_node.is_empty();
        let wg_on = self.config.wireguard_exit.enabled;
        if peer_set && wg_on {
            if exit_node_in_patch && !wg_enabled_in_patch {
                self.config.wireguard_exit.enabled = false;
            } else if wg_enabled_in_patch && !exit_node_in_patch {
                self.config.exit_node = String::new();
            }
            // both-in-patch: leave as the caller sent — caller will
            // typically have set them consistently (one true, other
            // empty), so this branch is dead in practice.
        }
        if let Some(value) = patch.wireguard_exit_interface {
            self.config.wireguard_exit.interface = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_address {
            self.config.wireguard_exit.address = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_private_key {
            self.config.wireguard_exit.private_key = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_peer_public_key {
            self.config.wireguard_exit.peer_public_key = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_peer_preshared_key {
            self.config.wireguard_exit.peer_preshared_key = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_endpoint {
            self.config.wireguard_exit.endpoint = value.trim().to_string();
        }
        if let Some(value) = patch.wireguard_exit_allowed_ips {
            self.config.wireguard_exit.allowed_ips = parse_advertised_routes(&value);
        }
        if let Some(value) = patch.wireguard_exit_dns {
            self.config.wireguard_exit.dns = parse_csv_values(&value);
        }
        if let Some(value) = patch.wireguard_exit_mtu {
            self.config.wireguard_exit.mtu = value;
        }
        if let Some(value) = patch.wireguard_exit_persistent_keepalive_secs {
            self.config.wireguard_exit.persistent_keepalive_secs = value;
        }
        if let Some(mut parsed) = parsed_wireguard_exit_config {
            let enabled = self.config.wireguard_exit.enabled;
            parsed.enabled = enabled;
            self.config.wireguard_exit = parsed;
        }
        if let Some(value) = patch.paid_exit_enabled {
            self.config.paid_exit.enabled = value;
        }
        if let Some(value) = patch.paid_exit_upstream {
            self.config.paid_exit.access.upstream = value
                .parse::<PaidExitUpstream>()
                .map_err(|error| anyhow!(error))?;
        }
        if let Some(value) = patch.paid_exit_meter {
            self.config.paid_exit.pricing.meter =
                value.parse::<PaidRouteMeter>().map_err(|error| anyhow!(error))?;
        }
        if let Some(value) = patch.paid_exit_price_msat {
            self.config.paid_exit.pricing.price_msat = value;
        }
        if let Some(value) = patch.paid_exit_per_units {
            self.config.paid_exit.pricing.per_units = value;
        }
        if let Some(value) = patch.paid_exit_accepted_mints {
            self.config.paid_exit.channel.accepted_mints = parse_csv_values(&value);
        }
        if let Some(value) = patch.paid_exit_max_channel_capacity_sat {
            self.config.paid_exit.channel.max_channel_capacity_sat = value;
        }
        if let Some(value) = patch.paid_exit_channel_expiry_secs {
            self.config.paid_exit.channel.channel_expiry_secs = value;
        }
        if let Some(value) = patch.paid_exit_free_probe_units {
            self.config.paid_exit.channel.free_probe_units = value;
        }
        if let Some(value) = patch.paid_exit_grace_units {
            self.config.paid_exit.channel.grace_units = value;
        }
        if let Some(value) = patch.paid_exit_country_code {
            self.config.paid_exit.location.country_code = value;
        }
        if let Some(value) = patch.paid_exit_region {
            self.config.paid_exit.location.region = value;
        }
        if let Some(value) = patch.paid_exit_asn {
            self.config.paid_exit.location.asn = parse_optional_asn(&value)?;
        }
        if let Some(value) = patch.paid_exit_network_class {
            self.config.paid_exit.location.network_class = value
                .parse::<ExitNetworkClass>()
                .map_err(|error| anyhow!(error))?;
        }
        if let Some(value) = patch.paid_exit_ipv4 {
            self.config.paid_exit.ip_support.ipv4 = value;
        }
        if let Some(value) = patch.paid_exit_ipv6 {
            self.config.paid_exit.ip_support.ipv6 = value;
        }
        if let Some(value) = patch.paid_exit_rating_file {
            self.config.paid_exit.rating_discovery.file = value.trim().to_string();
        }
        if let Some(value) = patch.paid_exit_rating_relays {
            self.config.paid_exit.rating_discovery.relays = normalize_relay_urls(value);
        }
        if let Some(value) = patch.paid_exit_trusted_rating_authors {
            self.config.paid_exit.rating_discovery.trusted_authors =
                Self::normalize_string_list(&value);
        }
        if let Some(value) = patch.paid_exit_rating_scope {
            self.config.paid_exit.rating_discovery.scope = value.trim().to_string();
        }
        if let Some(value) = patch.fips_host_tunnel_enabled {
            self.config.fips_host_tunnel_enabled = value;
        }
        if let Some(value) = patch.connect_to_non_roster_fips_peers {
            self.config.connect_to_non_roster_fips_peers = value;
        }
        if let Some(value) = patch.fips_nostr_discovery_enabled {
            self.config.fips_nostr_discovery_enabled = value;
        }
        if let Some(value) = patch.fips_bootstrap_enabled {
            self.config.fips_bootstrap_enabled = value;
        }
        if let Some(value) = patch.fips_bootstrap_peers {
            self.config.set_fips_bootstrap_peers(value);
        }
        if let Some(value) = patch.fips_host_inbound_tcp_ports {
            self.config.fips_host_inbound_tcp_ports = parse_tcp_ports(&value);
        }
        if let Some(value) = patch.autoconnect {
            self.config.autoconnect = value;
        }
        if let Some(value) = patch.launch_on_startup {
            self.config.launch_on_startup = value;
        }
        if let Some(value) = patch.close_to_tray_on_close {
            self.config.close_to_tray_on_close = value;
        }
        self.config.nostr.pubsub.normalize();
        Ok(())
    }

    fn apply_relay_settings_patch(
        &mut self,
        relays: Option<Vec<String>>,
        disabled_relays: Option<Vec<String>>,
    ) {
        if let Some(value) = relays {
            self.config.nostr.relays = normalize_relay_urls(value);
            let enabled_relays = self
                .config
                .nostr
                .relays
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            self.config
                .nostr
                .disabled_relays
                .retain(|relay| !enabled_relays.contains(relay));
        }
        if let Some(value) = disabled_relays {
            self.config.nostr.disabled_relays = normalize_relay_urls(value);
            let disabled_relays = self
                .config
                .nostr
                .disabled_relays
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            self.config
                .nostr
                .relays
                .retain(|relay| !disabled_relays.contains(relay));
        }
    }

    fn normalize_string_list(values: &[String]) -> Vec<String> {
        let mut values = values
            .iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        values.sort();
        values.dedup();
        values
    }

    fn connect_vpn(&mut self) -> Result<()> {
        if network_setup_required_for_config(&self.config) {
            self.vpn_enabled = false;
            self.vpn_active = false;
            self.vpn_status = "Create or join a network first".to_string();
            return Err(anyhow!(self.vpn_status.clone()));
        }
        self.vpn_enabled = true;
        if self.mobile_runtime {
            self.vpn_enabled = true;
            self.vpn_active = true;
            self.daemon_running = true;
            self.vpn_status = "VPN on".to_string();
            return self.refresh_mobile_status();
        }
        let _ = self.refresh_status();
        if self.daemon_running {
            let output = self.run_nvpn(["resume", "--config", self.config_path_str()?])?;
            ensure_success("nvpn resume", &output)?;
            return self.refresh_status();
        }
        if external_daemon_mode() {
            self.vpn_enabled = false;
            self.vpn_active = false;
            self.vpn_status = "VPN service starting".to_string();
            return Err(anyhow!(self.vpn_status.clone()));
        }
        #[cfg(target_os = "macos")]
        {
            self.refresh_service_status_if_due();
            if !self.service_running {
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = if self.service_installed {
                    "Start background service first".to_string()
                } else {
                    "Install background service first".to_string()
                };
                return Err(anyhow!(self.vpn_status.clone()));
            }
        }
        let output = self.run_nvpn([
            "start",
            "--daemon",
            "--connect",
            "--config",
            self.config_path_str()?,
        ])?;
        ensure_success("nvpn start", &output)?;
        self.refresh_status()
    }

    fn disconnect_vpn(&mut self) -> Result<()> {
        self.vpn_enabled = false;
        if self.mobile_runtime {
            self.vpn_enabled = false;
            self.vpn_active = false;
            self.daemon_running = false;
            self.vpn_status = "Disconnected".to_string();
            self.clear_mobile_runtime_state();
            return self.refresh_mobile_status();
        }
        let output = self.run_nvpn(["pause", "--config", self.config_path_str()?])?;
        ensure_success("nvpn pause", &output)?;
        self.refresh_status()
    }

    fn refresh_mobile_status(&mut self) -> Result<()> {
        self.reload_config_from_disk()?;
        self.refresh_lan_pairing();
        self.daemon_state = None;
        self.service_supported = false;
        self.service_enablement_supported = false;
        self.service_installed = false;
        self.service_disabled = false;
        self.service_running = false;
        self.service_binary_version.clear();
        self.service_status_detail = "Background service unsupported on mobile".to_string();
        let mobile_state = self.load_mobile_runtime_state();
        if self.vpn_enabled || mobile_state.is_some() {
            self.daemon_running = true;
            self.vpn_active = true;
            if self.vpn_status.trim().is_empty()
                || self.vpn_status == "CLI unavailable"
                || self.vpn_status.starts_with("nvpn CLI binary not found")
            {
                self.vpn_status = "VPN on".to_string();
            }
            if let Some(state) = mobile_state {
                self.daemon_state = Some(state.clone());
                self.daemon_running = state.vpn_enabled;
                self.vpn_enabled = state.vpn_enabled;
                self.vpn_active = state.vpn_active;
                self.vpn_status = state.vpn_status;
            }
        } else {
            self.daemon_running = false;
            self.vpn_active = false;
            self.vpn_status = "Disconnected".to_string();
        }
        Ok(())
    }

    fn load_mobile_runtime_state(&self) -> Option<DaemonRuntimeState> {
        let path = self.mobile_runtime_state_path()?;
        let raw = fs::read_to_string(path).ok()?;
        let state = serde_json::from_str::<DaemonRuntimeState>(&raw).ok()?;
        let age = age_secs_since_with_future_skew(
            state.updated_at,
            MOBILE_RUNTIME_STATE_MAX_FUTURE_SKEW_SECS,
        )?;
        if age > MOBILE_RUNTIME_STATE_STALE_SECS {
            return None;
        }
        Some(state)
    }

    fn clear_mobile_runtime_state(&self) {
        if let Some(path) = self.mobile_runtime_state_path() {
            let _ = fs::remove_file(path);
        }
    }

    fn mobile_runtime_state_path(&self) -> Option<PathBuf> {
        self.config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join(MOBILE_RUNTIME_STATE_FILE))
    }

    fn refresh_status(&mut self) -> Result<()> {
        self.reload_config_from_disk()?;
        self.refresh_service_status_if_due();
        self.refresh_lan_pairing();
        let output = self.run_nvpn([
            "status",
            "--json",
            "--discover-secs",
            "0",
            "--config",
            self.config_path_str()?,
        ]);

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let json_text = extract_json_document(&stdout)?;
                let parsed = serde_json::from_str::<CliStatusResponse>(json_text)
                    .context("failed to parse `nvpn status --json` output")?;
                self.daemon_state = parsed.daemon.state;
                self.daemon_running = parsed.daemon.running;
                self.vpn_enabled = self
                    .daemon_state
                    .as_ref()
                    .map_or(parsed.daemon.running, |state| state.vpn_enabled);
                self.vpn_active = self
                    .daemon_state
                    .as_ref()
                    .map_or(parsed.daemon.running, |state| state.vpn_active);
                self.vpn_status = self.daemon_state.as_ref().map_or_else(
                    || {
                        if parsed.daemon.running {
                            "Daemon running".to_string()
                        } else {
                            "Disconnected".to_string()
                        }
                    },
                    |state| state.vpn_status.clone(),
                );
                Ok(())
            }
            Ok(output) => {
                self.daemon_state = None;
                self.daemon_running = false;
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = "Daemon status unavailable".to_string();
                Err(command_failure("nvpn status", &output))
            }
            Err(error) => {
                self.daemon_state = None;
                self.daemon_running = false;
                self.vpn_enabled = false;
                self.vpn_active = false;
                self.vpn_status = "CLI unavailable".to_string();
                Err(error)
            }
        }
    }

    fn save_reload_and_refresh(&mut self) -> Result<()> {
        self.save_config()?;
        if self.mobile_runtime {
            self.refresh_mobile_status()
        } else {
            if self.daemon_running {
                let output = self.run_nvpn(["reload", "--config", self.config_path_str()?])?;
                ensure_success("nvpn reload", &output)?;
            }
            self.refresh_status()
        }
    }

    fn relay_views(&self) -> Vec<NativeRelayState> {
        let active_relays = effective_config_relays(&self.config);
        let active_lookup = active_relays
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut live_status = HashMap::new();
        if let Some(daemon_state) = self.daemon_state.as_ref()
            && !daemon_state.relays.is_empty()
        {
            live_status.extend(
                daemon_state
                    .relays
                    .iter()
                    .map(|relay| (relay.url.clone(), relay.status.clone())),
            );
        }

        let mut rows = active_relays
            .iter()
            .map(|url| NativeRelayState {
                url: url.clone(),
                status: live_status
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string()),
                enabled: true,
            })
            .collect::<Vec<_>>();

        rows.extend(
            normalize_relay_urls(self.config.nostr.disabled_relays.clone())
                .into_iter()
                .filter(|url| !active_lookup.contains(url))
                .map(|url| NativeRelayState {
                    url,
                    status: "disabled".to_string(),
                    enabled: false,
                }),
        );
        rows
    }

    fn save_reload_refresh_and_maybe_connect_for_join_requests(
        &mut self,
        enabled: bool,
    ) -> Result<()> {
        self.save_reload_and_refresh()?;
        if enabled && !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn save_config(&mut self) -> Result<()> {
        self.config.ensure_defaults();
        maybe_autoconfigure_node(&mut self.config);
        self.config.save(&self.config_path)
    }

    #[cfg(target_os = "macos")]
    fn save_config(&mut self) -> Result<()> {
        self.config.ensure_defaults();
        maybe_autoconfigure_node(&mut self.config);

        if self.service_installed || self.service_running || self.daemon_running {
            return self.save_config_via_macos_service();
        }

        self.config.save(&self.config_path)
    }

    #[cfg(target_os = "macos")]
    fn save_config_via_macos_service(&mut self) -> Result<()> {
        let source_path = self.write_config_apply_source()?;
        let result = self.apply_macos_config_source(&source_path);
        let remove_result = fs::remove_file(&source_path)
            .with_context(|| format!("failed to remove {}", source_path.display()));
        let secret_remove_result = AppConfig::delete_persisted_secrets_for_path(&source_path)
            .with_context(|| format!("failed to remove secrets for {}", source_path.display()));

        match (result, remove_result, secret_remove_result) {
            (Ok(()), Ok(()), Ok(())) => Ok(()),
            (Ok(()), Ok(()), Err(error)) | (Ok(()), Err(error), _) | (Err(error), _, _) => {
                Err(error)
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn apply_macos_config_source(&mut self, source_path: &Path) -> Result<()> {
        let source_arg = source_path
            .to_str()
            .ok_or_else(|| anyhow!("config apply source path is not valid UTF-8"))?;
        let config_arg = self.config_path_str()?;
        let daemon_result = self
            .run_nvpn([
                "apply-config-daemon",
                "--source",
                source_arg,
                "--config",
                config_arg,
            ])
            .and_then(|output| ensure_success("nvpn apply-config-daemon", &output));

        if daemon_result.is_ok() {
            return Ok(());
        }

        if self.service_installed || self.service_running {
            let daemon_error = daemon_result.err().map_or_else(
                || "daemon apply failed".to_string(),
                |error| format!("{error:#}"),
            );
            let output = self.run_nvpn_service_action_with_macos_admin([
                "apply-config",
                "--source",
                source_arg,
                "--config",
                config_arg,
            ])?;
            ensure_success("nvpn apply-config", &output)
                .with_context(|| format!("daemon config apply failed first: {daemon_error}"))?;
            return Ok(());
        }

        self.config.save(&self.config_path)
    }

    #[cfg(target_os = "macos")]
    fn write_config_apply_source(&self) -> Result<PathBuf> {
        let parent = self
            .config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;

        let file_name = self
            .config_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("config.toml");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());

        for attempt in 0..128u32 {
            let candidate = parent.join(format!(
                ".{file_name}.apply-{}-{nonce}-{attempt}.toml",
                std::process::id()
            ));
            if candidate.exists() {
                continue;
            }
            self.config
                .save_plaintext(&candidate)
                .with_context(|| format!("failed to write {}", candidate.display()))?;
            return Ok(candidate);
        }

        Err(anyhow!(
            "failed to allocate a unique config apply source file"
        ))
    }

    fn reload_config_from_disk(&mut self) -> Result<()> {
        if !self
            .config_path
            .try_exists()
            .with_context(|| format!("failed to inspect config {}", self.config_path.display()))?
        {
            return Err(anyhow!(
                "config file disappeared: {}",
                self.config_path.display()
            ));
        }

        self.config = AppConfig::load(&self.config_path)?;
        self.config.ensure_defaults();
        maybe_autoconfigure_node(&mut self.config);
        Ok(())
    }

    fn recover_from_startup_error(&mut self) -> Result<()> {
        if self.startup_error.is_none() {
            return Ok(());
        }

        let config_exists = self
            .config_path
            .try_exists()
            .with_context(|| format!("failed to inspect config {}", self.config_path.display()))?;
        let mut config = if config_exists {
            AppConfig::load(&self.config_path)?
        } else {
            AppConfig::generated_without_networks()
        };
        config.ensure_defaults();
        maybe_autoconfigure_node(&mut config);
        let pending_join_request_changed = if config.networks.iter().any(|network| network.enabled) {
            config.clear_pending_nostr_join_request()
        } else {
            config.ensure_pending_nostr_join_request(unix_timestamp())?
        };
        if !config_exists || pending_join_request_changed {
            config.save(&self.config_path)?;
        }

        self.config = config;
        self.startup_error = None;
        self.last_error.clear();
        self.refresh_expected_service_binary_version();
        Ok(())
    }

}
