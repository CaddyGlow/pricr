use chrono::{Datelike, NaiveDate};
use clap::Parser;
use pricr::{calc, config, error, output, provider};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::error::Result;

const APP_VERSION: &str = env!("PRICR_VERSION");
const MAX_CHART_FETCH_DAYS: u32 = 36_500;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SamplingArg {
    Auto,
    Hourly,
    Daily,
}

impl From<SamplingArg> for provider::HistoryInterval {
    fn from(value: SamplingArg) -> Self {
        match value {
            SamplingArg::Auto => Self::Auto,
            SamplingArg::Hourly => Self::Hourly,
            SamplingArg::Daily => Self::Daily,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ChartRangeArg {
    #[value(name = "1D")]
    OneDay,
    #[value(name = "5D")]
    FiveDays,
    #[value(name = "1M")]
    OneMonth,
    #[value(name = "6M")]
    SixMonths,
    #[value(name = "YTD")]
    Ytd,
    #[value(name = "1Y")]
    OneYear,
    #[value(name = "5Y")]
    FiveYears,
    #[value(name = "ALL")]
    All,
}

impl ChartRangeArg {
    fn label(self) -> &'static str {
        match self {
            Self::OneDay => "1D",
            Self::FiveDays => "5D",
            Self::OneMonth => "1M",
            Self::SixMonths => "6M",
            Self::Ytd => "YTD",
            Self::OneYear => "1Y",
            Self::FiveYears => "5Y",
            Self::All => "ALL",
        }
    }

    fn start_date(self, end_date: NaiveDate) -> Option<NaiveDate> {
        match self {
            Self::OneDay => Some(end_date - chrono::Duration::days(1)),
            Self::FiveDays => Some(end_date - chrono::Duration::days(5)),
            Self::OneMonth => end_date
                .checked_sub_months(chrono::Months::new(1))
                .or(Some(end_date - chrono::Duration::days(30))),
            Self::SixMonths => end_date
                .checked_sub_months(chrono::Months::new(6))
                .or(Some(end_date - chrono::Duration::days(182))),
            Self::Ytd => chrono::NaiveDate::from_ymd_opt(end_date.year(), 1, 1),
            Self::OneYear => end_date
                .checked_sub_months(chrono::Months::new(12))
                .or(Some(end_date - chrono::Duration::days(365))),
            Self::FiveYears => end_date
                .checked_sub_months(chrono::Months::new(60))
                .or(Some(end_date - chrono::Duration::days(365 * 5))),
            Self::All => None,
        }
    }
}

fn parse_chart_end_date(raw: &str) -> std::result::Result<NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| "invalid end date, expected format YYYY-MM-DD".to_string())
}

fn format_chart_range_label(
    start_date: Option<NaiveDate>,
    end_date: NaiveDate,
    fallback_interval: ChartRangeArg,
) -> String {
    match start_date {
        Some(start) => format!(
            "{}..{}",
            start.format("%Y-%m-%d"),
            end_date.format("%Y-%m-%d")
        ),
        None => fallback_interval.label().to_string(),
    }
}

fn resolve_search_query(cli: &Cli) -> Option<String> {
    if let Some(query) = cli.search.as_deref() {
        return Some(query.trim().to_string());
    }

    if !cli.symbols.is_empty() && cli.symbols[0].eq_ignore_ascii_case("search") {
        let mut tokens: Vec<String> = cli
            .symbols
            .iter()
            .skip(1)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if let Some(first) = tokens.first()
            && first.eq_ignore_ascii_case("search")
        {
            tokens.remove(0);
        }

        return Some(tokens.join(" ").trim().to_string());
    }

    None
}

fn resolve_watchlist<'a>(
    watchlists: &'a HashMap<String, Vec<String>>,
    name: &str,
) -> Option<&'a Vec<String>> {
    watchlists
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, symbols)| symbols)
}

