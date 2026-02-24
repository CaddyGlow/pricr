use pricr::error::Error;
use pricr::provider::PriceProvider;
use pricr::provider::coingecko::CoinGecko;
use pricr::provider::coinmarketcap::CoinMarketCap;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn coingecko_replay_fixture_parses_like_real_response() {
    let server = MockServer::start().await;
    let response: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/coingecko/simple_price_btc_eth_usd.json",
    ))
    .expect("coingecko fixture must be valid JSON");

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
    let prices = provider
        .get_prices(&symbols, "usd")
        .await
        .expect("fixture payload should parse");

    assert_eq!(prices.len(), 2);
    assert_eq!(prices[0].symbol, "BTC");
    assert_eq!(prices[0].provider, "CoinGecko");
    assert_eq!(prices[1].symbol, "ETH");
    assert_eq!(prices[1].provider, "CoinGecko");
}

#[tokio::test]
async fn coinmarketcap_replay_fixture_parses_like_real_response() {
    let server = MockServer::start().await;
    let response: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/coinmarketcap/quotes_latest_btc_eth_usd.json",
    ))
    .expect("cmc fixture must be valid JSON");

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
    let prices = provider
        .get_prices(&symbols, "usd")
        .await
        .expect("fixture payload should parse");

    assert_eq!(prices.len(), 2);
    assert_eq!(prices[0].symbol, "BTC");
    assert_eq!(prices[0].provider, "CoinMarketCap");
    assert_eq!(prices[1].symbol, "ETH");
    assert_eq!(prices[1].provider, "CoinMarketCap");
}

#[tokio::test]
async fn coinmarketcap_replay_error_fixture_returns_api_error() {
    let server = MockServer::start().await;
    let response: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/coinmarketcap/quotes_latest_error.json",
    ))
    .expect("cmc error fixture must be valid JSON");

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

    assert!(
        matches!(result, Err(Error::Api(ref msg)) if msg.contains("invalid")),
        "expected API error from replay fixture, got: {result:?}"
    );
}
