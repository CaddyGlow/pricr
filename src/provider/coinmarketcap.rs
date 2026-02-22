use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, trace};

use super::cache;
use super::{CoinPrice, HistoryInterval, PriceHistory, PricePoint, PriceProvider};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://pro-api.coinmarketcap.com/v1";
const WEB_CHART_BASE_URL: &str = "https://api.coinmarketcap.com/data-api/v3.3";
const COIN_SUMMARIES_URL: &str = "https://s3.coinmarketcap.com/whitepaper/summaries/coins.json";
const CATALOG_CACHE_TTL_SECS: i64 = 24 * 60 * 60;
const DAILY_CHART_CACHE_TTL_SECS: i64 = 12 * 60 * 60;
const PRICE_CACHE_TTL_SECS: i64 = 30;
const HOURLY_CHART_CACHE_TTL_SECS: i64 = 60 * 60;

/// CoinMarketCap price provider -- requires an API key.
pub struct CoinMarketCap {
    client: Client,
    api_key: Option<String>,
    base_url: String,
    chart_base_url: String,
    coin_summaries_url: String,
    coin_catalog: RwLock<Option<HashMap<String, (u64, String)>>>,
}

impl CoinMarketCap {
    /// Create a CoinMarketCap provider using the default production API URL.
    pub fn new(api_key: String) -> Self {
        Self::with_optional_key(
            Some(api_key),
            BASE_URL,
            WEB_CHART_BASE_URL,
            COIN_SUMMARIES_URL,
        )
    }

    /// Create a CoinMarketCap provider without an API key.
    pub fn without_key() -> Self {
        Self::with_optional_key(None, BASE_URL, WEB_CHART_BASE_URL, COIN_SUMMARIES_URL)
    }

    /// Create a CoinMarketCap provider with a custom base URL.
    pub fn with_base_url(api_key: String, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let chart_base_url = derive_chart_base_url(&base_url);
        let coin_summaries_url = derive_coin_summaries_url(&chart_base_url);
        Self::with_optional_key(Some(api_key), base_url, chart_base_url, coin_summaries_url)
    }

    fn with_optional_key(
        api_key: Option<String>,
        base_url: impl Into<String>,
        chart_base_url: impl Into<String>,
        coin_summaries_url: impl Into<String>,
    ) -> Self {
        let client = Client::builder()
            .user_agent("cryptoprice/0.1.0")
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            api_key,
            base_url: base_url.into(),
            chart_base_url: chart_base_url.into(),
            coin_summaries_url: coin_summaries_url.into(),
            coin_catalog: RwLock::new(None),
        }
    }

    fn required_api_key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            Error::Config(
                "CoinMarketCap price lookup requires --api-key or COINMARKETCAP_API_KEY".into(),
            )
        })
    }

    fn coin_catalog_cache_key(&self) -> String {
        format!("coin_summaries:{}", self.coin_summaries_url)
    }

    fn chart_cache_key(
        &self,
        coin_id: u64,
        convert_id: u64,
        interval: &str,
        range: &str,
    ) -> String {
        format!(
            "chart:{}:{}:{}:{}:{}",
            self.chart_base_url,
            coin_id,
            convert_id,
            interval,
            range.to_ascii_lowercase()
        )
    }
}

#[derive(Debug, Deserialize)]
struct CmcCoin {
    name: String,
    symbol: String,
    quote: HashMap<String, CmcQuote>,
}

#[derive(Debug, Deserialize)]
struct CmcQuote {
    price: Option<f64>,
    percent_change_24h: Option<f64>,
    market_cap: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CmcRawResponse {
    data: HashMap<String, serde_json::Value>,
    status: Option<CmcStatus>,
}

#[derive(Debug, Deserialize)]
struct CmcStatus {
    error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CmcHistoryRawResponse {
    data: serde_json::Value,
    status: Option<CmcStatus>,
}

#[derive(Debug, Deserialize)]
struct CmcWebChartResponse {
    data: CmcWebChartData,
}

#[derive(Debug, Deserialize)]
struct CmcWebChartData {
    points: Vec<CmcWebChartPoint>,
}

#[derive(Debug, Deserialize)]
struct CmcWebChartPoint {
    #[serde(rename = "s")]
    ts_seconds: String,
    #[serde(rename = "v")]
    values: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct CmcCoinSummary {
    symbol: String,
    name: String,
    id: u64,
}

struct WebChartRequest<'a> {
    symbol_upper: &'a str,
    display_name: &'a str,
    convert: &'a str,
    days: u32,
    coin_id: u64,
    convert_id: u64,
    interval: &'a str,
    range: &'a str,
}

#[async_trait]
impl PriceProvider for CoinMarketCap {
    fn name(&self) -> &str {
        "CoinMarketCap"
    }

