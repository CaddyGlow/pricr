use async_trait::async_trait;
use futures::future::join_all;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, trace};

use super::cache;
use super::{CoinPrice, HistoryInterval, PriceHistory, PricePoint, PriceProvider, TickerMatch};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://query2.finance.yahoo.com";
const QUOTE_CACHE_TTL_SECS: i64 = 30;
const SEARCH_CACHE_TTL_SECS: i64 = 10 * 60;
const HOURLY_HISTORY_CACHE_TTL_SECS: i64 = 60 * 60;
const DAILY_HISTORY_CACHE_TTL_SECS: i64 = 12 * 60 * 60;

/// Yahoo Finance provider for stocks/ETFs and ticker discovery.
pub struct YahooFinance {
    client: Client,
    base_url: String,
}

impl YahooFinance {
    /// Create a Yahoo Finance provider using the default production API URL.
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    /// Create a Yahoo Finance provider with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .user_agent("pricr/0.1.0")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.into(),
        }
    }
}

impl Default for YahooFinance {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct YahooChartEnvelope {
    chart: YahooChartResponse,
}

#[derive(Debug, Deserialize)]
struct YahooChartResponse {
    result: Option<Vec<YahooChartResult>>,
    error: Option<YahooApiError>,
}

#[derive(Debug, Deserialize)]
struct YahooChartResult {
    meta: YahooChartMeta,
    timestamp: Option<Vec<i64>>,
    indicators: YahooChartIndicators,
}

#[derive(Debug, Deserialize)]
struct YahooChartMeta {
    currency: Option<String>,
    #[serde(rename = "shortName")]
    short_name: Option<String>,
    #[serde(rename = "longName")]
    long_name: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "chartPreviousClose")]
    chart_previous_close: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct YahooChartIndicators {
    quote: Vec<YahooChartQuote>,
}

#[derive(Debug, Deserialize)]
struct YahooChartQuote {
    close: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
struct YahooApiError {
    description: Option<String>,
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

#[async_trait]
impl PriceProvider for YahooFinance {
    fn name(&self) -> &str {
        "Yahoo Finance"
    }

    fn id(&self) -> &str {
        "yahoo"
    }

    async fn get_prices(&self, symbols: &[String], currency: &str) -> Result<Vec<CoinPrice>> {
        let requested_currency = currency.to_uppercase();
        let futures = symbols
            .iter()
            .map(|symbol| self.fetch_latest_quote_for_symbol(symbol, &requested_currency));
        let mut results = Vec::new();
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
        let end = chrono::Utc::now();
        let start = end - chrono::Duration::days(days as i64);
        self.get_price_history_window(symbols, currency, Some(start), end, interval)
            .await
    }

    async fn get_price_history_window(
        &self,
        symbols: &[String],
        currency: &str,
        start: Option<chrono::DateTime<chrono::Utc>>,
        end: chrono::DateTime<chrono::Utc>,
        interval: HistoryInterval,
    ) -> Result<Vec<PriceHistory>> {
        let requested_currency = currency.to_uppercase();
        let futures = symbols.iter().map(|symbol| {
            self.fetch_history_for_symbol(symbol, &requested_currency, start, end, interval)
        });

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

        let endpoint = format!("{}/v1/finance/search", self.base_url);
        let limit_string = limit.to_string();
        let cache_key = format!("search:{}:{}:{}", self.base_url, trimmed, limit_string);

        let body = if let Some(cached_body) =
            cache::read_json::<String>("yahoo", &cache_key, SEARCH_CACHE_TTL_SECS).await
        {
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
            if !status.is_success() {
                return Err(Error::Api(format!(
                    "Yahoo Finance search returned {}: {}",
                    status, body
                )));
            }

            cache::write_json("yahoo", &cache_key, &body).await;
            body
        };

        let payload: YahooSearchResponse = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("Yahoo search JSON: {}", e)))?;

