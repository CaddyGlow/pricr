use std::collections::HashMap;

use serde::Deserialize;
use tracing::debug;

use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.frankfurter.dev/v1";

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
        let to_param = to.join(",");
        let url = format!(
            "{}/latest?from={}&to={}",
            self.base_url,
            from.to_uppercase(),
            to_param.to_uppercase(),
        );

        debug!(url = %url, "fetching forex rates from Frankfurter");

        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let body: FrankfurterResponse = resp.json().await?;

        debug!(rates = ?body.rates, "received forex rates");

        if body.rates.is_empty() {
            return Err(Error::NoResults);
        }

        Ok(body.rates)
    }
}

impl Default for Frankfurter {
    fn default() -> Self {
        Self::new()
    }
}

/// Response shape from `GET /latest` on the Frankfurter API.
#[derive(Debug, Deserialize)]
struct FrankfurterResponse {
    rates: HashMap<String, f64>,
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
}
