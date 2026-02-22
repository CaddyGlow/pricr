use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

use crate::calc::{self, Conversion};
use crate::output::chart;
use crate::provider::{CoinPrice, HistoryInterval, PriceHistory};

#[derive(Tabled)]
struct PriceRow {
    #[tabled(rename = "Symbol")]
    symbol: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Price")]
    price: String,
    #[tabled(rename = "24h Change")]
    change_24h: String,
    #[tabled(rename = "Market Cap")]
    market_cap: String,
    #[tabled(rename = "Provider")]
    provider: String,
}

/// Print prices as a styled table to stdout.
pub fn print_table(prices: &[CoinPrice]) {
    let rows: Vec<PriceRow> = prices
        .iter()
        .map(|p| {
            let change_str = match p.change_24h {
                Some(c) if c >= 0.0 => format!("+{:.2}%", c).green().to_string(),
                Some(c) => format!("{:.2}%", c).red().to_string(),
                None => "-".dimmed().to_string(),
            };

            PriceRow {
                symbol: p.symbol.clone().bold().to_string(),
                name: p.name.clone(),
                price: format_price(p.price, &p.currency),
                change_24h: change_str,
                market_cap: match p.market_cap {
                    Some(cap) => format_market_cap(cap, &p.currency),
                    None => "-".to_string(),
                },
                provider: p.provider.clone().dimmed().to_string(),
            }
        })
        .collect();

    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);
}

#[derive(Tabled)]
struct ConversionRow {
    #[tabled(rename = "Amount")]
    amount: String,
    #[tabled(rename = "")]
    arrow: String,
    #[tabled(rename = "Result")]
    result: String,
    #[tabled(rename = "Rate")]
    rate: String,
    #[tabled(rename = "Provider")]
    provider: String,
}

/// Print fiat-to-crypto conversions as a styled table to stdout.
pub fn print_conversions_table(conversions: &[Conversion]) {
    let rows: Vec<ConversionRow> = conversions
        .iter()
        .map(|c| {
            let from_sym = currency_symbol(&c.from_currency);
            let is_fiat = calc::is_known_fiat(&c.to_symbol);

            let result = if is_fiat {
                let to_sym = currency_symbol(&c.to_symbol);
                format!("{}{}", to_sym, format_with_commas(c.to_amount, 2))
            } else {
                format_crypto_amount(c.to_amount, &c.to_symbol)
            };

            ConversionRow {
                amount: format!("{}{}", from_sym, format_with_commas(c.from_amount, 2)),
                arrow: "->".to_string(),
                result,
                rate: format!(
                    "1 {} = {}{}",
                    c.to_symbol.to_uppercase(),
                    from_sym,
                    format_with_commas(c.rate, 2)
                ),
                provider: c.provider.clone().dimmed().to_string(),
            }
        })
        .collect();

    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);
}

/// Print ASCII charts for historical price series.
pub fn print_history_charts(histories: &[PriceHistory], days: u32, interval: HistoryInterval) {
    for history in histories {
        if history.points.is_empty() {
            continue;
        }

        let prices: Vec<f64> = history.points.iter().map(|p| p.price).collect();
        let start = prices[0];
        let end = prices[prices.len() - 1];
        let low = prices.iter().copied().fold(f64::INFINITY, f64::min);
        let high = prices.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let change_pct = if start.abs() > f64::EPSILON {
            ((end - start) / start) * 100.0
        } else {
            0.0
        };

        let trend = if change_pct >= 0.0 {
            format!("+{change_pct:.2}%").green().to_string()
        } else {
            format!("{change_pct:.2}%").red().to_string()
        };

        println!(
            "{} ({})  [{} {}d]",
            history.symbol.bold(),
            history.name,
            history.currency,
            days
        );
        println!("Interval: {}", interval.as_str());
        println!(
            "Start: {}  End: {}  Change: {}",
            format_price(start, &history.currency),
            format_price(end, &history.currency),
            trend
        );
        println!(
            "Low:   {}  High: {}",
            format_price(low, &history.currency),
            format_price(high, &history.currency)
        );
        println!("{}", chart::render_history_chart(history, 96, 18));
        println!("Provider: {}", history.provider.dimmed());
        println!();
    }
}

fn format_crypto_amount(amount: f64, symbol: &str) -> String {
    let upper = symbol.to_uppercase();
    if amount >= 0.0001 {
        format!("{:.6} {}", amount, upper)
    } else {
        format!("{:.10} {}", amount, upper)
    }
}

fn format_price(price: f64, currency: &str) -> String {
    let sym = currency_symbol(currency);
    if price >= 1.0 {
        format!("{}{}", sym, format_with_commas(price, 2))
    } else if price >= 0.01 {
        format!("{}{:.4}", sym, price)
    } else {
        format!("{}{:.8}", sym, price)
    }
}

fn format_with_commas(value: f64, decimals: usize) -> String {
    let formatted = format!("{value:.decimals$}");
    let parts: Vec<&str> = formatted.split('.').collect();
    let whole = parts[0];

    let mut result = String::new();
    for (i, ch) in whole.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    let whole_formatted: String = result.chars().rev().collect();

    if parts.len() > 1 {
        format!("{}.{}", whole_formatted, parts[1])
    } else {
        whole_formatted
    }
}

fn format_market_cap(cap: f64, currency: &str) -> String {
    let sym = currency_symbol(currency);
    if cap >= 1_000_000_000_000.0 {
        format!("{}{:.2}T", sym, cap / 1_000_000_000_000.0)
    } else if cap >= 1_000_000_000.0 {
        format!("{}{:.2}B", sym, cap / 1_000_000_000.0)
    } else if cap >= 1_000_000.0 {
        format!("{}{:.2}M", sym, cap / 1_000_000.0)
    } else if cap >= 1_000.0 {
        format!("{}{:.2}K", sym, cap / 1_000.0)
    } else {
        format!("{}{:.2}", sym, cap)
    }
}

fn currency_symbol(currency: &str) -> &str {
    match currency.to_uppercase().as_str() {
        "USD" => "$",
        "EUR" => "\u{20ac}",
        "GBP" => "\u{00a3}",
        "JPY" | "CNY" => "\u{00a5}",
        "CAD" => "CA$",
        "AUD" => "A$",
        "CHF" => "CHF ",
        "BTC" => "\u{20bf}",
        _ => "",
    }
}
