use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, trace};

use super::cache;
use super::{CoinPrice, HistoryInterval, PriceHistory, PricePoint, PriceProvider};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.coingecko.com/api/v3";
const PRICE_CACHE_TTL_SECS: i64 = 30;
const HOURLY_HISTORY_CACHE_TTL_SECS: i64 = 60 * 60;
const DAILY_HISTORY_CACHE_TTL_SECS: i64 = 12 * 60 * 60;

/// CoinGecko price provider -- free public API, no key required.
pub struct CoinGecko {
    client: Client,
    base_url: String,
}

impl CoinGecko {
    /// Create a CoinGecko provider using the default production API URL.
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    /// Create a CoinGecko provider with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .user_agent("cryptoprice/0.1.0")
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    /// Map common ticker symbols to (CoinGecko API id, display name).
    fn resolve(symbol: &str) -> (String, String) {
        let lower = symbol.to_lowercase();
        let (id, name) = match lower.as_str() {
            "btc" | "bitcoin" => ("bitcoin", "Bitcoin"),
            "eth" | "ethereum" => ("ethereum", "Ethereum"),
            "usdt" | "tether" => ("tether", "Tether"),
            "bnb" => ("binancecoin", "BNB"),
            "sol" | "solana" => ("solana", "Solana"),
            "xrp" | "ripple" => ("ripple", "XRP"),
            "usdc" => ("usd-coin", "USDC"),
            "ada" | "cardano" => ("cardano", "Cardano"),
            "doge" | "dogecoin" => ("dogecoin", "Dogecoin"),
            "dot" | "polkadot" => ("polkadot", "Polkadot"),
            "matic" | "polygon" => ("matic-network", "Polygon"),
            "ltc" | "litecoin" => ("litecoin", "Litecoin"),
            "avax" | "avalanche" => ("avalanche-2", "Avalanche"),
            "link" | "chainlink" => ("chainlink", "Chainlink"),
            "atom" | "cosmos" => ("cosmos", "Cosmos"),
            "uni" | "uniswap" => ("uniswap", "Uniswap"),
            "xlm" | "stellar" => ("stellar", "Stellar"),
            "shib" => ("shiba-inu", "Shiba Inu"),
            "trx" | "tron" => ("tron", "TRON"),
            "ton" => ("the-open-network", "Toncoin"),
            "pepe" => ("pepe", "Pepe"),
            "near" => ("near", "NEAR"),
            "apt" | "aptos" => ("aptos", "Aptos"),
            "arb" | "arbitrum" => ("arbitrum", "Arbitrum"),
            "op" | "optimism" => ("optimism", "Optimism"),
            "sui" => ("sui", "Sui"),
            _ => return (lower.clone(), capitalize(&lower)),
        };
        (id.to_string(), name.to_string())
    }
}

impl Default for CoinGecko {
    fn default() -> Self {
        Self::new()
    }
}

/// CoinGecko `/simple/price` response shape.
/// Example: `{ "bitcoin": { "usd": 50000, "usd_24h_change": 2.5, "usd_market_cap": 9.5e11 } }`
type SimplePrice = HashMap<String, HashMap<String, f64>>;

#[derive(Debug, Deserialize)]
struct MarketChartResponse {
    prices: Vec<[f64; 2]>,
}

#[async_trait]
impl PriceProvider for CoinGecko {
    fn name(&self) -> &str {
        "CoinGecko"
    }

    fn id(&self) -> &str {
        "coingecko"
    }

