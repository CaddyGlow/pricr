use serde::{Deserialize, Serialize};

/// Recognized fiat currency codes. Prevents false positives on tokens like `1inch` or `3btc`.
const KNOWN_FIAT: &[&str] = &[
    "USD", "EUR", "GBP", "JPY", "CNY", "CAD", "AUD", "CHF", "KRW", "INR", "BRL", "RUB", "TRY",
    "ZAR", "MXN", "SGD", "HKD", "NOK", "SEK", "DKK", "NZD", "PLN", "THB", "TWD", "CZK", "HUF",
    "ILS", "PHP", "MYR", "ARS", "CLP", "COP", "IDR", "SAR", "AED", "NGN", "VND", "PKR", "BDT",
    "EGP",
];

/// A parsed fiat amount from user input (e.g. `3.5EUR`).
#[derive(Debug, Clone)]
pub struct FiatAmount {
    pub amount: f64,
    pub currency: String,
}

/// Result of a fiat-to-crypto conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversion {
    pub from_amount: f64,
    pub from_currency: String,
    pub to_symbol: String,
    pub to_name: String,
    pub to_amount: f64,
    pub rate: f64,
    pub provider: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Try to parse a string like `3.5EUR` or `100usd` into a `FiatAmount`.
///
/// Returns `None` when the input does not match `<number><fiat_code>`, letting
/// the caller fall through to normal price-lookup mode.
pub fn parse_fiat_amount(s: &str) -> Option<FiatAmount> {
    // Find where the alphabetic suffix starts.
    let alpha_start = s.find(|c: char| c.is_ascii_alphabetic())?;
    if alpha_start == 0 {
        return None;
    }

    let (num_part, code_part) = s.split_at(alpha_start);
    let code_upper = code_part.to_uppercase();

    if !KNOWN_FIAT.contains(&code_upper.as_str()) {
        return None;
    }

    let amount: f64 = num_part.parse().ok()?;
    if amount <= 0.0 || !amount.is_finite() {
        return None;
    }

    Some(FiatAmount {
        amount,
        currency: code_upper,
    })
}

/// Returns `true` when `s` (case-insensitive) is a recognized fiat currency code.
pub fn is_known_fiat(s: &str) -> bool {
    KNOWN_FIAT.contains(&s.to_uppercase().as_str())
}

/// Human-readable name for a fiat currency code. Falls back to the code itself.
pub fn fiat_name(code: &str) -> &str {
    match code.to_uppercase().as_str() {
        "USD" => "US Dollar",
        "EUR" => "Euro",
        "GBP" => "British Pound",
        "JPY" => "Japanese Yen",
        "CNY" => "Chinese Yuan",
        "CAD" => "Canadian Dollar",
        "AUD" => "Australian Dollar",
        "CHF" => "Swiss Franc",
        "KRW" => "South Korean Won",
        "INR" => "Indian Rupee",
        "BRL" => "Brazilian Real",
        "RUB" => "Russian Ruble",
        "TRY" => "Turkish Lira",
        "ZAR" => "South African Rand",
        "MXN" => "Mexican Peso",
        "SGD" => "Singapore Dollar",
        "HKD" => "Hong Kong Dollar",
        "NOK" => "Norwegian Krone",
        "SEK" => "Swedish Krona",
        "DKK" => "Danish Krone",
        "NZD" => "New Zealand Dollar",
        "PLN" => "Polish Zloty",
        "THB" => "Thai Baht",
        "TWD" => "New Taiwan Dollar",
        "CZK" => "Czech Koruna",
        "HUF" => "Hungarian Forint",
        "ILS" => "Israeli Shekel",
        "PHP" => "Philippine Peso",
        "MYR" => "Malaysian Ringgit",
        "ARS" => "Argentine Peso",
        "CLP" => "Chilean Peso",
        "COP" => "Colombian Peso",
        "IDR" => "Indonesian Rupiah",
        "SAR" => "Saudi Riyal",
        "AED" => "UAE Dirham",
        "NGN" => "Nigerian Naira",
        "VND" => "Vietnamese Dong",
        "PKR" => "Pakistani Rupee",
        "BDT" => "Bangladeshi Taka",
        "EGP" => "Egyptian Pound",
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_cases() {
        let fa = parse_fiat_amount("3.5EUR").unwrap();
        assert!((fa.amount - 3.5).abs() < f64::EPSILON);
        assert_eq!(fa.currency, "EUR");

        let fa = parse_fiat_amount("100usd").unwrap();
        assert!((fa.amount - 100.0).abs() < f64::EPSILON);
        assert_eq!(fa.currency, "USD");
    }

    #[test]
    fn parse_lowercase_currency() {
        let fa = parse_fiat_amount("42gbp").unwrap();
        assert_eq!(fa.currency, "GBP");
    }

    #[test]
    fn rejects_crypto_symbols() {
        assert!(parse_fiat_amount("1inch").is_none());
        assert!(parse_fiat_amount("3btc").is_none());
    }

    #[test]
    fn rejects_plain_words() {
        assert!(parse_fiat_amount("btc").is_none());
        assert!(parse_fiat_amount("hello").is_none());
    }

    #[test]
    fn rejects_negative_and_zero() {
        assert!(parse_fiat_amount("-5USD").is_none());
        assert!(parse_fiat_amount("0USD").is_none());
    }

    #[test]
    fn rejects_no_number() {
        assert!(parse_fiat_amount("EUR").is_none());
    }

    #[test]
    fn is_known_fiat_works() {
        assert!(is_known_fiat("USD"));
        assert!(is_known_fiat("eur"));
        assert!(is_known_fiat("Gbp"));
        assert!(!is_known_fiat("BTC"));
        assert!(!is_known_fiat("ETH"));
        assert!(!is_known_fiat(""));
    }

    #[test]
    fn fiat_name_known_codes() {
        assert_eq!(fiat_name("USD"), "US Dollar");
        assert_eq!(fiat_name("eur"), "Euro");
        assert_eq!(fiat_name("GBP"), "British Pound");
    }

    #[test]
    fn fiat_name_unknown_returns_code() {
        assert_eq!(fiat_name("XYZ"), "XYZ");
    }
}