    fn id(&self) -> &str {
        "cmc"
    }

    async fn get_prices(&self, symbols: &[String], currency: &str) -> Result<Vec<CoinPrice>> {
        let api_key = self.required_api_key()?;
        let symbols_upper: Vec<String> = symbols.iter().map(|s| s.to_uppercase()).collect();
        let symbols_joined = symbols_upper.join(",");
        let convert = currency.to_uppercase();

        let url = format!(
            "{}/cryptocurrency/quotes/latest?symbol={}&convert={}",
            self.base_url, symbols_joined, convert
        );
        let cache_key = format!(
            "quotes_latest:{}:{}:{}",
            self.base_url, symbols_joined, convert
        );

        debug!(url = %url, "fetching prices from CoinMarketCap");

        let body = if let Some(cached_body) =
            cache::read_json::<String>("coinmarketcap", &cache_key, PRICE_CACHE_TTL_SECS).await
        {
            debug!(symbols = %symbols_joined, currency = %convert, "using cached CoinMarketCap quotes");
            cached_body
        } else {
            let resp = self
                .client
                .get(&url)
                .header("X-CMC_PRO_API_KEY", api_key)
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(status = %status, body_len = body.len(), "CoinMarketCap response");
            trace!(body = %body, "CoinMarketCap response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "CoinMarketCap returned {}: {}",
                    status, body
                )));
            }

            cache::write_json("coinmarketcap", &cache_key, &body).await;
            body
        };

        let raw: CmcRawResponse =
            serde_json::from_str(&body).map_err(|e| Error::Parse(format!("CMC JSON: {}", e)))?;

        if let Some(ref st) = raw.status
            && let Some(ref msg) = st.error_message
            && !msg.is_empty()
        {
            return Err(Error::Api(format!("CoinMarketCap: {}", msg)));
        }

        let mut results = Vec::new();
        for sym in &symbols_upper {
            if let Some(val) = raw.data.get(sym.as_str()) {
                // CMC may return a single coin object or an array for duplicate symbols.
                let coin: CmcCoin = if val.is_array() {
                    let coins: Vec<CmcCoin> = serde_json::from_value(val.clone())
                        .map_err(|e| Error::Parse(format!("CMC coin array: {}", e)))?;
                    match coins.into_iter().next() {
                        Some(c) => c,
                        None => continue,
                    }
                } else {
                    serde_json::from_value(val.clone())
                        .map_err(|e| Error::Parse(format!("CMC coin: {}", e)))?
                };

                if let Some(quote) = coin.quote.get(&convert) {
                    results.push(CoinPrice {
                        symbol: coin.symbol.clone(),
                        name: coin.name.clone(),
                        price: quote.price.unwrap_or(0.0),
                        change_24h: quote.percent_change_24h,
                        market_cap: quote.market_cap,
                        currency: convert.clone(),
                        provider: self.name().to_string(),
                        timestamp: chrono::Utc::now(),
                    });
                }
            }
        }

        if results.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(results)
    }

    async fn get_price_history(
        &self,
        symbols: &[String],
        currency: &str,
        days: u32,
        interval: HistoryInterval,
    ) -> Result<Vec<PriceHistory>> {
        let convert = currency.to_uppercase();
        let interval_param = match interval {
            HistoryInterval::Auto => {
                if days <= 30 {
                    "hourly"
                } else {
                    "daily"
                }
            }
            HistoryInterval::Hourly => "hourly",
            HistoryInterval::Daily => "daily",
        };

        let futures = symbols
            .iter()
            .map(|symbol| self.fetch_history_for_symbol(symbol, &convert, days, interval_param));

        let mut histories = Vec::new();
        for result in join_all(futures).await {
            histories.push(result?);
        }

        if histories.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(histories)
    }
}

impl CoinMarketCap {
    async fn fetch_history_for_symbol(
        &self,
        symbol: &str,
        convert: &str,
        days: u32,
        interval_param: &str,
    ) -> Result<PriceHistory> {
        let symbol_upper = symbol.to_uppercase();

        if let (Some((coin_id, display_name)), Some(convert_id)) = (
            self.resolve_coin_for_web_chart(&symbol_upper).await,
            cmc_convert_id(convert),
        ) {
            let web_interval = to_web_interval(interval_param);
            let web_range = to_web_range(days);

            match self
                .fetch_history_via_web_chart(WebChartRequest {
                    symbol_upper: &symbol_upper,
                    display_name: &display_name,
                    convert,
                    days,
                    coin_id,
                    convert_id,
                    interval: web_interval,
                    range: web_range,
                })
                .await
            {
                Ok(history) => return Ok(history),
                Err(err) => {
                    debug!(
                        symbol = %symbol_upper,
                        currency = %convert,
                        error = %err,
                        "CoinMarketCap web chart endpoint failed; falling back to pro historical endpoint"
                    );
                }
            }
        }

        self.fetch_history_via_pro_api(&symbol_upper, convert, days, interval_param)
            .await
    }