    async fn get_prices(&self, symbols: &[String], currency: &str) -> Result<Vec<CoinPrice>> {
        let resolved: Vec<(String, String)> = symbols.iter().map(|s| Self::resolve(s)).collect();
        let ids_param: String = resolved
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let cur = currency.to_lowercase();

        let url = format!(
            "{}/simple/price?ids={}&vs_currencies={}&include_24hr_change=true&include_market_cap=true",
            self.base_url, ids_param, cur
        );
        let cache_key = format!("simple_price:{}:{}:{}", self.base_url, ids_param, cur);

        debug!(url = %url, "fetching prices from CoinGecko");

        let body = if let Some(cached_body) =
            cache::read_json::<String>("coingecko", &cache_key, PRICE_CACHE_TTL_SECS).await
        {
            debug!(ids = %ids_param, currency = %cur, "using cached CoinGecko prices");
            cached_body
        } else {
            let resp = self.client.get(&url).send().await?;
            let status = resp.status();
            let body = resp.text().await?;

            debug!(status = %status, body_len = body.len(), "CoinGecko response");
            trace!(body = %body, "CoinGecko response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "CoinGecko returned {}: {}",
                    status, body
                )));
            }

            cache::write_json("coingecko", &cache_key, &body).await;
            body
        };

        let data: SimplePrice = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("CoinGecko JSON: {}", e)))?;

        let change_key = format!("{}_24h_change", cur);
        let cap_key = format!("{}_market_cap", cur);

        let mut results = Vec::new();
        for (i, (cg_id, display_name)) in resolved.iter().enumerate() {
            if let Some(coin_data) = data.get(cg_id.as_str()) {
                let price = coin_data.get(&cur).copied().unwrap_or(0.0);
                results.push(CoinPrice {
                    symbol: symbols[i].to_uppercase(),
                    name: display_name.clone(),
                    price,
                    change_24h: coin_data.get(&change_key).copied(),
                    market_cap: coin_data.get(&cap_key).copied(),
                    currency: cur.to_uppercase(),
                    provider: self.name().to_string(),
                    timestamp: chrono::Utc::now(),
                });
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
        let cur = currency.to_lowercase();
        let futures = symbols
            .iter()
            .map(|symbol| self.fetch_history_for_symbol(symbol, &cur, days, interval));

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

impl CoinGecko {
    async fn fetch_history_for_symbol(
        &self,
        symbol: &str,
        currency: &str,
        days: u32,
        interval: HistoryInterval,
    ) -> Result<PriceHistory> {
        let (cg_id, display_name) = Self::resolve(symbol);
        let interval_param = match interval {
            HistoryInterval::Auto => String::new(),
            HistoryInterval::Hourly => "&interval=hourly".to_string(),
            HistoryInterval::Daily => "&interval=daily".to_string(),
        };
        let url = format!(
            "{}/coins/{}/market_chart?vs_currency={}&days={}{}",
            self.base_url, cg_id, currency, days, interval_param
        );
        let cache_key = format!(
            "market_chart:{}:{}:{}:{}:{}",
            self.base_url,
            cg_id,
            currency,
            days,
            interval.as_str()
        );
        let cache_ttl = history_cache_ttl(interval, days);

        debug!(
            url = %url,
            symbol = %symbol,
            days,
            interval = interval.as_str(),
            "fetching chart data from CoinGecko"
        );

        let body = if let Some(cached_body) =
            cache::read_json::<String>("coingecko", &cache_key, cache_ttl).await
        {
            debug!(symbol = %symbol, currency = %currency, "using cached CoinGecko chart data");
            cached_body
        } else {
            let resp = self.client.get(&url).send().await?;
            let status = resp.status();
            let body = resp.text().await?;

            debug!(
                status = %status,
                body_len = body.len(),
                symbol = %symbol,
                "CoinGecko chart response"
            );
            trace!(body = %body, symbol = %symbol, "CoinGecko chart response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "CoinGecko returned {} for chart data: {}",
                    status, body
                )));
            }

            cache::write_json("coingecko", &cache_key, &body).await;
            body
        };

        let payload: MarketChartResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("CoinGecko market chart JSON: {}", e)))?;

        let mut points = Vec::new();
        for pair in payload.prices {
            let ts_ms = pair[0] as i64;
            let price = pair[1];

            if !price.is_finite() {
                continue;
            }

            if let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms) {
                points.push(PricePoint { timestamp, price });
            }
        }

        if points.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(PriceHistory {
            symbol: symbol.to_uppercase(),
            name: display_name,
            currency: currency.to_uppercase(),
            provider: self.name().to_string(),
            points,
        })
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let upper: String = c.to_uppercase().collect();
            upper + chars.as_str()
        }
    }
}

fn history_cache_ttl(interval: HistoryInterval, days: u32) -> i64 {
    match interval {
        HistoryInterval::Daily => DAILY_HISTORY_CACHE_TTL_SECS,
        HistoryInterval::Hourly => HOURLY_HISTORY_CACHE_TTL_SECS,
        HistoryInterval::Auto => {
            if days > 30 {
                DAILY_HISTORY_CACHE_TTL_SECS
            } else {
                HOURLY_HISTORY_CACHE_TTL_SECS
            }
        }
    }
}