        let matches = payload
            .quotes
            .into_iter()
            .filter_map(|quote| {
                let symbol = quote.symbol.trim().to_uppercase();
                if symbol.is_empty() {
                    return None;
                }

                Some(TickerMatch {
                    symbol: symbol.clone(),
                    name: quote.longname.or(quote.shortname).unwrap_or(symbol),
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

impl YahooFinance {
    async fn fetch_latest_quote_for_symbol(
        &self,
        symbol: &str,
        requested_currency: &str,
    ) -> Result<Option<CoinPrice>> {
        let symbol_upper = symbol.to_uppercase();
        let endpoint = format!("{}/v8/finance/chart/{}", self.base_url, symbol_upper);
        let cache_key = format!("latest_chart:{}:{}", self.base_url, symbol_upper);

        debug!(symbol = %symbol_upper, "fetching latest quote from Yahoo Finance chart endpoint");

        let body = if let Some(cached_body) =
            cache::read_json::<String>("yahoo", &cache_key, QUOTE_CACHE_TTL_SECS).await
        {
            cached_body
        } else {
            let resp = self
                .client
                .get(&endpoint)
                .query(&[("range", "5d"), ("interval", "1d")])
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                return Err(Error::Api(format!(
                    "Yahoo Finance returned {} for quote data: {}",
                    status, body
                )));
            }

            cache::write_json("yahoo", &cache_key, &body).await;
            body
        };

        let payload: YahooChartEnvelope = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("Yahoo quote chart JSON: {}", e)))?;

        if let Some(api_error) = payload.chart.error
            && let Some(description) = api_error.description
            && !description.is_empty()
        {
            return Err(Error::Api(format!("Yahoo Finance: {}", description)));
        }

        let chart = payload
            .chart
            .result
            .and_then(|mut values| values.drain(..).next());

        let Some(chart) = chart else {
            return Ok(None);
        };

        let mut closes = chart
            .indicators
            .quote
            .into_iter()
            .next()
            .and_then(|quote| quote.close)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .filter(|value| value.is_finite())
            .collect::<Vec<_>>();
        if closes.is_empty() {
            return Ok(None);
        }

        let price = chart
            .meta
            .regular_market_price
            .filter(|value| value.is_finite())
            .unwrap_or_else(|| *closes.last().expect("non-empty closes"));
        if !price.is_finite() {
            return Ok(None);
        }

        let change_24h = chart
            .meta
            .chart_previous_close
            .and_then(|prev| percent_change(prev, price))
            .or_else(|| {
                if closes.len() < 2 {
                    None
                } else {
                    let prev = closes.remove(closes.len() - 2);
                    percent_change(prev, price)
                }
            })
            .filter(|value| value.is_finite());

        let quote_currency = chart
            .meta
            .currency
            .unwrap_or_else(|| requested_currency.to_string())
            .to_uppercase();
        let name = chart
            .meta
            .long_name
            .or(chart.meta.short_name)
            .unwrap_or_else(|| symbol_upper.clone());

        Ok(Some(CoinPrice {
            symbol: symbol_upper,
            name,
            price,
            change_24h,
            market_cap: None,
            currency: quote_currency,
            provider: self.name().to_string(),
            timestamp: chrono::Utc::now(),
        }))
    }

    async fn fetch_history_for_symbol(
        &self,
        symbol: &str,
        requested_currency: &str,
        start: Option<chrono::DateTime<chrono::Utc>>,
        end: chrono::DateTime<chrono::Utc>,
        interval: HistoryInterval,
    ) -> Result<PriceHistory> {
        let symbol_upper = symbol.to_uppercase();
        let endpoint = format!("{}/v8/finance/chart/{}", self.base_url, symbol_upper);
        let interval_param = chart_interval(interval, start, end);
        let period1 = start.map(|dt| dt.timestamp()).unwrap_or(0);
        let period2 = (end + chrono::Duration::seconds(1))
            .timestamp()
            .max(period1 + 1);
        let cache_key = format!(
            "chart:{}:{}:{}:{}:{}",
            self.base_url, symbol_upper, period1, period2, interval_param
        );
        let cache_ttl = if interval_param == "1h" {
            HOURLY_HISTORY_CACHE_TTL_SECS
        } else {
            DAILY_HISTORY_CACHE_TTL_SECS
        };

        debug!(
            symbol = %symbol_upper,
            period1,
            period2,
            interval = interval_param,
            "fetching chart data from Yahoo Finance"
        );

        let body = if let Some(cached_body) =
            cache::read_json::<String>("yahoo", &cache_key, cache_ttl).await
        {
            debug!(symbol = %symbol_upper, "using cached Yahoo chart response");
            cached_body
        } else {
            let resp = self
                .client
                .get(&endpoint)
                .query(&[
                    ("period1", period1.to_string()),
                    ("period2", period2.to_string()),
                    ("interval", interval_param.to_string()),
                ])
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;

            debug!(
                status = %status,
                symbol = %symbol_upper,
                body_len = body.len(),
                "Yahoo chart response"
            );
            trace!(body = %body, symbol = %symbol_upper, "Yahoo chart response body");

            if !status.is_success() {
                return Err(Error::Api(format!(
                    "Yahoo Finance returned {} for chart data: {}",
                    status, body
                )));
            }

            cache::write_json("yahoo", &cache_key, &body).await;
            body
        };

        let payload: YahooChartEnvelope = serde_json::from_str(&body)
            .map_err(|e| Error::Parse(format!("Yahoo chart JSON: {}", e)))?;

        if let Some(api_error) = payload.chart.error
            && let Some(description) = api_error.description
            && !description.is_empty()
        {
            return Err(Error::Api(format!("Yahoo Finance: {}", description)));
        }

        let chart = payload
            .chart
            .result
            .and_then(|mut values| values.drain(..).next())
            .ok_or(Error::NoResults)?;

        let timestamps = chart.timestamp.unwrap_or_default();
        let closes = chart
            .indicators
            .quote
            .into_iter()
            .next()
            .and_then(|quote| quote.close)
            .unwrap_or_default();

        let mut points = Vec::new();
        for (ts, close) in timestamps.into_iter().zip(closes.into_iter()) {
            let Some(price) = close else {
                continue;
            };
            if !price.is_finite() {
                continue;
            }

            let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0) else {
                continue;
            };

            if timestamp > end {
                continue;
            }
            if let Some(start_ts) = start
                && timestamp < start_ts
            {
                continue;
            }

            points.push(PricePoint { timestamp, price });
        }

        points.sort_by_key(|point| point.timestamp);
        if points.is_empty() {
            return Err(Error::NoResults);
        }

        let currency = chart
            .meta
            .currency
            .unwrap_or_else(|| requested_currency.to_string())
            .to_uppercase();
        let name = chart
            .meta
            .long_name
            .or(chart.meta.short_name)
            .unwrap_or_else(|| symbol_upper.clone());

        Ok(PriceHistory {
            symbol: symbol_upper,
            name,
            currency,
            provider: self.name().to_string(),
            points,
        })
    }
}

fn percent_change(previous: f64, current: f64) -> Option<f64> {
    if !previous.is_finite() || previous.abs() <= f64::EPSILON {
        return None;
    }

    Some(((current - previous) / previous) * 100.0)
}

fn chart_interval(
    interval: HistoryInterval,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: chrono::DateTime<chrono::Utc>,
) -> &'static str {
    match interval {
        HistoryInterval::Daily => "1d",
        HistoryInterval::Hourly => "1h",
        HistoryInterval::Auto => {
            let days = start.map(|s| (end - s).num_days().max(1)).unwrap_or(366);
            if days <= 5 { "1h" } else { "1d" }
        }
    }
}