fn expand_symbol_tokens(
    raw_symbols: &[String],
    watchlists: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    let mut expanded = Vec::new();

    for token in raw_symbols {
        if let Some(name) = token.strip_prefix('@') {
            let trimmed_name = name.trim();
            if trimmed_name.is_empty() {
                return Err(error::Error::Config(
                    "watchlist name cannot be empty after '@'".into(),
                ));
            }

            let symbols = resolve_watchlist(watchlists, trimmed_name).ok_or_else(|| {
                error::Error::Config(format!(
                    "unknown watchlist '{}' -- define it under [watchlists] in config",
                    trimmed_name
                ))
            })?;

            let mut added = 0usize;
            for symbol in symbols {
                let trimmed_symbol = symbol.trim();
                if trimmed_symbol.is_empty() {
                    continue;
                }
                expanded.push(trimmed_symbol.to_string());
                added += 1;
            }

            if added == 0 {
                return Err(error::Error::Config(format!(
                    "watchlist '{}' is empty -- add symbols under [watchlists].{}",
                    trimmed_name, trimmed_name
                )));
            }

            continue;
        }

        expanded.push(token.clone());
    }

    Ok(expanded)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TickerMatchKey {
    symbol: String,
    name: String,
    exchange: String,
    asset_type: String,
}

fn ticker_match_key(candidate: &provider::TickerMatch) -> TickerMatchKey {
    TickerMatchKey {
        symbol: candidate.symbol.trim().to_uppercase(),
        name: candidate.name.trim().to_lowercase(),
        exchange: candidate.exchange.trim().to_lowercase(),
        asset_type: candidate.asset_type.trim().to_lowercase(),
    }
}

fn append_provider_name(existing: &mut String, provider_name: &str) {
    if existing
        .split(',')
        .map(str::trim)
        .any(|name| name.eq_ignore_ascii_case(provider_name))
    {
        return;
    }

    if existing.trim().is_empty() {
        *existing = provider_name.to_string();
        return;
    }

    existing.push_str(", ");
    existing.push_str(provider_name);
}

fn provider_ids_for_indices(
    providers: &[Box<dyn provider::PriceProvider>],
    indices: &[usize],
) -> Vec<String> {
    indices
        .iter()
        .filter_map(|&idx| providers.get(idx).map(|p| p.id().to_string()))
        .collect()
}

fn resolve_provider_indices(
    providers: &[Box<dyn provider::PriceProvider>],
    explicit_provider: Option<&str>,
    configured_order: Option<&[String]>,
) -> Result<Vec<usize>> {
    if let Some(provider_id) = explicit_provider {
        let requested = provider_id.trim();
        if requested.is_empty() {
            return Err(error::Error::Config(
                "provider cannot be empty -- use --list-providers to see options".into(),
            ));
        }

        let idx = provider::get_provider(providers, requested).ok_or_else(|| {
            error::Error::Config(format!(
                "unknown provider '{}' -- use --list-providers to see options",
                provider_id
            ))
        })?;
        return Ok(vec![idx]);
    }

    let mut ordered = Vec::new();
    let mut seen_ids = HashSet::new();

    if let Some(config_ids) = configured_order {
        for configured_id in config_ids {
            let raw = configured_id.trim();
            if raw.is_empty() {
                continue;
            }

            let normalized = raw.to_ascii_lowercase();
            if !seen_ids.insert(normalized.clone()) {
                continue;
            }

            let idx = provider::get_provider(providers, &normalized).ok_or_else(|| {
                error::Error::Config(format!(
                    "unknown provider '{}' in [defaults].provider_order -- use --list-providers to see options",
                    configured_id
                ))
            })?;
            ordered.push(idx);
        }
    }

    for (idx, prov) in providers.iter().enumerate() {
        let id = prov.id().to_string();
        if seen_ids.insert(id) {
            ordered.push(idx);
        }
    }

    if ordered.is_empty() {
        return Err(error::Error::Config(
            "no providers available -- use --list-providers to verify installation".into(),
        ));
    }

    Ok(ordered)
}

fn is_ignorable_search_error(err: &error::Error) -> bool {
    match err {
        error::Error::NoResults => true,
        error::Error::Config(message) => message
            .to_ascii_lowercase()
            .contains("does not support ticker search"),
        _ => false,
    }
}

fn is_ignorable_price_error(err: &error::Error) -> bool {
    match err {
        error::Error::NoResults => true,
        error::Error::Config(message) => {
            message.to_ascii_lowercase().contains("requires --api-key")
        }
        _ => false,
    }
}

async fn search_tickers_across_providers(
    providers: &[Box<dyn provider::PriceProvider>],
    provider_indices: &[usize],
    query: &str,
    limit: usize,
) -> Result<Vec<provider::TickerMatch>> {
    let mut matches: Vec<provider::TickerMatch> = Vec::new();
    let mut by_key: HashMap<TickerMatchKey, usize> = HashMap::new();
    let mut last_non_ignorable_error = None;

    for provider_idx in provider_indices {
        let prov = &providers[*provider_idx];
        match prov.search_tickers(query, limit).await {
            Ok(found) => {
                for candidate in found {
                    let key = ticker_match_key(&candidate);
                    if let Some(existing_idx) = by_key.get(&key).copied() {
                        append_provider_name(
                            &mut matches[existing_idx].provider,
                            &candidate.provider,
                        );
                        continue;
                    }

                    if matches.len() >= limit {
                        continue;
                    }

                    let idx = matches.len();
                    by_key.insert(key, idx);
                    matches.push(candidate);
                }
            }
            Err(err) if is_ignorable_search_error(&err) => {
                info!(provider = prov.id(), error = %err, "skipping unsupported or empty search provider");
            }
            Err(err) => {
                warn!(provider = prov.id(), error = %err, "ticker search failed for provider");
                last_non_ignorable_error = Some(err);
            }
        }
    }

    if matches.is_empty() {
        if let Some(err) = last_non_ignorable_error {
            return Err(err);
        }
        return Err(error::Error::NoResults);
    }

    matches.truncate(limit);
    Ok(matches)
}

async fn fetch_prices_with_provider_fallback(
    providers: &[Box<dyn provider::PriceProvider>],
    provider_indices: &[usize],
    symbols: &[String],
    currency: &str,
) -> Result<Vec<provider::CoinPrice>> {
    let mut pending: Vec<(usize, String)> = symbols
        .iter()
        .enumerate()
        .map(|(idx, symbol)| (idx, symbol.clone()))
        .collect();
    let mut resolved: Vec<Option<provider::CoinPrice>> = vec![None; symbols.len()];
    let mut last_non_ignorable_error = None;

    for provider_idx in provider_indices {
        if pending.is_empty() {
            break;
        }

        let request_symbols: Vec<String> =
            pending.iter().map(|(_, symbol)| symbol.clone()).collect();
        let prov = &providers[*provider_idx];

        match prov.get_prices(&request_symbols, currency).await {
            Ok(found) => {
                let mut found_by_symbol: HashMap<String, Vec<provider::CoinPrice>> = HashMap::new();
                for price in found {
                    found_by_symbol
                        .entry(price.symbol.trim().to_uppercase())
                        .or_default()
                        .push(price);
                }

                let mut next_pending = Vec::new();
                for (original_idx, symbol) in pending {
                    let key = symbol.trim().to_uppercase();
                    let price = found_by_symbol
                        .get_mut(&key)
                        .and_then(|bucket| bucket.pop());
                    if let Some(price) = price {
                        resolved[original_idx] = Some(price);
                    } else {
                        next_pending.push((original_idx, symbol));
                    }
                }
                pending = next_pending;
            }
            Err(err) if is_ignorable_price_error(&err) => {
                info!(provider = prov.id(), error = %err, "skipping provider during price fallback");
            }
            Err(err) => {
                warn!(provider = prov.id(), error = %err, "price lookup failed for provider");
                last_non_ignorable_error = Some(err);
            }
        }
    }

    let prices: Vec<provider::CoinPrice> = resolved.into_iter().flatten().collect();
    if prices.is_empty() {
        if let Some(err) = last_non_ignorable_error {
            return Err(err);
        }
        return Err(error::Error::NoResults);
    }

    Ok(prices)
}

#[derive(Parser)]
#[command(
    name = "pricr",
    version = APP_VERSION,
    about = "Fetch crypto and stock prices from your terminal"
)]
struct Cli {
    /// Asset symbols to look up (e.g. btc eth aapl msft) or watchlists via @name
    symbols: Vec<String>,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Plot historical price charts
    #[arg(long)]
    chart: bool,

