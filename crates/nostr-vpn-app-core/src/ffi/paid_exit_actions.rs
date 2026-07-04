impl NativeAppRuntime {
    pub(super) fn paid_exit_seller_state(
        &self,
        app: Option<&AppConfig>,
        port_mapping: Option<&PortMappingStatus>,
        mobile: bool,
    ) -> NativePaidExitSellerState {
        paid_exit_seller_state(
            app,
            port_mapping,
            paid_exit_seller_supported_for_current_target(mobile),
            &self.paid_route_store_path(),
        )
    }

    pub(super) fn paid_route_market_state(
        &self,
        app: Option<&AppConfig>,
    ) -> NativePaidRouteMarketState {
        paid_route_market_state(
            app,
            &self.paid_route_store_path(),
            &self.paid_route_market_filter,
            &self.paid_route_wallet_last_action,
            &self.paid_route_payment_last_action,
        )
    }

    fn paid_route_store_path(&self) -> PathBuf {
        paid_route_store_file_path(&self.config_path)
    }

    fn mutate_paid_route_store(
        &mut self,
        mutate: impl FnOnce(&mut PaidRouteStore) -> bool,
    ) -> Result<()> {
        let path = self.paid_route_store_path();
        let mut store = load_paid_route_store(&path)?;
        if mutate(&mut store) {
            write_paid_route_store(&path, &store)?;
        }
        Ok(())
    }

    pub(super) fn add_paid_route_wallet_mint(
        &mut self,
        url: &str,
        label: Option<&str>,
    ) -> Result<()> {
        let label = label.unwrap_or_default();
        self.mutate_paid_route_store(|store| {
            store.upsert_wallet_mint(url, label, None, unix_timestamp())
        })
    }

    pub(super) fn remove_paid_route_wallet_mint(&mut self, url: &str) -> Result<()> {
        self.mutate_paid_route_store(|store| store.remove_wallet_mint(url))
    }

    pub(super) fn set_paid_route_default_mint(&mut self, url: &str) -> Result<()> {
        self.mutate_paid_route_store(|store| store.set_default_mint(url))
    }

    pub(super) fn refresh_paid_route_wallet(&mut self, refresh: bool) -> Result<()> {
        let mut args = vec!["show".to_string()];
        if refresh {
            args.push("--refresh".to_string());
        }
        let _ = self.run_paid_route_wallet_json(args)?;
        self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
            kind: "refresh".to_string(),
            status_text: "Wallet refreshed".to_string(),
            ..NativePaidRouteWalletActionState::default()
        };
        Ok(())
    }

    pub(super) fn top_up_paid_route_wallet(
        &mut self,
        mint_url: Option<&str>,
        amount_sat: u64,
    ) -> Result<()> {
        let mut args = vec!["topup".to_string(), amount_sat.to_string()];
        push_optional_wallet_mint(&mut args, mint_url);
        let value = self.run_paid_route_wallet_json(args)?;
        let quote = value
            .get("quote")
            .ok_or_else(|| anyhow!("wallet top-up output is missing quote"))?;
        self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
            kind: "topup".to_string(),
            status_text: format!("Top-up invoice for {amount_sat} sat"),
            mint_url: json_string(quote, "mint_url"),
            amount_sat: json_u64(quote, "amount_sat"),
            amount_text: paid_route_sat_text(amount_sat),
            quote_id: json_string(quote, "quote_id"),
            payment_request: json_string(quote, "payment_request"),
            expires_at_unix: json_u64(quote, "expiry_unix"),
            ..NativePaidRouteWalletActionState::default()
        };
        Ok(())
    }

    pub(super) fn receive_paid_route_wallet_token(&mut self, token: &str) -> Result<()> {
        let token = token.trim();
        if token.is_empty() {
            return Err(anyhow!("Cashu token is empty"));
        }
        let value = self.run_paid_route_wallet_json_with_stdin(
            vec!["receive".to_string(), "--token-stdin".to_string()],
            token.as_bytes(),
        )?;
        let received = value
            .get("received")
            .ok_or_else(|| anyhow!("wallet receive output is missing received payment"))?;
        let amount_sat = json_u64(received, "amount_sat");
        self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
            kind: "receive".to_string(),
            status_text: format!("Received {amount_sat} sat"),
            mint_url: json_string(received, "mint_url"),
            amount_sat,
            amount_text: paid_route_sat_text(amount_sat),
            ..NativePaidRouteWalletActionState::default()
        };
        Ok(())
    }

    pub(super) fn send_paid_route_wallet_token(
        &mut self,
        mint_url: Option<&str>,
        amount_sat: u64,
    ) -> Result<()> {
        let mut args = vec!["send".to_string(), amount_sat.to_string()];
        push_optional_wallet_mint(&mut args, mint_url);
        let value = self.run_paid_route_wallet_json(args)?;
        let sent = value
            .get("sent")
            .ok_or_else(|| anyhow!("wallet send output is missing sent payment"))?;
        let amount_sat = json_u64(sent, "amount_sat");
        let fee_sat = json_u64(sent, "send_fee_sat");
        self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
            kind: "send".to_string(),
            status_text: format!("Created token for {amount_sat} sat"),
            mint_url: json_string(sent, "mint_url"),
            amount_sat,
            amount_text: paid_route_sat_text(amount_sat),
            fee_sat,
            fee_text: paid_route_fee_text(fee_sat),
            operation_id: json_string(sent, "operation_id"),
            token: json_string(sent, "token"),
            ..NativePaidRouteWalletActionState::default()
        };
        Ok(())
    }

    pub(super) fn withdraw_paid_route_wallet_lightning(
        &mut self,
        mint_url: Option<&str>,
        invoice: &str,
    ) -> Result<()> {
        let invoice = invoice.trim();
        if invoice.is_empty() {
            return Err(anyhow!("Lightning invoice is empty"));
        }
        let mut args = vec!["withdraw".to_string(), invoice.to_string()];
        push_optional_wallet_mint(&mut args, mint_url);
        let value = self.run_paid_route_wallet_json(args)?;
        let withdrawal = value
            .get("withdrawal")
            .ok_or_else(|| anyhow!("wallet withdraw output is missing withdrawal"))?;
        let amount_sat = json_u64(withdrawal, "amount_sat");
        let fee_sat = json_u64(withdrawal, "fee_paid_sat");
        self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
            kind: "withdraw".to_string(),
            status_text: format!("Paid Lightning invoice for {amount_sat} sat"),
            mint_url: json_string(withdrawal, "mint_url"),
            amount_sat,
            amount_text: paid_route_sat_text(amount_sat),
            fee_sat,
            fee_text: paid_route_fee_text(fee_sat),
            quote_id: json_string(withdrawal, "quote_id"),
            preimage: json_string(withdrawal, "preimage"),
            ..NativePaidRouteWalletActionState::default()
        };
        Ok(())
    }

    fn run_paid_route_wallet_json(
        &self,
        mut wallet_args: Vec<String>,
    ) -> Result<serde_json::Value> {
        let args = self.paid_route_wallet_args(&mut wallet_args)?;
        let output = self.run_nvpn_vec(&args)?;
        decode_paid_route_wallet_json_output(output)
    }

    fn run_paid_route_wallet_json_with_stdin(
        &self,
        mut wallet_args: Vec<String>,
        stdin: &[u8],
    ) -> Result<serde_json::Value> {
        let args = self.paid_route_wallet_args(&mut wallet_args)?;
        let output = self.run_nvpn_vec_with_stdin(&args, stdin)?;
        decode_paid_route_wallet_json_output(output)
    }

    fn paid_route_wallet_args(&self, wallet_args: &mut Vec<String>) -> Result<Vec<String>> {
        let mut args = vec![
            "paid-exit".to_string(),
            "wallet".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
        ];
        args.append(wallet_args);
        Ok(args)
    }

    pub(super) fn buy_paid_route_offer(
        &mut self,
        offer_key: &str,
        mint_url: Option<&str>,
        channel_capacity_sat: Option<u64>,
    ) -> Result<()> {
        let buyer_npub = self
            .config
            .nostr_keys()?
            .public_key()
            .to_bech32()
            .context("failed to encode buyer npub")?;
        let path = self.paid_route_store_path();
        let mut store = load_paid_route_store(&path)?;
        let open_channel_from_wallet =
            paid_route_wallet_configured_for_channel_open(&store.wallet, mint_url);
        let result = store.open_buyer_session(OpenPaidRouteBuyerSessionRequest {
            offer_selector: offer_key.to_string(),
            buyer_npub,
            mint_url: mint_url.map(ToOwned::to_owned),
            channel_capacity_sat,
            initial_paid_msat: 0,
            now_unix: unix_timestamp(),
        })?;
        if result.changed {
            write_paid_route_store(&path, &store)?;
        }

        if open_channel_from_wallet {
            self.open_paid_route_channel_from_wallet(
                &result.session_id,
                Some(&result.mint_url),
                None,
                None,
                None,
            )?;
            let envelope_json = self.paid_route_payment_last_action.envelope_json.clone();
            if !envelope_json.trim().is_empty() {
                self.send_paid_route_payment_envelope(&envelope_json)?;
            }
        }

        Ok(())
    }

    pub(super) fn select_paid_route_session(
        &mut self,
        session_id: &str,
        connect: bool,
    ) -> Result<()> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(anyhow!("paid route session id is empty"));
        }
        let store = load_paid_route_store(&self.paid_route_store_path())?;
        let seller_npub = store.buyer_session_seller_npub(session_id)?;
        if connect && !store.buyer_session_allows_routing(session_id, unix_timestamp())? {
            return Err(anyhow!(
                "paid route session is not ready to route yet; fund it or wait for seller admission"
            ));
        }
        self.config.select_public_paid_exit_node(&seller_npub)?;
        self.save_reload_and_refresh()?;
        if connect && !self.vpn_enabled {
            self.connect_vpn()?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_paid_route_probe(
        &mut self,
        session_id: &str,
        realized_exit_ip: Option<&str>,
        observed_country_code: Option<&str>,
        observed_asn: Option<u32>,
        latency_ms: Option<u32>,
        jitter_ms: Option<u32>,
        packet_loss_ppm: Option<u32>,
        down_bps: Option<u64>,
        up_bps: Option<u64>,
        uptime_secs: Option<u64>,
        last_seen_unix: Option<u64>,
    ) -> Result<()> {
        let quality = PaidRouteQualityMetrics {
            latency_ms,
            jitter_ms,
            packet_loss_ppm,
            down_bps,
            up_bps,
            uptime_secs,
            last_seen_unix,
        };
        let path = self.paid_route_store_path();
        let mut store = load_paid_route_store(&path)?;
        let result = store.update_session_probe(UpdatePaidRouteSessionProbeRequest {
            session_id: session_id.to_string(),
            realized_exit_ip: realized_exit_ip.map(ToOwned::to_owned),
            observed_country_code: observed_country_code.map(ToOwned::to_owned),
            observed_asn,
            quality: (!quality.is_empty()).then_some(quality),
            now_unix: unix_timestamp(),
        })?;
        if result.changed {
            write_paid_route_store(&path, &store)?;
        }
        Ok(())
    }

    pub(super) fn probe_paid_route_session(
        &mut self,
        session_id: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        let args = vec![
            "paid-exit".to_string(),
            "probe".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            session_id.trim().to_string(),
            "--timeout-secs".to_string(),
            timeout_secs.max(1).to_string(),
        ];
        let output = self.run_nvpn_vec(&args)?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit probe")?;
        self.paid_route_payment_last_action = paid_route_probe_action_state(&value);
        Ok(())
    }

    pub(super) fn create_paid_route_payment_envelope(
        &mut self,
        session_id: &str,
        kind: &str,
        payment_json: &str,
        delivered_units: Option<u64>,
        paid_msat: Option<u64>,
    ) -> Result<()> {
        let mut args = vec![
            "paid-exit".to_string(),
            "create-payment".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            session_id.trim().to_string(),
            "--kind".to_string(),
            kind.trim().to_string(),
            "--payment-stdin".to_string(),
        ];
        if let Some(delivered_units) = delivered_units {
            args.push("--delivered-units".to_string());
            args.push(delivered_units.to_string());
        }
        if let Some(paid_msat) = paid_msat {
            args.push("--paid-msat".to_string());
            args.push(paid_msat.to_string());
        }

        let output = self.run_nvpn_vec_with_stdin(&args, payment_json.as_bytes())?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit create-payment")?;
        self.paid_route_payment_last_action = paid_route_payment_action_state("create", &value)?;
        Ok(())
    }

    pub(super) fn open_paid_route_channel_from_wallet(
        &mut self,
        session_id: &str,
        mint_url: Option<&str>,
        paid_msat: Option<u64>,
        max_amount_per_output: Option<u64>,
        keyset_id: Option<&str>,
    ) -> Result<()> {
        let mut args = vec![
            "paid-exit".to_string(),
            "create-payment".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            session_id.trim().to_string(),
            "--kind".to_string(),
            "channel-open".to_string(),
            "--open-from-wallet".to_string(),
        ];
        push_optional_wallet_mint(&mut args, mint_url);
        if let Some(paid_msat) = paid_msat {
            args.push("--paid-msat".to_string());
            args.push(paid_msat.to_string());
        }
        if let Some(max_amount_per_output) = max_amount_per_output {
            args.push("--max-amount-per-output".to_string());
            args.push(max_amount_per_output.to_string());
        }
        if let Some(keyset_id) = keyset_id
            .map(str::trim)
            .filter(|keyset_id| !keyset_id.is_empty())
        {
            args.push("--keyset-id".to_string());
            args.push(keyset_id.to_string());
        }

        let output = self.run_nvpn_vec(&args)?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit create-payment")?;
        self.paid_route_payment_last_action =
            paid_route_payment_action_state("open_channel", &value)?;
        if let Some(wallet_send) = value
            .get("wallet_open")
            .and_then(|wallet_open| wallet_open.get("wallet_send"))
        {
            let amount_sat = json_u64(wallet_send, "amount_sat");
            let fee_sat = json_u64(wallet_send, "send_fee_sat");
            self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
                kind: "open_channel".to_string(),
                status_text: format!("Opened payment channel with {amount_sat} sat"),
                mint_url: json_string(wallet_send, "mint_url"),
                amount_sat,
                amount_text: paid_route_sat_text(amount_sat),
                fee_sat,
                fee_text: paid_route_fee_text(fee_sat),
                operation_id: json_string(wallet_send, "operation_id"),
                ..NativePaidRouteWalletActionState::default()
            };
        }
        Ok(())
    }

    pub(super) fn sign_paid_route_payment_envelope_from_wallet(
        &mut self,
        session_id: &str,
        kind: &str,
        delivered_units: Option<u64>,
        paid_msat: Option<u64>,
    ) -> Result<()> {
        let mut args = vec![
            "paid-exit".to_string(),
            "create-payment".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            session_id.trim().to_string(),
            "--kind".to_string(),
            kind.trim().to_string(),
            "--sign-from-wallet".to_string(),
        ];
        if let Some(delivered_units) = delivered_units {
            args.push("--delivered-units".to_string());
            args.push(delivered_units.to_string());
        }
        if let Some(paid_msat) = paid_msat {
            args.push("--paid-msat".to_string());
            args.push(paid_msat.to_string());
        }

        let output = self.run_nvpn_vec(&args)?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit create-payment")?;
        self.paid_route_payment_last_action = paid_route_payment_action_state("sign", &value)?;
        Ok(())
    }

    pub(super) fn close_paid_route_channel_from_wallet(
        &mut self,
        session_id: &str,
        publish: bool,
    ) -> Result<()> {
        let mut args = vec![
            "paid-exit".to_string(),
            "settle".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            session_id.trim().to_string(),
        ];
        if !publish {
            args.push("--no-publish".to_string());
        }

        let output = self.run_nvpn_vec(&args)?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit settle")?;
        self.paid_route_payment_last_action = paid_route_payment_settle_action_state(&value)?;
        Ok(())
    }

    pub(super) fn apply_paid_route_payment_envelope(
        &mut self,
        envelope_json: &str,
    ) -> Result<()> {
        let args = vec![
            "paid-exit".to_string(),
            "apply-payment".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            "--envelope-stdin".to_string(),
        ];
        let output = self.run_nvpn_vec_with_stdin(&args, envelope_json.as_bytes())?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit apply-payment")?;
        self.paid_route_payment_last_action = paid_route_payment_action_state("apply", &value)?;
        Ok(())
    }

    pub(super) fn send_paid_route_payment_envelope(
        &mut self,
        envelope_json: &str,
    ) -> Result<()> {
        let value = self.send_paid_route_payment_envelope_value(envelope_json)?;
        self.paid_route_payment_last_action = paid_route_payment_send_action_state(&value);
        Ok(())
    }

    fn send_paid_route_payment_envelope_value(
        &mut self,
        envelope_json: &str,
    ) -> Result<serde_json::Value> {
        let args = vec![
            "paid-exit".to_string(),
            "send-payment".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            "--envelope-stdin".to_string(),
        ];
        let output = self.run_nvpn_vec_with_stdin(&args, envelope_json.as_bytes())?;
        decode_paid_route_command_json_output(output, "nvpn paid-exit send-payment")
    }

    pub(super) fn stream_paid_route_payments(
        &mut self,
        publish: bool,
        min_increment_msat: u64,
        limit: u64,
    ) -> Result<()> {
        let mut args = vec![
            "paid-exit".to_string(),
            "stream-payments".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--json".to_string(),
            "--min-increment-msat".to_string(),
            min_increment_msat.to_string(),
        ];
        if publish {
            args.push("--publish".to_string());
        }
        if limit > 0 {
            args.push("--limit".to_string());
            args.push(limit.to_string());
        }
        let output = self.run_nvpn_vec(&args)?;
        let value =
            decode_paid_route_command_json_output(output, "nvpn paid-exit stream-payments")?;
        self.paid_route_payment_last_action = paid_route_payment_stream_action_state(&value)?;
        Ok(())
    }

    pub(super) fn receive_paid_route_payments(&mut self, duration_secs: u64) -> Result<()> {
        let duration_secs = duration_secs.clamp(1, 30).to_string();
        let output = self.run_nvpn([
            "paid-exit",
            "receive-payments",
            "--config",
            self.config_path_str()?,
            "--duration-secs",
            &duration_secs,
            "--json",
        ])?;
        let value =
            decode_paid_route_command_json_output(output, "nvpn paid-exit receive-payments")?;
        self.paid_route_payment_last_action = paid_route_payment_receive_action_state(&value)?;
        Ok(())
    }

    pub(super) fn collect_paid_exit_channel(&mut self, channel_id: &str) -> Result<()> {
        let channel_id = channel_id.trim();
        if channel_id.is_empty() {
            return Err(anyhow!("paid exit channel id is empty"));
        }
        let output = self.run_nvpn([
            "paid-exit",
            "collect",
            "--config",
            self.config_path_str()?,
            "--json",
            channel_id,
        ])?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit collect")?;
        self.paid_route_payment_last_action = paid_route_payment_collect_action_state(&value);
        if let Some(wallet_collect) = value
            .get("wallet_collect")
            .filter(|wallet_collect| !wallet_collect.is_null())
        {
            let amount_sat = json_u64(wallet_collect, "amount_sat");
            self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
                kind: "collect".to_string(),
                status_text: if amount_sat > 0 {
                    format!("Added {} to wallet", paid_route_sat_text(amount_sat))
                } else {
                    "Channel funds already in wallet".to_string()
                },
                mint_url: json_string(wallet_collect, "mint_url"),
                amount_sat,
                amount_text: paid_route_sat_text(amount_sat),
                ..NativePaidRouteWalletActionState::default()
            };
        }
        Ok(())
    }

    pub(super) fn collect_due_paid_exit_channels(&mut self) -> Result<()> {
        let output = self.run_nvpn([
            "paid-exit",
            "collect-due",
            "--config",
            self.config_path_str()?,
            "--json",
        ])?;
        let value = decode_paid_route_command_json_output(output, "nvpn paid-exit collect-due")?;
        self.paid_route_payment_last_action = paid_route_payment_collect_due_action_state(&value);
        let amount_sat = value
            .get("collected")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("wallet_collect"))
            .map(|wallet_collect| json_u64(wallet_collect, "amount_sat"))
            .fold(0_u64, u64::saturating_add);
        if amount_sat > 0 {
            self.paid_route_wallet_last_action = NativePaidRouteWalletActionState {
                kind: "collect".to_string(),
                status_text: format!("Added {} to wallet", paid_route_sat_text(amount_sat)),
                amount_sat,
                amount_text: paid_route_sat_text(amount_sat),
                ..NativePaidRouteWalletActionState::default()
            };
        }
        Ok(())
    }

    pub(super) fn publish_paid_exit_offer(&mut self) -> Result<()> {
        self.config.paid_exit.access.upstream = selected_paid_exit_upstream(&self.config);
        self.save_config()?;
        let output = self.run_nvpn([
            "paid-exit",
            "run",
            "--config",
            self.config_path_str()?,
            "--publish",
            "--json",
        ])?;
        ensure_success("nvpn paid-exit offer --publish", &output)
    }

    pub(super) fn discover_paid_route_offers(&mut self, duration_secs: u64) -> Result<()> {
        let duration_secs = duration_secs.clamp(1, 30).to_string();
        let rating_discovery = &self.config.paid_exit.rating_discovery;
        let mut args = vec![
            "paid-exit".to_string(),
            "discover".to_string(),
            "--config".to_string(),
            self.config_path_str()?.to_string(),
            "--duration-secs".to_string(),
            duration_secs,
            "--json".to_string(),
        ];
        if !rating_discovery.file.trim().is_empty() {
            args.push("--fips-peer-ratings".to_string());
            args.push(rating_discovery.file.clone());
        }
        for relay in &rating_discovery.relays {
            args.push("--fips-peer-ratings-relay".to_string());
            args.push(relay.clone());
        }
        if rating_discovery.configured() {
            args.push("--rating-scope".to_string());
            args.push(rating_discovery.scope.clone());
        }
        let output = self.run_nvpn_vec(&args)?;
        ensure_success("nvpn paid-exit discover", &output)
    }

    fn run_nvpn_vec(&self, args: &[String]) -> Result<Output> {
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

    fn run_nvpn_vec_with_stdin(&self, args: &[String], stdin_bytes: &[u8]) -> Result<Output> {
        let Some(nvpn_bin) = &self.nvpn_bin else {
            return Err(anyhow!(
                "nvpn CLI binary not found; set {NVPN_BIN_ENV} or install nvpn"
            ));
        };
        let mut child = Command::new(nvpn_bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .hide_console_window()
            .spawn()
            .with_context(|| format!("failed to execute {}", nvpn_bin.display()))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow!("failed to open nvpn stdin"))?;
            stdin
                .write_all(stdin_bytes)
                .context("failed to write nvpn stdin")?;
        }
        child
            .wait_with_output()
            .with_context(|| format!("failed to wait for {}", nvpn_bin.display()))
    }
}