    async fn resolve_coin_for_web_chart(&self, symbol_upper: &str) -> Option<(u64, String)> {
        if let Some(found) = self.lookup_coin_in_catalog(symbol_upper).await {
            return Some(found);
        }

        cmc_coin_for_symbol(symbol_upper).map(|(id, name)| (id, name.to_string()))
    }

    async fn lookup_coin_in_catalog(&self, symbol_upper: &str) -> Option<(u64, String)> {
        {
            let guard = self.coin_catalog.read().await;
            if let Some(catalog) = guard.as_ref() {
                return catalog.get(symbol_upper).cloned();
            }
        }

        let mut guard = self.coin_catalog.write().await;
        if guard.is_none() {
            match self.fetch_coin_catalog().await {
                Ok(catalog) => {
                    *guard = Some(catalog);
                }
                Err(err) => {
                    debug!(
                        url = %self.coin_summaries_url,
                        error = %err,
                        "failed to fetch CoinMarketCap coin catalog"
                    );
                    *guard = Some(HashMap::new());
                }
            }
        }

        guard
            .as_ref()
            .and_then(|catalog| catalog.get(symbol_upper))
            .cloned()
    }

    async fn fetch_coin_catalog(&self) -> Result<HashMap<String, (u64, String)>> {
        let catalog_cache_key = self.coin_catalog_cache_key();

        if let Some(cached_body) =
            cache::read_json::<String>("coinmarketcap", &catalog_cache_key, CATALOG_CACHE_TTL_SECS)
                .await
        {
            debug!("using cached CoinMarketCap coin catalog");

            if let Ok(catalog) = parse_coin_catalog(&cached_body) {
                return Ok(catalog);
            }

            debug!("cached CoinMarketCap coin catalog is invalid; refetching");
        }

        let resp = self.client.get(&self.coin_summaries_url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;

        debug!(
            url = %self.coin_summaries_url,
            status = %status,
            body_len = body.len(),
            "CoinMarketCap coin catalog response"
        );

        if !status.is_success() {
            return Err(Error::Api(format!(
                "CoinMarketCap coin catalog returned {}: {}",
                status, body
            )));
        }

        cache::write_json("coinmarketcap", &catalog_cache_key, &body).await;

        parse_coin_catalog(&body)
    }

    async fn fetch_history_via_web_chart(&self, req: WebChartRequest<'_>) -> Result<PriceHistory> {
        let url = format!(
            "{}/cryptocurrency/detail/chart?id={}&interval={}&convertId={}&range={}",
            self.chart_base_url, req.coin_id, req.interval, req.convert_id, req.range
        );

        debug!(
            url = %url,
            symbol = %req.symbol_upper,
            currency = %req.convert,
            interval = req.interval,
            range = req.range,
            "fetching chart data from CoinMarketCap web endpoint"
        );

        let cache_key = self.chart_cache_key(req.coin_id, req.convert_id, req.interval, req.range);
        let cache_ttl = chart_ttl(req.interval);

        let body = if let Some(cached_body) =
            cache::read_json::<String>("coinmarketcap", &cache_key, cache_ttl).await
        {
            debug!(symbol = %req.symbol_upper, interval = req.interval, "using cached CoinMarketCap web chart response");
            cached_body
        } else {
            let fetched = self.fetch_web_chart_body(&url, req.symbol_upper).await?;
            cache::write_json("coinmarketcap", &cache_key, &fetched).await;
            fetched
        };

        let raw: CmcWebChartResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("CMC web chart JSON: {}", e)))?;

        let mut points = Vec::new();
        for point in raw.data.points {
            let ts_seconds = match point.ts_seconds.parse::<i64>() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let price = match point.values.first().copied() {
                Some(v) if v.is_finite() => v,
                _ => continue,
            };

            let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp(ts_seconds, 0)
            else {
                continue;
            };

            points.push(PricePoint { timestamp, price });
        }

        points.sort_by_key(|p| p.timestamp);

        trim_points_to_days(&mut points, req.days);

