use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::debug;

use super::cache;
use super::{PriceHistory, PricePoint};
use crate::calc;
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.frankfurter.dev/v1";
const LATEST_RATES_CACHE_TTL_SECS: i64 = 10 * 60;
const HISTORY_CACHE_TTL_SECS: i64 = 12 * 60 * 60;

/// Frankfurter forex provider backed by ECB reference rates.
pub struct Frankfurter {
    client: reqwest::Client,
    base_url: String,
}

impl Frankfurter {
    /// Create a Frankfurter provider using the default production API URL.
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    /// Create a Frankfurter provider with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Fetch forex rates from Frankfurter.
    ///
    /// Returns a map of target currency code to rate where each value is
    /// expressed as "1 source = rate target".
    pub async fn get_rates(&self, from: &str, to: &[String]) -> Result<HashMap<String, f64>> {
        let from_upper = from.to_uppercase();
        let to_param = to.join(",").to_uppercase();
        let url = format!(
            "{}/latest?from={}&to={}",
            self.base_url, from_upper, to_param,
        );
        let cache_key = format!("latest:{}:{}:{}", self.base_url, from_upper, to_param);

        debug!(url = %url, "fetching forex rates from Frankfurter");

        let body: FrankfurterResponse = if let Some(cached) =
            cache::read_json("frankfurter", &cache_key, LATEST_RATES_CACHE_TTL_SECS).await
        {
            debug!(from = %from_upper, to = %to_param, "using cached Frankfurter rates");
            cached
        } else {
            let resp = self.client.get(&url).send().await?.error_for_status()?;
            let fetched: FrankfurterResponse = resp.json().await?;
            cache::write_json("frankfurter", &cache_key, &fetched).await;
            fetched
        };

        debug!(rates = ?body.rates, "received forex rates");

        if body.rates.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(body.rates)
    }

    /// Fetch historical forex rates from Frankfurter.
    ///
    /// Returns one history series per target code where each point is
    /// expressed as "1 source = rate target".
    pub async fn get_history(
        &self,
        from: &str,
        to: &[String],
        days: u32,
    ) -> Result<Vec<PriceHistory>> {
        let from_upper = from.to_uppercase();
        let to_upper: Vec<String> = to.iter().map(|s| s.to_uppercase()).collect();
        let to_param = to_upper.join(",");

        let end = chrono::Utc::now().date_naive();
        let start = end - chrono::Duration::days(days as i64);
        let url = format!(
            "{}/{}..{}?from={}&to={}",
            self.base_url,
            start.format("%Y-%m-%d"),
            end.format("%Y-%m-%d"),
            from_upper,
            to_param
        );
        let cache_key = format!(
            "history:{}:{}:{}:{}",
            self.base_url, from_upper, to_param, days
        );

        debug!(url = %url, "fetching historical forex rates from Frankfurter");

        let body: FrankfurterHistoryResponse = if let Some(cached) =
            cache::read_json("frankfurter", &cache_key, HISTORY_CACHE_TTL_SECS).await
        {
            debug!(from = %from_upper, to = %to_param, days, "using cached Frankfurter history");
            cached
        } else {
            let resp = self.client.get(&url).send().await?.error_for_status()?;
            let fetched: FrankfurterHistoryResponse = resp.json().await?;
            cache::write_json("frankfurter", &cache_key, &fetched).await;
            fetched
        };

        if body.rates.is_empty() {
            return Err(Error::NoResults);
        }

        let mut histories = Vec::new();
        for target in to_upper {
            let mut points = Vec::new();

            for (date, rate_map) in &body.rates {
                let Some(rate) = rate_map.get(&target).copied() else {
                    continue;
                };

                let Ok(parsed_date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                    continue;
                };

                let Some(naive_dt) = parsed_date.and_hms_opt(0, 0, 0) else {
                    continue;
                };

                points.push(PricePoint {
                    timestamp: naive_dt.and_utc(),
                    price: rate,
                });
            }

            points.sort_by_key(|p| p.timestamp);

            if points.is_empty() {
                continue;
            }

            histories.push(PriceHistory {
                symbol: target.clone(),
                name: calc::fiat_name(&target).to_string(),
                currency: from_upper.clone(),
                provider: "Frankfurter/ECB".to_string(),
                points,
            });
        }

        if histories.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(histories)
    }
}

impl Default for Frankfurter {
    fn default() -> Self {
        Self::new()
    }
}

/// Response shape from `GET /latest` on the Frankfurter API.
#[derive(Debug, Serialize, Deserialize)]
struct FrankfurterResponse {
    rates: HashMap<String, f64>,
}

/// Response shape from date-range history endpoints on the Frankfurter API.
#[derive(Debug, Serialize, Deserialize)]
struct FrankfurterHistoryResponse {
    rates: HashMap<String, HashMap<String, f64>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frankfurter_response_parsing() {
        let json = r#"{"amount":1.0,"base":"USD","date":"2026-02-20","rates":{"EUR":0.84983,"GBP":0.74174}}"#;
        let resp: FrankfurterResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.rates.len(), 2);
        assert!((resp.rates["EUR"] - 0.84983).abs() < 1e-6);
        assert!((resp.rates["GBP"] - 0.74174).abs() < 1e-6);
    }

    #[test]
    fn frankfurter_history_response_parsing() {
        let json = r#"{
          "amount": 1.0,
          "base": "USD",
          "start_date": "2026-02-15",
          "end_date": "2026-02-22",
          "rates": {
            "2026-02-20": {"EUR": 0.92, "GBP": 0.79},
            "2026-02-21": {"EUR": 0.93, "GBP": 0.80}
          }
        }"#;
        let resp: FrankfurterHistoryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.rates.len(), 2);
        assert!((resp.rates["2026-02-20"]["EUR"] - 0.92).abs() < 1e-6);
        assert!((resp.rates["2026-02-21"]["GBP"] - 0.80).abs() < 1e-6);
    }
}
