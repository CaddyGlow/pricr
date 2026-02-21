use clap::Parser;
use cryptoprice::{calc, config, error, output, provider};
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::error::Result;

const APP_VERSION: &str = env!("CRYPTOPRICE_VERSION");

#[derive(Parser)]
#[command(
    name = "cryptoprice",
    version = APP_VERSION,
    about = "Fetch cryptocurrency prices from your terminal"
)]
struct Cli {
    /// Coin symbols to look up (e.g. btc eth sol)
    symbols: Vec<String>,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Price provider to use
    #[arg(long, short, default_value = config::DEFAULT_PROVIDER)]
    provider: String,

    /// Fiat currency for prices
    #[arg(long, short)]
    currency: Option<String>,

    /// API key for providers that require one
    #[arg(long, env = "COINMARKETCAP_API_KEY")]
    api_key: Option<String>,

    /// Explicit config file path (overrides XDG lookup)
    #[arg(long)]
    config: Option<PathBuf>,

    /// List available providers
    #[arg(long)]
    list_providers: bool,

    /// Increase log verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn init_logging(verbose: u8) {
    let default_level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() {
    // Load .env before CLI parsing so env-backed args (e.g. COINMARKETCAP_API_KEY) pick it up.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();
    init_logging(cli.verbose);

    if let Err(e) = run(cli).await {
        error!(error = %e, "fatal error");
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let app_config = match cli.config.as_deref() {
        Some(path) => config::load_from_path(path)?,
        None => config::load()?,
    };

    let merged_api_key = cli
        .api_key
        .or_else(|| app_config.coinmarketcap.api_key.clone());
    let providers = provider::available_providers(merged_api_key);

    let currency = cli
        .currency
        .or_else(|| app_config.defaults.currency.clone())
        .unwrap_or_else(|| config::DEFAULT_CURRENCY.to_string());

    if cli.list_providers {
        println!("Available providers:");
        for p in &providers {
            println!("  {:12} {}", p.id(), p.name());
        }
        return Ok(());
    }

    if cli.symbols.is_empty() {
        return Err(error::Error::Config(
            "no coin symbols provided -- usage: cryptoprice btc eth".into(),
        ));
    }

    let idx = provider::get_provider(&providers, &cli.provider).ok_or_else(|| {
        error::Error::Config(format!(
            "unknown provider '{}' -- use --list-providers to see options",
            cli.provider
        ))
    })?;

    let prov = &providers[idx];

    // Calc mode: detect `<number><fiat>` as first positional arg.
    if let Some(fiat) = calc::parse_fiat_amount(&cli.symbols[0]) {
        let targets: Vec<String> = cli.symbols[1..].to_vec();
        if targets.is_empty() {
            return Err(error::Error::Config(
                "calc mode requires at least one target coin -- usage: cryptoprice 3.5EUR xmr"
                    .into(),
            ));
        }

        // Partition targets into fiat currencies and crypto symbols.
        let (fiat_targets, crypto_targets): (Vec<String>, Vec<String>) =
            targets.into_iter().partition(|t| calc::is_known_fiat(t));

        info!(
            provider = prov.id(),
            amount = fiat.amount,
            currency = %fiat.currency,
            fiat_targets = ?fiat_targets,
            crypto_targets = ?crypto_targets,
            "calc mode: fetching prices for conversion"
        );

        let mut conversions: Vec<calc::Conversion> = Vec::new();
        let fiat_provider = provider::frankfurter::Frankfurter::new();

        match (fiat_targets.is_empty(), crypto_targets.is_empty()) {
            // Both fiat and crypto targets -- fetch concurrently.
            (false, false) => {
                let fiat_fut = fiat_provider.get_rates(&fiat.currency, &fiat_targets);
                let crypto_fut = prov.get_prices(&crypto_targets, &fiat.currency);

                let (fiat_result, crypto_result) = tokio::join!(fiat_fut, crypto_fut);

                let rates = fiat_result?;
                for target in &fiat_targets {
                    let upper = target.to_uppercase();
                    if let Some(&rate) = rates.get(&upper) {
                        conversions.push(calc::Conversion {
                            from_amount: fiat.amount,
                            from_currency: fiat.currency.clone(),
                            to_symbol: upper.clone(),
                            to_name: calc::fiat_name(&upper).to_string(),
                            to_amount: fiat.amount * rate,
                            rate: 1.0 / rate,
                            provider: "Frankfurter/ECB".to_string(),
                            timestamp: chrono::Utc::now(),
                        });
                    }
                }

                let prices = crypto_result?;
                for p in &prices {
                    conversions.push(calc::Conversion {
                        from_amount: fiat.amount,
                        from_currency: fiat.currency.clone(),
                        to_symbol: p.symbol.clone(),
                        to_name: p.name.clone(),
                        to_amount: fiat.amount / p.price,
                        rate: p.price,
                        provider: p.provider.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                }
            }
            // Only fiat targets.
            (false, true) => {
                let rates = fiat_provider
                    .get_rates(&fiat.currency, &fiat_targets)
                    .await?;
                for target in &fiat_targets {
                    let upper = target.to_uppercase();
                    if let Some(&rate) = rates.get(&upper) {
                        conversions.push(calc::Conversion {
                            from_amount: fiat.amount,
                            from_currency: fiat.currency.clone(),
                            to_symbol: upper.clone(),
                            to_name: calc::fiat_name(&upper).to_string(),
                            to_amount: fiat.amount * rate,
                            rate: 1.0 / rate,
                            provider: "Frankfurter/ECB".to_string(),
                            timestamp: chrono::Utc::now(),
                        });
                    }
                }
            }
            // Only crypto targets (existing behavior).
            (true, false) => {
                let prices = prov.get_prices(&crypto_targets, &fiat.currency).await?;
                for p in &prices {
                    conversions.push(calc::Conversion {
                        from_amount: fiat.amount,
                        from_currency: fiat.currency.clone(),
                        to_symbol: p.symbol.clone(),
                        to_name: p.name.clone(),
                        to_amount: fiat.amount / p.price,
                        rate: p.price,
                        provider: p.provider.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                }
            }
            // Both empty -- unreachable since we checked targets.is_empty() above.
            (true, true) => unreachable!(),
        }

        if cli.json {
            output::json::print_conversions_json(&conversions)?;
        } else {
            output::table::print_conversions_table(&conversions);
        }

        return Ok(());
    }

    info!(
        provider = prov.id(),
        symbols = ?cli.symbols,
        currency = %currency,
        "fetching prices"
    );

    let prices = prov.get_prices(&cli.symbols, &currency).await?;

    if cli.json {
        output::json::print_json(&prices)?;
    } else {
        output::table::print_table(&prices);
    }

    Ok(())
}
