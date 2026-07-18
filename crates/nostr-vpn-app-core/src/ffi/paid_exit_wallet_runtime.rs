#[derive(Debug)]
pub(super) struct PaidRouteWalletRuntime {
    runtime: tokio::runtime::Runtime,
    service: cashu_service::CashuWalletService,
}

impl PaidRouteWalletRuntime {
    pub(super) fn open(config_path: &Path) -> Result<Self> {
        let data_dir = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to create the Cashu wallet async runtime")?;

        #[cfg(any(target_os = "ios", target_os = "android"))]
        let service = runtime.block_on(cashu_service::CashuWalletService::open_with_seed_store(
            data_dir,
            std::sync::Arc::new(nostr_vpn_core::PlatformCashuWalletSeedStore::new(
                config_path,
            )),
        ))?;
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        let service = runtime.block_on(cashu_service::CashuWalletService::open_file_backed(data_dir))?;

        let recovery = runtime.block_on(service.recover_startup_state());
        for warning in recovery.warnings {
            tracing::warn!(warning, "Cashu wallet startup recovery is incomplete");
        }

        Ok(Self { runtime, service })
    }

    pub(super) fn overview(
        &self,
        refresh_quotes: bool,
    ) -> Result<cashu_service::CashuWalletOverview> {
        self.runtime
            .block_on(self.service.load_wallet_overview(refresh_quotes))
    }

    pub(super) fn activity(&self) -> Result<Vec<cashu_service::CashuWalletActivityEntry>> {
        self.runtime.block_on(self.service.load_wallet_activity())
    }

    pub(super) fn create_topup_quote(
        &self,
        mint_url: &str,
        amount_sat: u64,
    ) -> Result<cashu_service::CashuTopupQuote> {
        self.runtime
            .block_on(self.service.create_topup_quote(mint_url, amount_sat))
    }

    pub(super) fn receive_token(
        &self,
        token: &str,
    ) -> Result<cashu_service::CashuReceivedPayment> {
        self.runtime
            .block_on(self.service.receive_payment_token(token))
    }

    pub(super) fn send_token(
        &self,
        mint_url: &str,
        amount_sat: u64,
    ) -> Result<cashu_service::CashuSentPayment> {
        self.runtime
            .block_on(self.service.send_payment_token(mint_url, amount_sat))
    }

    pub(super) fn pay_lightning(
        &self,
        mint_url: &str,
        invoice: &str,
    ) -> Result<cashu_service::CashuLightningPayment> {
        self.runtime
            .block_on(self.service.send_lightning_payment(mint_url, invoice))
    }

    pub(super) fn open_spilman_channel(
        &self,
        request: cashu_service::StreamingRouteOpenCashuSpilmanChannelFromWalletRequest,
    ) -> Result<cashu_service::StreamingRouteOpenCashuSpilmanChannelFromWalletResult> {
        self.runtime
            .block_on(self.service.open_streaming_route_cashu_spilman_channel(request))
    }
}
