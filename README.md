# pricr

`pricr` is a Rust CLI for fast crypto and stock price lookup plus fiat conversion from the terminal.

Requirements: Rust 1.85+ (edition 2024).

## Install

Install with Cargo from this repository:

```sh
cargo install --locked --git https://github.com/CaddyGlow/pricr pricr
```

Tip: pin installs with `--tag <version>` or `--rev <commit>` when you need reproducible CI/dev environments.

Or build from source:

```sh
cargo build --release
```

## Install with Nix

Build from this repository:

```sh
nix build .#pricr
```

Run directly:

```sh
nix run .#pricr -- btc eth
```

## Install on NixOS

### From this repository as a flake input

Add `pricr` as an input in your system flake and include it in
`environment.systemPackages`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    pricr.url = "github:CaddyGlow/pricr";
  };

  outputs = { nixpkgs, pricr, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            pricr.packages.${pkgs.system}.pricr
          ];
        })
      ];
    };
  };
}
```

Apply it with:

```sh
sudo nixos-rebuild switch --flake .#my-host
```

### After `pricr` is in nixpkgs

Install directly from `pkgs`:

```nix
{ pkgs, ... }:
{
  environment.systemPackages = with pkgs; [
    pricr
  ];
}
```

## Docker Image

Release tags publish a multi-arch image to GHCR:

- `ghcr.io/caddyglow/pricr`
- Platforms: `linux/amd64`, `linux/arm64`

Published tags:

- `vX.Y.Z` (git tag name)
- `X.Y.Z`
- `X.Y`
- `latest` (stable releases only, not pre-releases)

Examples:

```sh
docker run --rm ghcr.io/caddyglow/pricr:latest btc eth
docker run --rm ghcr.io/caddyglow/pricr:<version> --provider coingecko btc
```

## Configuration File (XDG)

`pricr` reads optional config from:

- `$XDG_CONFIG_HOME/pricr.toml`
- `~/.config/pricr.toml` (fallback when `XDG_CONFIG_HOME` is not set)

You can also pass an explicit file path:

```sh
pricr --config /path/to/pricr.toml btc eth
```

Example:

```toml
[defaults]
currency = "eur"
provider_order = ["coingecko", "yahoo", "stooq", "cmc"]

[coinmarketcap]
api_key = "YOUR_COINMARKETCAP_API_KEY"

