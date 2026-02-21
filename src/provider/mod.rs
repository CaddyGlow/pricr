pub mod coingecko;
pub mod coinmarketcap;
pub mod frankfurter;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A single coin's price data returned by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinPrice {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub change_24h: Option<f64>,
    pub market_cap: Option<f64>,
    pub currency: String,
    pub provider: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Trait implemented by all price data providers.
#[async_trait]
pub trait PriceProvider: Send + Sync {
    /// Human-readable provider name.
    fn name(&self) -> &str;

    /// Short identifier used in CLI flags.
    fn id(&self) -> &str;

    /// Fetch prices for the given coin symbols in the specified fiat currency.
    async fn get_prices(&self, symbols: &[String], currency: &str) -> Result<Vec<CoinPrice>>;
}

/// Build the list of available providers based on configuration.
pub fn available_providers(api_key: Option<String>) -> Vec<Box<dyn PriceProvider>> {
    let mut providers: Vec<Box<dyn PriceProvider>> = vec![Box::new(coingecko::CoinGecko::new())];

    if let Some(key) = api_key {
        providers.push(Box::new(coinmarketcap::CoinMarketCap::new(key)));
    } else if let Ok(key) = std::env::var("COINMARKETCAP_API_KEY") {
        providers.push(Box::new(coinmarketcap::CoinMarketCap::new(key)));
    }

    providers
}

/// Look up a provider index by its short id.
pub fn get_provider(providers: &[Box<dyn PriceProvider>], id: &str) -> Option<usize> {
    providers.iter().position(|p| p.id() == id)
}
