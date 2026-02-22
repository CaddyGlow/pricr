use crate::calc::Conversion;
use crate::error::Result;
use crate::provider::{CoinPrice, PriceHistory};

/// Print prices as formatted JSON to stdout.
pub fn print_json(prices: &[CoinPrice]) -> Result<()> {
    let output = serde_json::to_string_pretty(prices)
        .map_err(|e| crate::error::Error::Parse(format!("JSON serialize: {}", e)))?;
    println!("{}", output);
    Ok(())
}

/// Print fiat-to-crypto conversions as formatted JSON to stdout.
pub fn print_conversions_json(conversions: &[Conversion]) -> Result<()> {
    let output = serde_json::to_string_pretty(conversions)
        .map_err(|e| crate::error::Error::Parse(format!("JSON serialize: {}", e)))?;
    println!("{}", output);
    Ok(())
}

/// Print historical prices as formatted JSON to stdout.
pub fn print_history_json(histories: &[PriceHistory]) -> Result<()> {
    let output = serde_json::to_string_pretty(histories)
        .map_err(|e| crate::error::Error::Parse(format!("JSON serialize: {}", e)))?;
    println!("{}", output);
    Ok(())
}
