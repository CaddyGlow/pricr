use cryptoprice::error::Error;
use cryptoprice::provider::coingecko::CoinGecko;
use cryptoprice::provider::coinmarketcap::CoinMarketCap;
use cryptoprice::provider::frankfurter::Frankfurter;
use cryptoprice::provider::{HistoryInterval, PriceProvider};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn coingecko_provider_fetches_and_parses_mocked_response() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "bitcoin": {
            "usd": 50000.0,
            "usd_24h_change": 1.5,
            "usd_market_cap": 999999999.0
        },
        "ethereum": {
            "usd": 3000.0,
            "usd_24h_change": -0.5,
            "usd_market_cap": 500000000.0
        }
    });

    Mock::given(method("GET"))
        .and(path("/api/v3/simple/price"))
        .and(query_param("ids", "bitcoin,ethereum"))
        .and(query_param("vs_currencies", "usd"))
        .and(query_param("include_24hr_change", "true"))
        .and(query_param("include_market_cap", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider = CoinGecko::with_base_url(format!("{}/api/v3", server.uri()));
    let symbols = vec!["btc".to_string(), "eth".to_string()];
    let prices = provider.get_prices(&symbols, "usd").await.unwrap();

    assert_eq!(prices.len(), 2);
    assert_eq!(prices[0].symbol, "BTC");
    assert_eq!(prices[0].name, "Bitcoin");
    assert!((prices[0].price - 50000.0).abs() < f64::EPSILON);
    assert_eq!(prices[0].change_24h, Some(1.5));
    assert_eq!(prices[0].market_cap, Some(999999999.0));
    assert_eq!(prices[0].currency, "USD");
    assert_eq!(prices[0].provider, "CoinGecko");

    assert_eq!(prices[1].symbol, "ETH");
    assert_eq!(prices[1].name, "Ethereum");
    assert!((prices[1].price - 3000.0).abs() < f64::EPSILON);
    assert_eq!(prices[1].change_24h, Some(-0.5));
    assert_eq!(prices[1].market_cap, Some(500000000.0));
    assert_eq!(prices[1].currency, "USD");
    assert_eq!(prices[1].provider, "CoinGecko");
}

#[tokio::test]
async fn coingecko_provider_returns_api_error_on_non_success_status() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v3/simple/price"))
        .and(query_param("ids", "bitcoin"))
        .and(query_param("vs_currencies", "usd"))
        .and(query_param("include_24hr_change", "true"))
        .and(query_param("include_market_cap", "true"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .mount(&server)
        .await;

    let provider = CoinGecko::with_base_url(format!("{}/api/v3", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::Api(ref msg)) if msg.contains("429")));
}

#[tokio::test]
async fn coingecko_provider_fetches_history_for_chart_mode() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "prices": [
            [1700000000000_i64, 40000.0],
            [1700086400000_i64, 41000.0],
            [1700172800000_i64, 40500.0]
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v3/coins/bitcoin/market_chart"))
        .and(query_param("vs_currency", "usd"))
        .and(query_param("days", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider = CoinGecko::with_base_url(format!("{}/api/v3", server.uri()));
    let symbols = vec!["btc".to_string()];
    let history = provider
        .get_price_history(&symbols, "usd", 7, HistoryInterval::Daily)
        .await
        .expect("history should parse");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].symbol, "BTC");
    assert_eq!(history[0].currency, "USD");
    assert_eq!(history[0].provider, "CoinGecko");
    assert_eq!(history[0].points.len(), 3);
    assert!((history[0].points[0].price - 40000.0).abs() < f64::EPSILON);
    assert!((history[0].points[2].price - 40500.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn coinmarketcap_provider_fetches_history_for_chart_mode() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "status": { "error_message": null },
        "data": {
            "name": "Bitcoin",
            "symbol": "BTC",
            "quotes": [
                {
                    "timestamp": "2026-02-19T00:00:00.000Z",
                    "quote": { "USD": { "price": 96000.0 } }
                },
                {
                    "timestamp": "2026-02-20T00:00:00.000Z",
                    "quote": { "USD": { "price": 97000.0 } }
                },
                {
                    "timestamp": "2026-02-21T00:00:00.000Z",
                    "quote": { "USD": { "price": 95500.0 } }
                }
            ]
        }
    });

    Mock::given(method("GET"))
        .and(path("/v1/cryptocurrency/quotes/historical"))
        .and(query_param("symbol", "BTC"))
        .and(query_param("convert", "USD"))
        .and(query_param("interval", "daily"))
        .and(header("X-CMC_PRO_API_KEY", "test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string()];
    let history = provider
        .get_price_history(&symbols, "usd", 7, HistoryInterval::Daily)
        .await
        .expect("history should parse");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].symbol, "BTC");
    assert_eq!(history[0].name, "Bitcoin");
    assert_eq!(history[0].currency, "USD");
    assert_eq!(history[0].provider, "CoinMarketCap");
    assert_eq!(history[0].points.len(), 3);
    assert!((history[0].points[0].price - 96000.0).abs() < f64::EPSILON);
    assert!((history[0].points[2].price - 95500.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn coinmarketcap_provider_fetches_history_from_web_chart_endpoint() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "data": {
            "points": [
                { "s": "1767787200", "v": [92074.48, 1.0, 1.0], "c": {} },
                { "s": "1767790800", "v": [91935.38, 1.0, 1.0], "c": {} },
                { "s": "1767794400", "v": [91990.69, 1.0, 1.0], "c": {} }
            ]
        },
        "status": {
            "error_code": "0",
            "error_message": "SUCCESS"
        }
    });

    Mock::given(method("GET"))
        .and(path("/data-api/v3.3/cryptocurrency/detail/chart"))
        .and(query_param("id", "1"))
        .and(query_param("interval", "1h"))
        .and(query_param("convertId", "2781"))
        .and(query_param("range", "1M"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string()];
    let history = provider
        .get_price_history(&symbols, "usd", 30, HistoryInterval::Hourly)
        .await
        .expect("history should parse from web chart endpoint");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].symbol, "BTC");
    assert_eq!(history[0].currency, "USD");
    assert_eq!(history[0].points.len(), 3);
    assert!((history[0].points[0].price - 92074.48).abs() < f64::EPSILON);
    assert!((history[0].points[2].price - 91990.69).abs() < f64::EPSILON);
}