[watchlists]
commodities = ["GC=F", "SI=F", "CL=F", "BZ=F", "NG=F"]
metals = ["GC=F", "SI=F"]
```

Precedence:

- `--config <path>` selects which config file to read; otherwise XDG lookup is used.
- CLI flags win over config values.
- For CoinMarketCap API key, `--api-key` / `COINMARKETCAP_API_KEY` are checked first, then `[coinmarketcap].api_key`.
- If no currency is set via `--currency` or config, `usd` is used.

Notes:

- `[defaults].currency` sets the default quote currency for normal price lookup mode (for example `pricr btc eth`).
- `[defaults].provider_order` controls provider priority when `--provider` is omitted. Unknown provider ids return a config error.
- `[watchlists]` lets you define reusable symbol groups and call them as positional arguments with `@name` (for example `pricr @commodities`).
- Conversion mode does not use `[defaults].currency` for the source currency; it uses the first argument (for example `100usd`).

## CLI Overview

`pricr` supports three modes:

1. Price lookup mode: query one or more symbols (crypto or stocks).
2. Conversion mode: provide `<amount><fiat>` as the first argument, then one or more target symbols/currencies.
3. Ticker search mode: search symbols by keyword.

Price lookup mode also supports chart output for historical prices.

### Price Lookup Mode

Examples:

```sh
pricr --provider coingecko btc eth
pricr -p cmc -c eur btc sol
pricr -p yahoo CW8.PA VWCE.DE
pricr -p stooq aapl msft nvda
pricr --provider yahoo @commodities
pricr @commodities
pricr --json -p coingecko btc eth
pricr --chart --interval 1M -p coingecko btc eth
pricr --chart --interval 1Y -p yahoo CW8.PA
pricr --chart --interval 5D --sampling hourly -p cmc btc
pricr --list-providers
```

Notes:

- `cmc` (CoinMarketCap) spot price lookup requires an API key via `--api-key`, `COINMARKETCAP_API_KEY`, or config file.
- `coingecko` works without an API key.
- `yahoo` works without an API key and supports global stock/ETF symbols.
- `stooq` works without an API key and supports stock/ETF symbols (US tickers default to `.US`).
- When `--provider` is omitted, price lookup and conversion mode use provider fallback in `[defaults].provider_order` (then append remaining available providers).
- Use `@watchlist_name` to expand symbols from config before lookup (for example `@commodities`).
- `--list-providers` always includes `coingecko`, `cmc`, `yahoo`, and `stooq`.
- Increase logging with `-v`, `-vv`, or `-vvv` (logs are written to stderr).

### Ticker Search Mode

Use `--search` to find matching ticker symbols before running price lookup.

You can also use shorthand style `pricr search <query>`.

Examples:

```sh
pricr --search apple
pricr search apple
pricr --provider stooq --search apple
pricr --provider stooq --search tesla --search-limit 5
pricr --provider stooq --search nvidia --json
pricr search --provider stooq apple
pricr search --provider yahoo cw8
```

Notes:

- Ticker search support is available on `stooq` and `yahoo`.
- When `--provider` is omitted, ticker search runs across providers in `[defaults].provider_order` and merges duplicate matches by combining provider names.
- `--search-limit` defaults to `10` and supports `1..=50`.

### Chart Mode (Price History)

Use `--chart` to render an ASCII trend chart from historical prices.

Examples:

```sh
pricr --chart btc
pricr --chart --interval 1M --currency eur btc eth
pricr --chart --interval 5D --json btc
pricr --chart --interval 5D --sampling hourly --provider cmc btc
pricr --chart --interval 6M --end-date 2025-12-31 usd eur gbp
pricr --chart --provider yahoo --start-date 2025-01-01 --end-date 2025-12-31 CW8.PA
```

Notes:

- `--interval` controls the chart range preset: `1D`, `5D`, `1M`, `6M`, `YTD`, `1Y`, `5Y`, `ALL` (default `1M`).
- `--sampling` controls point density (`auto`, `hourly`, `daily`; default `auto`).
- `--start-date YYYY-MM-DD` sets an explicit chart window start and overrides `--interval`.
- `--end-date YYYY-MM-DD` sets the chart window end date in UTC (defaults to today).
- Chart mode works in price lookup mode, not conversion mode.
- Chart history is supported by `coingecko`, `cmc`, `yahoo`, and `stooq` providers.
- CMC chart mode uses CoinMarketCap's public web chart endpoint for `USD` and falls back to the Pro API for other quote currencies.
- Yahoo chart mode uses explicit `period1/period2` windows when `--start-date`/`--end-date` are provided.
- Stooq chart mode is daily and does not provide market cap values.
- All providers use shared XDG file cache (`$XDG_CACHE_HOME/pricr` or `~/.cache/pricr`): CoinMarketCap coin catalog TTL is 24h, daily chart TTL is 12h; CoinGecko quote TTL is 30s and chart TTL is 1h (hourly) / 12h (daily); Yahoo quote TTL is 30s, search TTL is 10m, and chart TTL is 1h (hourly) / 12h (daily); Stooq quote TTL is 30s and history TTL is 12h; Frankfurter latest rates TTL is 10m and history TTL is 12h.

### Fiat Chart Mode (Frankfurter)

When `--chart` is enabled and all positional symbols are fiat codes, the first code is treated as the base currency and remaining codes are chart targets.

Examples:

```sh
pricr --chart usd eur
pricr --chart --interval 6M usd eur gbp jpy
pricr --chart --json usd eur
```

Notes:

- Fiat chart mode uses Frankfurter (ECB reference rates).
- Fiat history is daily; `--sampling hourly` is not supported in fiat chart mode.

### Conversion Mode (Fiat to Crypto and Fiat)

When the first positional argument matches `<number><fiat_code>`, conversion mode is enabled.

Input rules:

- Use a single token with no spaces, like `100usd` or `3.5eur`.
- Fiat code must be one of the supported codes listed below.

Examples:

```sh
pricr 100usd btc eth eur jpy
pricr 250eur usd chf
pricr --json -p coingecko 75gbp sol usd
```

How conversion works:

- Fiat to crypto uses the selected crypto provider (`coingecko` or `cmc`).
- Fiat to fiat uses Frankfurter (ECB reference rates).
- You can mix fiat and crypto targets in one command.

## Fiat Support

Conversion mode recognizes these fiat codes:

`USD EUR GBP JPY CNY CAD AUD CHF KRW INR BRL RUB TRY ZAR MXN SGD HKD NOK SEK DKK NZD PLN THB TWD CZK HUF ILS PHP MYR ARS CLP COP IDR SAR AED NGN VND PKR BDT EGP`

## Example Output

Command:

```sh
pricr --provider coingecko btc eth
```

Example table output:

```text
+--------+----------+-----------+------------+------------+-----------+
| Symbol | Name     | Price     | 24h Change | Market Cap | Provider  |
+--------+----------+-----------+------------+------------+-----------+
| BTC    | Bitcoin  | $96,420.1 | +1.42%     | $1.91T     | CoinGecko |
| ETH    | Ethereum | $3,212.77 | -0.38%     | $386.55B   | CoinGecko |
+--------+----------+-----------+------------+------------+-----------+
```

Command:

```sh
pricr 100usd btc eur
```

Example conversion output:

```text
+---------+----+------------+--------------------+-----------------+
| Amount  |    | Result     | Rate               | Provider        |
+---------+----+------------+--------------------+-----------------+
| $100.00 | -> | 0.001037 BTC | 1 BTC = $96,420.10 | CoinGecko       |
| $100.00 | -> | EUR 92.15  | 1 EUR = $1.08      | Frankfurter/ECB |
+---------+----+------------+--------------------+-----------------+
```

### JSON Output Example

Command:

```sh
pricr --json --provider coingecko btc eth
```

Example JSON output:

```json
[
  {
    "symbol": "BTC",
    "name": "Bitcoin",
    "price": 96420.1,
    "change_24h": 1.42,
    "market_cap": 1910000000000.0,
    "currency": "USD",
    "provider": "CoinGecko",
    "timestamp": "2026-02-21T12:34:56Z"
  },
  {
    "symbol": "ETH",
    "name": "Ethereum",
    "price": 3212.77,
    "change_24h": -0.38,
    "market_cap": 386550000000.0,
    "currency": "USD",
    "provider": "CoinGecko",
    "timestamp": "2026-02-21T12:34:56Z"
  }
]
```

Command (conversion mode):

```sh
pricr --json 100usd btc eur
```

Example conversion JSON output:

```json
[
  {
    "from_amount": 100.0,
    "from_currency": "USD",
    "to_symbol": "BTC",
    "to_name": "Bitcoin",
    "to_amount": 0.001037,
    "rate": 96420.1,
    "provider": "CoinGecko",
    "timestamp": "2026-02-21T12:34:56Z"
  },
  {
    "from_amount": 100.0,
    "from_currency": "USD",
    "to_symbol": "EUR",
    "to_name": "Euro",
    "to_amount": 92.15,
    "rate": 1.08497,
    "provider": "Frankfurter/ECB",
    "timestamp": "2026-02-21T12:34:56Z"
  }
]
```

## Development

See `CONTRIBUTING.md` for development workflow and contribution guidelines.

## License

MIT. See `LICENSE`.
