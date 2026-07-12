use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

pub const REFRESH_CADENCE: Duration = Duration::from_mins(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SOURCE_SPREAD: f64 = 0.05;
const COINBASE_URL: &str = "https://api.coinbase.com/v2/exchange-rates";
const KRAKEN_URL: &str = "https://api.kraken.com/0/public/Ticker";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FiatCurrency {
    Usd,
    Eur,
    Gbp,
    Cad,
    Aud,
    Jpy,
    Chf,
}

impl FiatCurrency {
    pub const ALL: [Self; 7] = [
        Self::Usd,
        Self::Eur,
        Self::Gbp,
        Self::Cad,
        Self::Aud,
        Self::Jpy,
        Self::Chf,
    ];

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Cad => "CAD",
            Self::Aud => "AUD",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedCurrency(String);

impl UnsupportedCurrency {
    #[must_use]
    pub fn code(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnsupportedCurrency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported fiat currency: {}", self.0)
    }
}

impl std::error::Error for UnsupportedCurrency {}

impl TryFrom<&str> for FiatCurrency {
    type Error = UnsupportedCurrency;

    fn try_from(code: &str) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|currency| currency.code().eq_ignore_ascii_case(code))
            .ok_or_else(|| UnsupportedCurrency(code.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExchangeRateSource {
    Coinbase,
    Kraken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExchangeRateStatus {
    Unavailable,
    Refreshing,
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExchangeRateSnapshot {
    pub currency: FiatCurrency,
    pub rate: Option<f64>,
    pub sources: Vec<ExchangeRateSource>,
    pub status: ExchangeRateStatus,
    pub timestamp: Option<SystemTime>,
    pub stale: bool,
}

#[derive(Clone)]
pub struct ExchangeRateService {
    inner: Arc<Inner>,
}

struct Inner {
    currency: FiatCurrency,
    client: reqwest::Client,
    state: Mutex<State>,
}

struct State {
    rate: Option<f64>,
    sources: Vec<ExchangeRateSource>,
    status: ExchangeRateStatus,
    timestamp: Option<SystemTime>,
    refreshed_at: Option<Instant>,
    next_refresh_at: Option<Instant>,
    refreshing: bool,
}

impl ExchangeRateService {
    pub fn new(currency: &str) -> Result<Self, UnsupportedCurrency> {
        Ok(Self::for_currency(currency.try_into()?))
    }

    #[must_use]
    pub fn for_currency(currency: FiatCurrency) -> Self {
        Self {
            inner: Arc::new(Inner {
                currency,
                client: reqwest::Client::new(),
                state: Mutex::new(State {
                    rate: None,
                    sources: Vec::new(),
                    status: ExchangeRateStatus::Unavailable,
                    timestamp: None,
                    refreshed_at: None,
                    next_refresh_at: None,
                    refreshing: false,
                }),
            }),
        }
    }

    #[must_use]
    pub fn refresh_if_due(&self) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let now = Instant::now();
        {
            let mut state = self.inner.state();
            if state.refreshing
                || state
                    .next_refresh_at
                    .is_some_and(|next_refresh| now < next_refresh)
            {
                return false;
            }
            state.refreshing = true;
            state.next_refresh_at = Some(now + REFRESH_CADENCE);
            state.status = ExchangeRateStatus::Refreshing;
        }
        let inner = Arc::clone(&self.inner);
        handle.spawn(async move { inner.refresh().await });
        true
    }

    #[must_use]
    pub fn snapshot(&self) -> ExchangeRateSnapshot {
        let state = self.inner.state();
        let snapshot_is_stale = state.refreshed_at.is_none_or(|refreshed_at| {
            Instant::now().saturating_duration_since(refreshed_at) >= REFRESH_CADENCE
        });
        ExchangeRateSnapshot {
            currency: self.inner.currency,
            rate: state.rate,
            sources: state.sources.clone(),
            status: state.status,
            timestamp: state.timestamp,
            stale: snapshot_is_stale,
        }
    }
}

impl Inner {
    async fn refresh(self: Arc<Self>) {
        let result = fetch_rates(&self.client, self.currency).await;
        let mut state = self.state();
        state.refreshing = false;
        match result {
            Ok((rate, sources)) => {
                state.rate = Some(rate);
                state.sources = sources;
                state.status = ExchangeRateStatus::Ready;
                state.timestamp = Some(SystemTime::now());
                state.refreshed_at = Some(Instant::now());
            }
            Err(_) => state.status = ExchangeRateStatus::Failed,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Deserialize)]
struct CoinbaseResponse {
    data: CoinbaseData,
}

#[derive(Deserialize)]
struct CoinbaseData {
    currency: String,
    rates: HashMap<String, String>,
}

#[derive(Deserialize)]
struct KrakenResponse {
    error: Vec<String>,
    #[serde(default)]
    result: HashMap<String, KrakenTicker>,
}

#[derive(Deserialize)]
struct KrakenTicker {
    #[serde(rename = "c")]
    last_trade: [String; 2],
}

async fn fetch_rates(
    client: &reqwest::Client,
    currency: FiatCurrency,
) -> Result<(f64, Vec<ExchangeRateSource>)> {
    let (coinbase, kraken) = tokio::join!(
        tokio::time::timeout(REQUEST_TIMEOUT, fetch_coinbase(client, currency)),
        tokio::time::timeout(REQUEST_TIMEOUT, fetch_kraken(client, currency)),
    );
    combine_rates(
        coinbase.ok().and_then(Result::ok),
        kraken.ok().and_then(Result::ok),
    )
}

async fn fetch_coinbase(client: &reqwest::Client, currency: FiatCurrency) -> Result<f64> {
    let payload = client
        .get(COINBASE_URL)
        .query(&[("currency", "BTC")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_coinbase(&payload, currency)
}

async fn fetch_kraken(client: &reqwest::Client, currency: FiatCurrency) -> Result<f64> {
    let pair = format!("XBT{}", currency.code());
    let payload = client
        .get(KRAKEN_URL)
        .query(&[("pair", pair)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_kraken(&payload)
}

fn parse_coinbase(payload: &str, currency: FiatCurrency) -> Result<f64> {
    let response: CoinbaseResponse = serde_json::from_str(payload)?;
    if response.data.currency != "BTC" {
        bail!("unexpected Coinbase base currency");
    }
    let value = response
        .data
        .rates
        .get(currency.code())
        .ok_or_else(|| anyhow!("Coinbase rate is missing"))?;
    parse_rate(value)
}

fn parse_kraken(payload: &str) -> Result<f64> {
    let response: KrakenResponse = serde_json::from_str(payload)?;
    if !response.error.is_empty() {
        bail!("Kraken returned an API error");
    }
    if response.result.len() != 1 {
        bail!("Kraken returned an unexpected number of pairs");
    }
    let ticker = response
        .result
        .into_values()
        .next()
        .ok_or_else(|| anyhow!("Kraken rate is missing"))?;
    parse_rate(&ticker.last_trade[0])
}

fn combine_rates(
    coinbase: Option<f64>,
    kraken: Option<f64>,
) -> Result<(f64, Vec<ExchangeRateSource>)> {
    let mut rates = Vec::with_capacity(2);
    if let Some(rate) = coinbase.filter(|rate| is_valid_rate(*rate)) {
        rates.push((ExchangeRateSource::Coinbase, rate));
    }
    if let Some(rate) = kraken.filter(|rate| is_valid_rate(*rate)) {
        rates.push((ExchangeRateSource::Kraken, rate));
    }
    match rates.as_slice() {
        [] => bail!("no valid exchange-rate sources"),
        [(source, rate)] => Ok((*rate, vec![*source])),
        [(first_source, first), (second_source, second)] => {
            let (low, high) = if first <= second {
                (*first, *second)
            } else {
                (*second, *first)
            };
            if (high - low) / low > MAX_SOURCE_SPREAD {
                bail!("exchange-rate source spread exceeds limit");
            }
            let rate = first / 2.0 + second / 2.0;
            if !is_valid_rate(rate) {
                bail!("combined exchange rate is invalid");
            }
            Ok((rate, vec![*first_source, *second_source]))
        }
        _ => unreachable!("only two providers are configured"),
    }
}

fn parse_rate(value: &str) -> Result<f64> {
    let rate = value.parse::<f64>()?;
    if !is_valid_rate(rate) {
        bail!("exchange rate must be finite and positive");
    }
    Ok(rate)
}

fn is_valid_rate(rate: f64) -> bool {
    rate.is_finite() && rate > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rate_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_supported_currency_case_insensitively() {
        assert_eq!(FiatCurrency::try_from("eur"), Ok(FiatCurrency::Eur));
        assert_eq!(FiatCurrency::try_from("NZD").unwrap_err().code(), "NZD");
    }

    #[test]
    fn parses_selected_coinbase_rate() {
        let payload = r#"{"data":{"currency":"BTC","rates":{"EUR":"58421.25","USD":"63000"}}}"#;
        assert_rate_eq(
            parse_coinbase(payload, FiatCurrency::Eur).unwrap(),
            58_421.25,
        );
    }

    #[test]
    fn rejects_invalid_coinbase_payloads() {
        let wrong_base = r#"{"data":{"currency":"ETH","rates":{"USD":"3000"}}}"#;
        let invalid_rate = r#"{"data":{"currency":"BTC","rates":{"USD":"NaN"}}}"#;
        assert!(parse_coinbase(wrong_base, FiatCurrency::Usd).is_err());
        assert!(parse_coinbase(invalid_rate, FiatCurrency::Usd).is_err());
    }

    #[test]
    fn parses_dynamic_kraken_pair_key() {
        let payload = r#"{"error":[],"result":{"XXBTZCAD":{"c":["86000.5","0.01"]}}}"#;
        assert_rate_eq(parse_kraken(payload).unwrap(), 86_000.5);
    }

    #[test]
    fn rejects_kraken_errors_and_ambiguous_results() {
        let api_error = r#"{"error":["EQuery:Unknown asset pair"],"result":{}}"#;
        let ambiguous = r#"{"error":[],"result":{"XBTUSD":{"c":["62000","1"]},"XXBTZUSD":{"c":["62001","1"]}}}"#;
        assert!(parse_kraken(api_error).is_err());
        assert!(parse_kraken(ambiguous).is_err());
    }

    #[test]
    fn averages_consistent_sources() {
        let (rate, sources) = combine_rates(Some(60_000.0), Some(61_000.0)).unwrap();
        assert_rate_eq(rate, 60_500.0);
        assert_eq!(
            sources,
            vec![ExchangeRateSource::Coinbase, ExchangeRateSource::Kraken]
        );
    }

    #[test]
    fn accepts_one_valid_source() {
        let (rate, sources) = combine_rates(Some(f64::INFINITY), Some(61_000.0)).unwrap();
        assert_rate_eq(rate, 61_000.0);
        assert_eq!(sources, vec![ExchangeRateSource::Kraken]);
    }

    #[test]
    fn rejects_excessive_spread_or_no_valid_source() {
        assert!(combine_rates(Some(60_000.0), Some(63_000.0)).is_ok());
        assert!(combine_rates(Some(60_000.0), Some(63_001.0)).is_err());
        assert!(combine_rates(Some(0.0), Some(f64::NAN)).is_err());
    }
}