#[tokio::test]
async fn coinmarketcap_provider_resolves_coin_id_from_coin_catalog() {
    let server = MockServer::start().await;

    let catalog = serde_json::json!([
        {
            "symbol": "BCH",
            "name": "Bitcoin Cash",
            "id": 1831,
            "slug": "bitcoin-cash",
            "levels": ["beginner"]
        }
    ]);

    let chart_response = serde_json::json!({
        "data": {
            "points": [
                { "s": "1767787200", "v": [443.12, 1.0, 1.0], "c": {} },
                { "s": "1767790800", "v": [447.55, 1.0, 1.0], "c": {} }
            ]
        },
        "status": {
            "error_code": "0",
            "error_message": "SUCCESS"
        }
    });

    Mock::given(method("GET"))
        .and(path("/whitepaper/summaries/coins.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalog))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/data-api/v3.3/cryptocurrency/detail/chart"))
        .and(query_param("id", "1831"))
        .and(query_param("interval", "1h"))
        .and(query_param("convertId", "2781"))
        .and(query_param("range", "7D"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chart_response))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["bch".to_string()];
    let history = provider
        .get_price_history(&symbols, "usd", 7, HistoryInterval::Hourly)
        .await
        .expect("history should parse from catalog-derived coin id");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].symbol, "BCH");
    assert_eq!(history[0].name, "Bitcoin Cash");
    assert_eq!(history[0].points.len(), 2);
    assert!((history[0].points[0].price - 443.12).abs() < f64::EPSILON);
}

#[tokio::test]
async fn frankfurter_provider_fetches_history_for_fiat_chart_mode() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "amount": 1.0,
        "base": "USD",
        "start_date": "2026-02-15",
        "end_date": "2026-02-22",
        "rates": {
            "2026-02-20": { "EUR": 0.92, "GBP": 0.79 },
            "2026-02-21": { "EUR": 0.93, "GBP": 0.80 }
        }
    });

    Mock::given(method("GET"))
        .and(query_param("from", "USD"))
        .and(query_param("to", "EUR,GBP"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider = Frankfurter::with_base_url(format!("{}/v1", server.uri()));
    let targets = vec!["eur".to_string(), "gbp".to_string()];
    let history = provider
        .get_history("usd", &targets, 7)
        .await
        .expect("fiat history should parse");

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].currency, "USD");
    assert_eq!(history[0].provider, "Frankfurter/ECB");
    assert_eq!(history[0].points.len(), 2);
}

