use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, trace};

use super::cache;
use super::{CoinPrice, HistoryInterval, PriceHistory, PricePoint, PriceProvider, TickerMatch};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://stooq.com";
const SEARCH_BASE_URL: &str = "https://query2.finance.yahoo.com";
const HISTORY_CACHE_TTL_SECS: i64 = 12 * 60 * 60;
const PRICE_CACHE_TTL_SECS: i64 = 30;
const SEARCH_CACHE_TTL_SECS: i64 = 10 * 60;

/// Stooq price provider for stock and ETF symbols.
pub struct Stooq {
    client: Client,
    base_url: String,
    search_base_url: String,
}

impl Stooq {
    /// Create a Stooq provider using the default production API URL.
    pub fn new() -> Self {
        Self::with_base_urls(BASE_URL, SEARCH_BASE_URL)
    }

    /// Create a Stooq provider with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self::with_base_urls(base_url, SEARCH_BASE_URL)
    }

    /// Create a Stooq provider with custom quote/history and search base URLs.
    pub fn with_base_urls(base_url: impl Into<String>, search_base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .user_agent("pricr/0.1.0")
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            base_url: base_url.into(),
            search_base_url: search_base_url.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct YahooSearchResponse {
    quotes: Vec<YahooSearchQuote>,
}

#[derive(Debug, Deserialize)]
struct YahooSearchQuote {
    symbol: String,
    shortname: Option<String>,
    longname: Option<String>,
    #[serde(rename = "exchDisp")]
    exch_disp: Option<String>,
    #[serde(rename = "typeDisp")]
    type_disp: Option<String>,
}

impl Default for Stooq {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PriceProvider for Stooq {
    fn name(&self) -> &str {
        "Stooq"
    }

    fn id(&self) -> &str {
        "stooq"
    }

    async fn get_prices(&self, symbols: &[String], currency: &str) -> Result<Vec<CoinPrice>> {
        let requested_currency = currency.to_uppercase();
        let requested: Vec<(String, String)> = symbols
            .iter()
            .map(|symbol| (symbol.to_uppercase(), normalize_symbol(symbol)))
            .collect();

        let mut results = Vec::new();
        let futures = requested.iter().map(|(display_symbol, normalized)| {
            self.fetch_quote_for_symbol(display_symbol, normalized, &requested_currency)
        });

        for result in join_all(futures).await {
            if let Some(price) = result? {
                results.push(price);
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
        if matches!(interval, HistoryInterval::Hourly) {
            return Err(Error::Config(
                "provider 'stooq' supports daily history only".into(),
            ));
        }

        let requested_currency = currency.to_uppercase();
        let futures = symbols
            .iter()
            .map(|symbol| self.fetch_history_for_symbol(symbol, &requested_currency, days));

        let mut histories = Vec::new();
        for result in join_all(futures).await {
            histories.push(result?);
        }

        if histories.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(histories)
    }

    async fn search_tickers(&self, query: &str, limit: usize) -> Result<Vec<TickerMatch>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(Error::Config("ticker search query cannot be empty".into()));
        }

        let endpoint = format!("{}/v1/finance/search", self.search_base_url);
        let query_lower = trimmed.to_lowercase();
        let limit_string = limit.to_string();
        let cache_key = format!(
            "search:{}:{}:{}",
            self.search_base_url, query_lower, limit_string
        );

        debug!(query = %trimmed, limit, "searching tickers via Yahoo Finance search API");

        let body = if let Some(cached_body) =
            cache::read_json::<String>("stooq", &cache_key, SEARCH_CACHE_TTL_SECS).await
        {
            debug!(query = %trimmed, limit, "using cached ticker search response");
            cached_body
        } else {
            let resp = self
                .client
                .get(&endpoint)
                .query(&[
                    ("q", trimmed),
                    ("quotesCount", limit_string.as_str()),
                    ("newsCount", "0"),
                ])
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(status = %status, body_len = body.len(), "ticker search response");
            trace!(body = %body, query = %trimmed, "ticker search response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "ticker search returned {}: {}",
                    status, body
                )));
            }

            cache::write_json("stooq", &cache_key, &body).await;
            body
        };

        let raw: YahooSearchResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("ticker search JSON: {}", e)))?;

        let matches = raw
            .quotes
            .into_iter()
            .filter_map(|quote| {
                let symbol = quote.symbol.trim().to_uppercase();
                if symbol.is_empty() {
                    return None;
                }

                let name = quote
                    .longname
                    .or(quote.shortname)
                    .unwrap_or_else(|| symbol.clone());

                Some(TickerMatch {
                    symbol,
                    name,
                    exchange: quote.exch_disp.unwrap_or_else(|| "Unknown".to_string()),
                    asset_type: quote.type_disp.unwrap_or_else(|| "Unknown".to_string()),
                    provider: self.name().to_string(),
                })
            })
            .take(limit)
            .collect::<Vec<_>>();

        if matches.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(matches)
    }
}