    /// Chart interval preset (1D, 5D, 1M, 6M, YTD, 1Y, 5Y, ALL)
    #[arg(long, value_enum, default_value_t = ChartRangeArg::OneMonth)]
    interval: ChartRangeArg,

    /// Sampling density for chart mode
    #[arg(long, value_enum, default_value_t = SamplingArg::Auto)]
    sampling: SamplingArg,

    /// End date for chart mode in UTC (YYYY-MM-DD)
    #[arg(long, value_parser = parse_chart_end_date, requires = "chart")]
    end_date: Option<NaiveDate>,

    /// Start date for chart mode in UTC (YYYY-MM-DD). Overrides --interval preset.
    #[arg(long, value_parser = parse_chart_end_date, requires = "chart")]
    start_date: Option<NaiveDate>,

    /// Price provider to use
    #[arg(long, short)]
    provider: Option<String>,

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

    /// Search ticker symbols by keyword (provider-dependent)
    #[arg(
        long,
        short = 's',
        conflicts_with = "chart",
        conflicts_with = "symbols"
    )]
    search: Option<String>,

    /// Max ticker search results
    #[arg(
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u8).range(1..=50)
    )]
    search_limit: u8,

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

fn compute_chart_fetch_days(start_date: Option<NaiveDate>) -> u32 {
    match start_date {
        Some(start) => {
            let today = chrono::Utc::now().date_naive();
            let days = (today - start).num_days().max(1);
            (days as u32).min(MAX_CHART_FETCH_DAYS)
        }
        None => MAX_CHART_FETCH_DAYS,
    }
}