        if points.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(PriceHistory {
            symbol: req.symbol_upper.to_string(),
            name: req.display_name.to_string(),
            currency: req.convert.to_uppercase(),
            provider: "CoinMarketCap".to_string(),
            points,
        })
    }

    async fn fetch_web_chart_body(&self, url: &str, symbol_upper: &str) -> Result<String> {
        let resp = self
            .client
            .get(url)
            .header("accept", "application/json, text/plain, */*")
            .header("platform", "web")
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        debug!(
            status = %status,
            body_len = body.len(),
            symbol = %symbol_upper,
            "CoinMarketCap web chart response"
        );
        trace!(body = %body, symbol = %symbol_upper, "CoinMarketCap web chart response body");

        if !status.is_success() {
            return Err(Error::Api(format!(
                "CoinMarketCap web chart returned {}: {}",
                status, body
            )));
        }

        Ok(body)
    }

    async fn fetch_history_via_pro_api(
        &self,
        symbol_upper: &str,
        convert: &str,
        days: u32,
        interval_param: &str,
    ) -> Result<PriceHistory> {
        let api_key = self.required_api_key()?;
        let time_end = chrono::Utc::now();
        let time_start = time_end - chrono::Duration::days(days as i64);
        let url = format!(
            "{}/cryptocurrency/quotes/historical?symbol={}&convert={}&time_start={}&time_end={}&interval={}",
            self.base_url,
            symbol_upper,
            convert,
            time_start.to_rfc3339(),
            time_end.to_rfc3339(),
            interval_param
        );
        let cache_key = format!(
            "quotes_historical:{}:{}:{}:{}:{}",
            self.base_url, symbol_upper, convert, days, interval_param
        );
        let history_ttl = chart_ttl(interval_param);

        debug!(
            url = %url,
            symbol = %symbol_upper,
            currency = %convert,
            days,
            interval = %interval_param,
            "fetching chart data from CoinMarketCap"
        );

        let body = if let Some(cached_body) =
            cache::read_json::<String>("coinmarketcap", &cache_key, history_ttl).await
        {
            debug!(symbol = %symbol_upper, currency = %convert, "using cached CoinMarketCap pro history");
            cached_body
        } else {
            let resp = self
                .client
                .get(&url)
                .header("X-CMC_PRO_API_KEY", api_key)
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(
                status = %status,
                body_len = body.len(),
                symbol = %symbol_upper,
                "CoinMarketCap chart response"
            );
            trace!(body = %body, symbol = %symbol_upper, "CoinMarketCap chart response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "CoinMarketCap returned {} for chart data: {}",
                    status, body
                )));
            }

            cache::write_json("coinmarketcap", &cache_key, &body).await;
            body
        };

        let raw: CmcHistoryRawResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("CMC history JSON: {}", e)))?;

        if let Some(ref st) = raw.status
            && let Some(ref msg) = st.error_message
            && !msg.is_empty()
        {
            return Err(Error::Api(format!("CoinMarketCap: {}", msg)));
        }

        parse_history_data(raw.data, symbol_upper, convert)
    }
}

fn derive_chart_base_url(base_url: &str) -> String {
    if let Some(prefix) = base_url.strip_suffix("/v1") {
        return format!("{}/data-api/v3.3", prefix.trim_end_matches('/'));
    }

    format!("{}/data-api/v3.3", base_url.trim_end_matches('/'))
}

fn derive_coin_summaries_url(chart_base_url: &str) -> String {
    if let Some((origin, _)) = chart_base_url.split_once("/data-api/") {
        return format!(
            "{}/whitepaper/summaries/coins.json",
            origin.trim_end_matches('/')
        );
    }

    COIN_SUMMARIES_URL.to_string()
}

fn to_web_interval(interval: &str) -> &str {
    match interval {
        "hourly" => "1h",
        _ => "1d",
    }
}

fn to_web_range(days: u32) -> &'static str {
    match days {
        1 => "1D",
        2..=7 => "7D",
        8..=30 => "1M",
        31..=90 => "3M",
        91..=180 => "6M",
        _ => "1Y",
    }
}

fn chart_ttl(interval: &str) -> i64 {
    match interval {
        "1d" | "daily" => DAILY_CHART_CACHE_TTL_SECS,
        _ => HOURLY_CHART_CACHE_TTL_SECS,
    }
}

fn trim_points_to_days(points: &mut Vec<PricePoint>, days: u32) {
    if points.is_empty() || days == 0 {
        return;
    }

    let Some(last) = points.last().map(|p| p.timestamp) else {
        return;
    };
    let cutoff = last - chrono::Duration::days(days as i64);
    points.retain(|p| p.timestamp >= cutoff);
}