impl Stooq {
    async fn fetch_quote_for_symbol(
        &self,
        display_symbol: &str,
        normalized: &str,
        requested_currency: &str,
    ) -> Result<Option<CoinPrice>> {
        let endpoint = format!("{}/q/l/", self.base_url);
        let cache_key = format!("quote:{}:{}", self.base_url, normalized);

        debug!(symbol = %normalized, "fetching quote from Stooq");

        let body = if let Some(cached_body) =
            cache::read_json::<String>("stooq", &cache_key, PRICE_CACHE_TTL_SECS).await
        {
            debug!(symbol = %normalized, "using cached Stooq quote response");
            cached_body
        } else {
            let resp = self
                .client
                .get(&endpoint)
                .query(&[("s", normalized), ("i", "d")])
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(
                status = %status,
                symbol = %normalized,
                body_len = body.len(),
                "Stooq quote response"
            );
            trace!(body = %body, symbol = %normalized, "Stooq quote response body");

            if !status.is_success() {
                return Err(Error::Api(format!("Stooq returned {}: {}", status, body)));
            }

            cache::write_json("stooq", &cache_key, &body).await;
            body
        };

        let key = normalized.to_uppercase();
        let row = body
            .lines()
            .filter_map(parse_quote_row)
            .find(|row| row.symbol == key);

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(CoinPrice {
            symbol: display_symbol.to_string(),
            name: display_symbol.to_string(),
            price: row.close,
            change_24h: row
                .open
                .and_then(|open| percent_change(open, row.close))
                .filter(|v| v.is_finite()),
            market_cap: None,
            currency: currency_for_symbol(normalized, requested_currency),
            provider: self.name().to_string(),
            timestamp: chrono::Utc::now(),
        }))
    }

    async fn fetch_history_for_symbol(
        &self,
        symbol: &str,
        requested_currency: &str,
        days: u32,
    ) -> Result<PriceHistory> {
        let display_symbol = symbol.to_uppercase();
        let normalized = normalize_symbol(symbol);
        let endpoint = format!("{}/q/d/l/", self.base_url);
        let cache_key = format!("history:{}:{}:{}", self.base_url, normalized, days);

        debug!(
            symbol = %normalized,
            days,
            "fetching chart data from Stooq"
        );

        let body = if let Some(cached_body) =
            cache::read_json::<String>("stooq", &cache_key, HISTORY_CACHE_TTL_SECS).await
        {
            debug!(symbol = %normalized, "using cached Stooq history response");
            cached_body
        } else {
            let resp = self
                .client
                .get(&endpoint)
                .query(&[("s", normalized.as_str()), ("i", "d")])
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(
                status = %status,
                symbol = %normalized,
                body_len = body.len(),
                "Stooq history response"
            );
            trace!(body = %body, symbol = %normalized, "Stooq history response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "Stooq returned {} for chart data: {}",
                    status, body
                )));
            }

            cache::write_json("stooq", &cache_key, &body).await;
            body
        };

        let mut points = Vec::new();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("Date,") {
                continue;
            }

            let cols: Vec<&str> = trimmed.split(',').collect();
            if cols.len() < 5 {
                continue;
            }

            let Ok(date) = chrono::NaiveDate::parse_from_str(cols[0].trim(), "%Y-%m-%d") else {
                continue;
            };
            let Some(close) = parse_decimal(cols[4]) else {
                continue;
            };

            let Some(naive_dt) = date.and_hms_opt(0, 0, 0) else {
                continue;
            };

            points.push(PricePoint {
                timestamp: naive_dt.and_utc(),
                price: close,
            });
        }

        points.sort_by_key(|p| p.timestamp);
        trim_points_to_days(&mut points, days);

        if points.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(PriceHistory {
            symbol: display_symbol.clone(),
            name: display_symbol,
            currency: currency_for_symbol(&normalized, requested_currency),
            provider: self.name().to_string(),
            points,
        })
    }
}

struct QuoteRow {
    symbol: String,
    open: Option<f64>,
    close: f64,
}

fn parse_quote_row(line: &str) -> Option<QuoteRow> {
    let cols: Vec<&str> = line.trim().split(',').collect();
    if cols.len() < 7 {
        return None;
    }

    if cols.get(1).map(|v| v.trim()) == Some("N/D") {
        return None;
    }

    let symbol = cols.first()?.trim().to_uppercase();
    let close = parse_decimal(cols[6])?;
    let open = parse_decimal(cols[3]);

    Some(QuoteRow {
        symbol,
        open,
        close,
    })
}

fn parse_decimal(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    if parsed.is_finite() {
        Some(parsed)
    } else {
        None
    }
}

fn percent_change(open: f64, close: f64) -> Option<f64> {
    if open.abs() <= f64::EPSILON {
        return None;
    }

    Some(((close - open) / open) * 100.0)
}

fn normalize_symbol(symbol: &str) -> String {
    let trimmed = symbol.trim().to_lowercase();
    if trimmed.contains('.') {
        trimmed
    } else {
        format!("{}.us", trimmed)
    }
}

fn currency_for_symbol(normalized_symbol: &str, fallback: &str) -> String {
    if normalized_symbol.ends_with(".us") {
        "USD".to_string()
    } else {
        fallback.to_string()
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