#[tokio::test]
async fn coingecko_provider_returns_parse_error_on_malformed_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v3/simple/price"))
        .and(query_param("ids", "bitcoin"))
        .and(query_param("vs_currencies", "usd"))
        .and(query_param("include_24hr_change", "true"))
        .and(query_param("include_market_cap", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{not-json"))
        .mount(&server)
        .await;

    let provider = CoinGecko::with_base_url(format!("{}/api/v3", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::Parse(ref msg)) if msg.contains("CoinGecko JSON")));
}

#[tokio::test]
async fn coingecko_provider_returns_no_results_when_response_is_empty() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v3/simple/price"))
        .and(query_param("ids", "bitcoin"))
        .and(query_param("vs_currencies", "usd"))
        .and(query_param("include_24hr_change", "true"))
        .and(query_param("include_market_cap", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let provider = CoinGecko::with_base_url(format!("{}/api/v3", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::NoResults)));
}

#[tokio::test]
async fn coinmarketcap_provider_fetches_and_parses_mocked_response() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "status": {
            "error_message": null
        },
        "data": {
            "BTC": {
                "name": "Bitcoin",
                "symbol": "BTC",
                "quote": {
                    "USD": {
                        "price": 50000.0,
                        "percent_change_24h": 2.25,
                        "market_cap": 1000000000.0
                    }
                }
            },
            "ETH": {
                "name": "Ethereum",
                "symbol": "ETH",
                "quote": {
                    "USD": {
                        "price": 3000.0,
                        "percent_change_24h": -1.2,
                        "market_cap": 500000000.0
                    }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/v1/cryptocurrency/quotes/latest"))
        .and(query_param("symbol", "BTC,ETH"))
        .and(query_param("convert", "USD"))
        .and(header("X-CMC_PRO_API_KEY", "test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string(), "eth".to_string()];
    let prices = provider.get_prices(&symbols, "usd").await.unwrap();

    assert_eq!(prices.len(), 2);
    assert_eq!(prices[0].symbol, "BTC");
    assert_eq!(prices[0].name, "Bitcoin");
    assert!((prices[0].price - 50000.0).abs() < f64::EPSILON);
    assert_eq!(prices[0].change_24h, Some(2.25));
    assert_eq!(prices[0].market_cap, Some(1000000000.0));
    assert_eq!(prices[0].currency, "USD");
    assert_eq!(prices[0].provider, "CoinMarketCap");

    assert_eq!(prices[1].symbol, "ETH");
    assert_eq!(prices[1].name, "Ethereum");
    assert!((prices[1].price - 3000.0).abs() < f64::EPSILON);
    assert_eq!(prices[1].change_24h, Some(-1.2));
    assert_eq!(prices[1].market_cap, Some(500000000.0));
    assert_eq!(prices[1].currency, "USD");
    assert_eq!(prices[1].provider, "CoinMarketCap");
}

#[tokio::test]
async fn coinmarketcap_provider_returns_api_error_on_non_success_status() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/cryptocurrency/quotes/latest"))
        .and(query_param("symbol", "BTC"))
        .and(query_param("convert", "USD"))
        .and(header("X-CMC_PRO_API_KEY", "test-api-key"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::Api(ref msg)) if msg.contains("500")));
}

#[tokio::test]
async fn coinmarketcap_provider_returns_parse_error_on_malformed_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/cryptocurrency/quotes/latest"))
        .and(query_param("symbol", "BTC"))
        .and(query_param("convert", "USD"))
        .and(header("X-CMC_PRO_API_KEY", "test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{broken-json"))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::Parse(ref msg)) if msg.contains("CMC JSON")));
}

#[tokio::test]
async fn coinmarketcap_provider_returns_no_results_when_response_has_no_data() {
    let server = MockServer::start().await;
    let response = serde_json::json!({
        "status": {
            "error_message": null
        },
        "data": {}
    });

    Mock::given(method("GET"))
        .and(path("/v1/cryptocurrency/quotes/latest"))
        .and(query_param("symbol", "BTC"))
        .and(query_param("convert", "USD"))
        .and(header("X-CMC_PRO_API_KEY", "test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;

    let provider =
        CoinMarketCap::with_base_url("test-api-key".to_string(), format!("{}/v1", server.uri()));
    let symbols = vec!["btc".to_string()];
    let result = provider.get_prices(&symbols, "usd").await;

    assert!(matches!(result, Err(Error::NoResults)));
}