fn cmc_convert_id(convert: &str) -> Option<u64> {
    match convert {
        "USD" => Some(2781),
        _ => None,
    }
}

fn cmc_coin_for_symbol(symbol_upper: &str) -> Option<(u64, &'static str)> {
    match symbol_upper {
        "BTC" => Some((1, "Bitcoin")),
        "ETH" => Some((1027, "Ethereum")),
        "USDT" => Some((825, "Tether")),
        "BNB" => Some((1839, "BNB")),
        "SOL" => Some((5426, "Solana")),
        "XRP" => Some((52, "XRP")),
        "USDC" => Some((3408, "USDC")),
        "ADA" => Some((2010, "Cardano")),
        "DOGE" => Some((74, "Dogecoin")),
        "DOT" => Some((6636, "Polkadot")),
        "MATIC" => Some((3890, "Polygon")),
        "LTC" => Some((2, "Litecoin")),
        "AVAX" => Some((5805, "Avalanche")),
        "LINK" => Some((1975, "Chainlink")),
        "ATOM" => Some((3794, "Cosmos")),
        "UNI" => Some((7083, "Uniswap")),
        "XLM" => Some((512, "Stellar")),
        "SHIB" => Some((5994, "Shiba Inu")),
        "TRX" => Some((1958, "TRON")),
        "TON" => Some((11419, "Toncoin")),
        "PEPE" => Some((24478, "Pepe")),
        "NEAR" => Some((6535, "NEAR")),
        "APT" => Some((21794, "Aptos")),
        "ARB" => Some((11841, "Arbitrum")),
        "OP" => Some((11840, "Optimism")),
        "SUI" => Some((20947, "Sui")),
        _ => None,
    }
}

fn parse_coin_catalog(body: &str) -> Result<HashMap<String, (u64, String)>> {
    let entries: Vec<CmcCoinSummary> = serde_json::from_str(body)
        .map_err(|e| Error::Parse(format!("CMC coin catalog JSON: {}", e)))?;

    let mut catalog = HashMap::new();
    for entry in entries {
        catalog
            .entry(entry.symbol.to_uppercase())
            .or_insert((entry.id, entry.name));
    }

    Ok(catalog)
}

fn parse_history_data(
    data: serde_json::Value,
    symbol_upper: &str,
    convert: &str,
) -> Result<PriceHistory> {
    let payload = history_payload_for_symbol(&data, symbol_upper)
        .ok_or_else(|| Error::Parse("CMC history response missing payload".to_string()))?;

    let name = payload
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string)
        .unwrap_or_else(|| symbol_upper.to_string());

    let symbol = payload
        .get("symbol")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(symbol_upper)
        .to_uppercase();

    let quotes = payload
        .get("quotes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Parse("CMC history response missing quotes".to_string()))?;

    let mut points = Vec::new();
    for quote in quotes {
        let ts_raw = match quote.get("timestamp").and_then(serde_json::Value::as_str) {
            Some(ts) => ts,
            None => continue,
        };

        let timestamp = match chrono::DateTime::parse_from_rfc3339(ts_raw) {
            Ok(ts) => ts.with_timezone(&chrono::Utc),
            Err(_) => continue,
        };

        let quote_obj = match quote.get("quote").and_then(serde_json::Value::as_object) {
            Some(obj) => obj,
            None => continue,
        };

        let price = quote_obj
            .get(convert)
            .or_else(|| quote_obj.get(&convert.to_lowercase()))
            .and_then(|v| v.get("price"))
            .and_then(serde_json::Value::as_f64);

        let Some(price) = price else {
            continue;
        };

        if !price.is_finite() {
            continue;
        }

        points.push(PricePoint { timestamp, price });
    }

    points.sort_by_key(|p| p.timestamp);

    if points.is_empty() {
        return Err(Error::NoResults);
    }

    Ok(PriceHistory {
        symbol,
        name,
        currency: convert.to_uppercase(),
        provider: "CoinMarketCap".to_string(),
        points,
    })
}

fn history_payload_for_symbol<'a>(
    data: &'a serde_json::Value,
    symbol_upper: &str,
) -> Option<&'a serde_json::Value> {
    if data.get("quotes").is_some() {
        return Some(data);
    }

    if let Some(by_symbol) = data.get(symbol_upper) {
        if by_symbol.get("quotes").is_some() {
            return Some(by_symbol);
        }

        if let Some(arr) = by_symbol.as_array()
            && let Some(first) = arr.first()
        {
            return Some(first);
        }
    }

    None
}