fn filter_histories_by_time_window(
    histories: &mut Vec<provider::PriceHistory>,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: chrono::DateTime<chrono::Utc>,
) {
    for history in histories.iter_mut() {
        history.points.retain(|point| {
            point.timestamp <= end && start.map(|s| point.timestamp >= s).unwrap_or(true)
        });
    }

    histories.retain(|history| !history.points.is_empty());
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

    let search_query = resolve_search_query(&cli);

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

    let provider_indices = resolve_provider_indices(
        &providers,
        cli.provider.as_deref(),
        app_config.defaults.provider_order.as_deref(),
    )?;
    let primary_provider_idx = provider_indices[0];
    let prov = &providers[primary_provider_idx];

    if let Some(query) = search_query {
        if query.is_empty() {
            return Err(error::Error::Config(
                "search mode requires a query -- usage: pricr --search apple".into(),
            ));
        }

        let matches = if cli.provider.is_some() {
            info!(provider = prov.id(), query = %query, limit = cli.search_limit, "searching tickers");
            prov.search_tickers(&query, cli.search_limit as usize)
                .await?
        } else {
            let ordered_ids = provider_ids_for_indices(&providers, &provider_indices);
            info!(
                providers = ?ordered_ids,
                query = %query,
                limit = cli.search_limit,
                "searching tickers across providers"
            );
            search_tickers_across_providers(
                &providers,
                &provider_indices,
                &query,
                cli.search_limit as usize,
            )
            .await?
        };

        if cli.json {
            output::json::print_ticker_matches_json(&matches)?;
        } else {
            output::table::print_ticker_matches_table(&matches);
        }

        return Ok(());
    }

    let symbols = expand_symbol_tokens(&cli.symbols, &app_config.watchlists)?;

    if symbols.is_empty() {
        return Err(error::Error::Config(
            "no symbols provided -- usage: pricr btc eth".into(),
        ));
    }

    let chart_end_date = cli
        .end_date
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    if chart_end_date > chrono::Utc::now().date_naive() {
        return Err(error::Error::Config(
            "chart end date cannot be in the future".into(),
        ));
    }

    let chart_start_date = cli
        .start_date
        .or_else(|| cli.interval.start_date(chart_end_date));
    if let Some(start) = chart_start_date
        && start > chart_end_date
    {
        return Err(error::Error::Config(
            "chart start date cannot be after chart end date".into(),
        ));
    }

    let chart_range_label =
        format_chart_range_label(chart_start_date, chart_end_date, cli.interval);
    let chart_start_ts = chart_start_date
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc());
    let chart_end_ts = chart_end_date
        .and_hms_opt(23, 59, 59)
        .ok_or_else(|| error::Error::Config("invalid chart end date".into()))?
        .and_utc();
    let chart_fetch_days = compute_chart_fetch_days(chart_start_date);

    if cli.chart && calc::is_known_fiat(&symbols[0]) {
        let base = symbols[0].to_uppercase();
        let targets: Vec<String> = symbols[1..].iter().map(|s| s.to_uppercase()).collect();

        if targets.is_empty() {
            return Err(error::Error::Config(
                "fiat chart mode requires a base and at least one target currency -- usage: pricr --chart usd eur"
                    .into(),
            ));
        }

        if targets.iter().any(|t| !calc::is_known_fiat(t)) {
            return Err(error::Error::Config(
                "fiat chart mode only supports fiat currency codes (example: usd eur gbp)".into(),
            ));
        }

        if matches!(cli.sampling, SamplingArg::Hourly) {
            return Err(error::Error::Config(
                "fiat chart mode supports daily history only -- use --sampling auto or --sampling daily"
                    .into(),
            ));
        }

        info!(
            base = %base,
            targets = ?targets,
            range = %chart_range_label,
            start_date = ?chart_start_date,
            end_date = %chart_end_date,
            fetch_days = chart_fetch_days,
            "fetching fiat historical rates"
        );

        let fiat_provider = provider::frankfurter::Frankfurter::new();
        let mut histories = fiat_provider
            .get_history(&base, &targets, chart_fetch_days)
            .await?;
        filter_histories_by_time_window(&mut histories, chart_start_ts, chart_end_ts);
        if histories.is_empty() {
            return Err(error::Error::NoResults);
        }

        if cli.json {
            output::json::print_history_json(&histories)?;
        } else {
            output::table::print_history_charts(
                &histories,
                &chart_range_label,
                provider::HistoryInterval::Daily,
            );
        }

        return Ok(());
    }

    // Calc mode: detect `<number><fiat>` as first positional arg.
    if let Some(fiat) = calc::parse_fiat_amount(&symbols[0]) {
        if cli.chart {
            return Err(error::Error::Config(
                "chart mode is only available for direct symbol lookup".into(),
            ));
        }

        let targets: Vec<String> = symbols[1..].to_vec();
        if targets.is_empty() {
            return Err(error::Error::Config(
                "calc mode requires at least one target coin -- usage: pricr 3.5EUR xmr".into(),
            ));
        }

        // Partition targets into fiat currencies and crypto symbols.
        let (fiat_targets, crypto_targets): (Vec<String>, Vec<String>) =
            targets.into_iter().partition(|t| calc::is_known_fiat(t));

        if cli.provider.is_some() {
            info!(
                provider = prov.id(),
                amount = fiat.amount,
                currency = %fiat.currency,
                fiat_targets = ?fiat_targets,
                crypto_targets = ?crypto_targets,
                "calc mode: fetching prices for conversion"
            );
        } else {
            let ordered_ids = provider_ids_for_indices(&providers, &provider_indices);
            info!(
                providers = ?ordered_ids,
                amount = fiat.amount,
                currency = %fiat.currency,
                fiat_targets = ?fiat_targets,
                crypto_targets = ?crypto_targets,
                "calc mode: fetching prices for conversion with provider fallback"
            );
        }

        let mut conversions: Vec<calc::Conversion> = Vec::new();
        let fiat_provider = provider::frankfurter::Frankfurter::new();

        match (fiat_targets.is_empty(), crypto_targets.is_empty()) {
            // Both fiat and crypto targets -- fetch concurrently.
            (false, false) => {
                let fiat_fut = fiat_provider.get_rates(&fiat.currency, &fiat_targets);
                let crypto_fut = async {
                    if cli.provider.is_some() {
                        prov.get_prices(&crypto_targets, &fiat.currency).await
                    } else {
                        fetch_prices_with_provider_fallback(
                            &providers,
                            &provider_indices,
                            &crypto_targets,
                            &fiat.currency,
                        )
                        .await
                    }
                };

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
                let prices = if cli.provider.is_some() {
                    prov.get_prices(&crypto_targets, &fiat.currency).await?
                } else {
                    fetch_prices_with_provider_fallback(
                        &providers,
                        &provider_indices,
                        &crypto_targets,
                        &fiat.currency,
                    )
                    .await?
                };
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

    if cli.chart {
        info!(
            provider = prov.id(),
            symbols = ?symbols,
            currency = %currency,
            range = %chart_range_label,
            start_date = ?chart_start_date,
            end_date = %chart_end_date,
            fetch_days = chart_fetch_days,
            "fetching historical prices"
        );

        let mut histories = match prov
            .get_price_history_window(
                &symbols,
                &currency,
                chart_start_ts,
                chart_end_ts,
                cli.sampling.into(),
            )
            .await
        {
            Ok(histories) => histories,
            Err(error::Error::Config(message))
                if message.contains("does not support explicit chart date windows") =>
            {
                prov.get_price_history(&symbols, &currency, chart_fetch_days, cli.sampling.into())
                    .await?
            }
            Err(other) => return Err(other),
        };
        filter_histories_by_time_window(&mut histories, chart_start_ts, chart_end_ts);
        if histories.is_empty() {
            return Err(error::Error::NoResults);
        }

        if cli.json {
            output::json::print_history_json(&histories)?;
        } else {
            output::table::print_history_charts(
                &histories,
                &chart_range_label,
                cli.sampling.into(),
            );
        }

        return Ok(());
    }

    let prices = if cli.provider.is_some() {
        info!(
            provider = prov.id(),
            symbols = ?symbols,
            currency = %currency,
            "fetching prices"
        );
        prov.get_prices(&symbols, &currency).await?
    } else {
        let ordered_ids = provider_ids_for_indices(&providers, &provider_indices);
        info!(
            providers = ?ordered_ids,
            symbols = ?symbols,
            currency = %currency,
            "fetching prices with provider fallback"
        );
        fetch_prices_with_provider_fallback(&providers, &provider_indices, &symbols, &currency)
            .await?
    };

    if cli.json {
        output::json::print_json(&prices)?;
    } else {
        output::table::print_table(&prices);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watchlists_for_tests() -> HashMap<String, Vec<String>> {
        HashMap::from([
            (
                "commodities".to_string(),
                vec!["GC=F".to_string(), "SI=F".to_string(), "CL=F".to_string()],
            ),
            (
                "metals".to_string(),
                vec!["GC=F".to_string(), "SI=F".to_string()],
            ),
        ])
    }

    #[test]
    fn append_provider_name_adds_unique_values() {
        let mut provider = "Yahoo Finance".to_string();
        append_provider_name(&mut provider, "Stooq");
        append_provider_name(&mut provider, "stooq");
        assert_eq!(provider, "Yahoo Finance, Stooq");
    }

    #[test]
    fn resolve_provider_indices_uses_configured_order_then_remaining() {
        let providers = provider::available_providers(None);
        let configured = vec!["yahoo".to_string(), "coingecko".to_string()];

        let indices = resolve_provider_indices(&providers, None, Some(&configured)).unwrap();
        let ids = provider_ids_for_indices(&providers, &indices);

        assert!(ids.len() >= 4);
        assert_eq!(ids[0], "yahoo");
        assert_eq!(ids[1], "coingecko");
        assert!(ids.contains(&"stooq".to_string()));
        assert!(ids.contains(&"cmc".to_string()));
    }

    #[test]
    fn resolve_provider_indices_rejects_unknown_configured_provider() {
        let providers = provider::available_providers(None);
        let configured = vec!["not-a-provider".to_string()];

        let err = resolve_provider_indices(&providers, None, Some(&configured)).unwrap_err();
        match err {
            error::Error::Config(message) => {
                assert!(message.contains("[defaults].provider_order"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn expand_symbol_tokens_resolves_watchlists() {
        let raw = vec!["@commodities".to_string(), "btc".to_string()];
        let expanded = expand_symbol_tokens(&raw, &watchlists_for_tests()).unwrap();

        assert_eq!(expanded, vec!["GC=F", "SI=F", "CL=F", "btc"]);
    }

    #[test]
    fn expand_symbol_tokens_resolves_watchlists_case_insensitively() {
        let raw = vec!["@MeTaLs".to_string()];
        let expanded = expand_symbol_tokens(&raw, &watchlists_for_tests()).unwrap();

        assert_eq!(expanded, vec!["GC=F", "SI=F"]);
    }

    #[test]
    fn expand_symbol_tokens_rejects_unknown_watchlist() {
        let raw = vec!["@unknown".to_string()];
        let err = expand_symbol_tokens(&raw, &watchlists_for_tests()).unwrap_err();

        match err {
            error::Error::Config(message) => {
                assert!(message.contains("unknown watchlist"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
