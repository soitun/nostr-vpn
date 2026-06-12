impl NativeAppRuntime {
    fn refresh_service_status_if_due(&mut self) {
        if !desktop_service_supported() {
            self.service_supported = false;
            self.service_enablement_supported = false;
            self.service_installed = false;
            self.service_disabled = false;
            self.service_running = false;
            self.service_binary_version.clear();
            self.service_status_detail =
                "Background service unsupported on this platform".to_string();
            return;
        }

        let now = Instant::now();
        if self
            .last_service_status_refresh_at
            .is_some_and(|last| now.duration_since(last) < SERVICE_STATUS_REFRESH_INTERVAL)
        {
            return;
        }

        if let Err(error) = self.refresh_service_status() {
            self.service_supported = true;
            self.service_enablement_supported = true;
            self.service_installed = false;
            self.service_disabled = false;
            self.service_running = false;
            self.service_binary_version.clear();
            self.service_status_detail = format!("Service status unavailable: {error}");
        }
    }

    fn refresh_service_status(&mut self) -> Result<()> {
        self.last_service_status_refresh_at = Some(Instant::now());
        let output = self.run_nvpn([
            "service",
            "status",
            "--json",
            "--skip-binary-version",
            "--config",
            self.config_path_str()?,
        ])?;
        if !output.status.success() {
            return Err(command_failure("nvpn service status", &output));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_text = extract_json_document(&stdout)?;
        let status = serde_json::from_str::<CliServiceStatusResponse>(json_text)
            .context("failed to parse `nvpn service status --json` output")?;
        self.service_supported = status.supported;
        self.service_enablement_supported = status.supported;
        self.service_installed = status.installed;
        self.service_disabled = status.disabled;
        self.service_running = status.running;
        self.service_binary_version
            .clone_from(&status.binary_version);
        self.service_status_detail = service_status_detail(&status);
        Ok(())
    }

    fn invalidate_service_status(&mut self) {
        self.last_service_status_refresh_at = None;
    }

    /// Cache the version of the bundled `nvpn` CLI — i.e. the version that
    /// `service install --force` would deploy. Compared against the installed
    /// daemon binary version to decide whether a service update is needed.
    fn refresh_expected_service_binary_version(&mut self) {
        #[derive(Deserialize)]
        struct VersionView {
            version: String,
        }
        let Some(nvpn_bin) = self.nvpn_bin.as_deref() else {
            self.expected_service_binary_version.clear();
            return;
        };
        let Ok(output) = Command::new(nvpn_bin)
            .args(["version", "--json"])
            .hide_console_window()
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Ok(info) = serde_json::from_str::<VersionView>(stdout.trim()) {
            let trimmed = info.version.trim();
            if !trimmed.is_empty() {
                self.expected_service_binary_version = trimmed.to_string();
            }
        }
    }

    fn magic_dns_status(&self) -> String {
        if self.config.magic_dns_suffix.trim().is_empty() {
            return "DNS disabled".to_string();
        }
        if self.vpn_active {
            format!("Serving .{} names", self.config.magic_dns_suffix)
        } else {
            "DNS disabled (VPN off)".to_string()
        }
    }

    fn self_magic_dns_label_for_display(&self) -> Option<String> {
        self.config.self_magic_dns_label().or_else(|| {
            let own_pubkey = self.config.own_nostr_pubkey_hex().ok()?;
            if !self
                .config
                .active_network_signal_pubkeys_hex()
                .iter()
                .any(|member| member == &own_pubkey)
            {
                return None;
            }
            normalize_magic_dns_label(&self.config.node_name).or_else(|| Some("self".to_string()))
        })
    }

    fn self_magic_dns_name_for_display(&self) -> String {
        let Some(alias) = self.self_magic_dns_label_for_display() else {
            return String::new();
        };
        if self.config.magic_dns_suffix.is_empty() {
            alias
        } else {
            format!("{alias}.{}", self.config.magic_dns_suffix)
        }
    }

    fn peer_state_label(
        &self,
        participant: &str,
        peer: Option<&DaemonPeerState>,
        is_local: bool,
        vpn_active: bool,
    ) -> String {
        if !vpn_active {
            return "off".to_string();
        }
        if is_local {
            return "local".to_string();
        }
        if peer.is_some_and(|peer| peer.reachable) {
            return "online".to_string();
        }
        if peer
            .and_then(peer_last_fips_seen_secs)
            .is_some_and(within_presence_grace)
        {
            return "pending".to_string();
        }
        if peer.is_some() {
            return "offline".to_string();
        }
        if self
            .config
            .all_participant_pubkeys_hex()
            .iter()
            .any(|configured| configured == participant)
        {
            return "unknown".to_string();
        }
        "unknown".to_string()
    }

    fn peer_mesh_label(peer: Option<&DaemonPeerState>, is_local: bool, vpn_active: bool) -> String {
        if !vpn_active {
            return "off".to_string();
        }
        if is_local {
            return "local".to_string();
        }
        if peer.is_some_and(|peer| peer.reachable)
            || peer
                .and_then(peer_last_fips_seen_secs)
                .is_some_and(within_presence_grace)
        {
            return "present".to_string();
        }
        if peer.is_some() {
            return "absent".to_string();
        }
        "unknown".to_string()
    }

    fn peer_status_text(peer: Option<&DaemonPeerState>, is_local: bool, state: &str) -> String {
        if is_local {
            return "local".to_string();
        }
        match state {
            "online" => peer.map_or_else(
                || "online".to_string(),
                |peer| {
                    if let Some(link) = peer_link_text(peer) {
                        format!("online via {link}")
                    } else if let Some(age) =
                        peer.last_handshake_at.and_then(presence_age_secs_since)
                    {
                        format!("online (seen {})", compact_age_text(age))
                    } else {
                        "online".to_string()
                    }
                },
            ),
            "pending" => peer
                .and_then(|peer| {
                    peer_link_text(peer).or_else(|| {
                        non_empty(peer.runtime_endpoint.as_deref().unwrap_or(&peer.endpoint))
                    })
                })
                .map_or_else(
                    || "fips link pending".to_string(),
                    |endpoint| format!("fips pending via {}", shorten_middle(&endpoint, 18, 10)),
                ),
            "offline" => peer.and_then(peer_last_fips_seen_age_secs).map_or_else(
                || "offline".to_string(),
                |age| format!("offline ({})", compact_age_text(age)),
            ),
            _ => "unknown".to_string(),
        }
    }

    fn peer_last_fips_seen_text(peer: Option<&DaemonPeerState>, is_local: bool) -> String {
        if is_local {
            return "this device".to_string();
        }
        peer.and_then(peer_last_fips_seen_age_secs)
            .map_or_else(String::new, |age| format!("seen {}", compact_age_text(age)))
    }

    fn peer_last_fips_control_seen_text(peer: Option<&DaemonPeerState>, is_local: bool) -> String {
        Self::peer_last_fips_channel_seen_text(peer, is_local, |peer| {
            peer.last_fips_control_seen_at
        })
    }

    fn peer_last_fips_data_seen_text(peer: Option<&DaemonPeerState>, is_local: bool) -> String {
        Self::peer_last_fips_channel_seen_text(peer, is_local, |peer| peer.last_fips_data_seen_at)
    }

    fn peer_last_fips_channel_seen_text(
        peer: Option<&DaemonPeerState>,
        is_local: bool,
        channel_seen_at: impl FnOnce(&DaemonPeerState) -> Option<u64>,
    ) -> String {
        if is_local {
            return "this device".to_string();
        }
        peer.and_then(|peer| channel_seen_at(peer).and_then(presence_age_secs_since))
            .map_or_else(String::new, |age| format!("seen {}", compact_age_text(age)))
    }

    fn config_path_str(&self) -> Result<&str> {
        self.config_path
            .to_str()
            .ok_or_else(|| anyhow!("config path is not valid UTF-8"))
    }

    fn run_nvpn<const N: usize>(&self, args: [&str; N]) -> Result<Output> {
        let Some(nvpn_bin) = &self.nvpn_bin else {
            return Err(anyhow!(
                "nvpn CLI binary not found; set {NVPN_BIN_ENV} or install nvpn"
            ));
        };
        Command::new(nvpn_bin)
            .args(args)
            .hide_console_window()
            .output()
            .with_context(|| format!("failed to execute {}", nvpn_bin.display()))
    }

    fn run_nvpn_service_action<const N: usize>(&self, args: [&str; N]) -> Result<Output> {
        #[cfg(target_os = "macos")]
        {
            self.run_nvpn_service_action_with_macos_admin(args)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.run_nvpn(args)
        }
    }

    #[cfg(target_os = "macos")]
    fn run_nvpn_service_action_with_macos_admin<const N: usize>(
        &self,
        args: [&str; N],
    ) -> Result<Output> {
        let Some(nvpn_bin) = &self.nvpn_bin else {
            return Err(anyhow!(
                "nvpn CLI binary not found; set {NVPN_BIN_ENV} or install nvpn"
            ));
        };
        if let Some(handle) = &self.privileged_command_runner {
            let outcome = handle.0.run(
                nvpn_bin.display().to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            );
            if outcome.cancelled {
                return Err(anyhow!("user cancelled the administrator prompt"));
            }
            return Ok(privileged_outcome_to_output(outcome));
        }
        let shell_command = macos_service_action_shell_command(nvpn_bin, &args);
        let script = format!(
            "do shell script {} with administrator privileges",
            applescript_quote(&shell_command)
        );
        Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .context("failed to request administrator privileges")
    }

    fn set_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.last_error.clone_from(&error);
        if !error.trim().is_empty() {
            self.vpn_status = error;
        }
    }
}
