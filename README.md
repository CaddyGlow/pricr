# cryptoprice

`cryptoprice` is a Rust CLI for fast crypto price lookup and fiat conversion from the terminal.

Requirements: Rust 1.85+ (edition 2024).

## Install

Install with Cargo from this repository:

```sh
cargo install --locked --git https://github.com/CaddyGlow/cryptoprice cryptoprice
```

Tip: pin installs with `--tag <version>` or `--rev <commit>` when you need reproducible CI/dev environments.

Or build from source:

```sh
cargo build --release
```

## Install with Nix

Build from this repository:

```sh
nix build .#cryptoprice
```

Run directly:

```sh
nix run .#cryptoprice -- btc eth
```

## Install on NixOS

### From this repository as a flake input

Add `cryptoprice` as an input in your system flake and include it in
`environment.systemPackages`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    cryptoprice.url = "github:CaddyGlow/cryptoprice";
  };

  outputs = { nixpkgs, cryptoprice, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            cryptoprice.packages.${pkgs.system}.cryptoprice
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

### After `cryptoprice` is in nixpkgs

Install directly from `pkgs`:

```nix
{ pkgs, ... }:
{
  environment.systemPackages = with pkgs; [
    cryptoprice
  ];
}
```

## Docker Image

Release tags publish a multi-arch image to GHCR:

- `ghcr.io/caddyglow/cryptoprice`
- Platforms: `linux/amd64`, `linux/arm64`

Published tags:

- `vX.Y.Z` (git tag name)
- `X.Y.Z`
- `X.Y`
- `latest` (stable releases only, not pre-releases)

Examples:

```sh
docker run --rm ghcr.io/caddyglow/cryptoprice:latest btc eth
docker run --rm ghcr.io/caddyglow/cryptoprice:<version> --provider coingecko btc
```

## Configuration File (XDG)

`cryptoprice` reads optional config from:

- `$XDG_CONFIG_HOME/cryptoprice.toml`
- `~/.config/cryptoprice.toml` (fallback when `XDG_CONFIG_HOME` is not set)

You can also pass an explicit file path:

```sh
cryptoprice --config /path/to/cryptoprice.toml btc eth
```

Example:

```toml
[defaults]
currency = "eur"

[coinmarketcap]
api_key = "YOUR_COINMARKETCAP_API_KEY"
```

Precedence:

- `--config <path>` selects which config file to read; otherwise XDG lookup is used.
- CLI flags win over config values.
- For CoinMarketCap API key, `--api-key` / `COINMARKETCAP_API_KEY` are checked first, then `[coinmarketcap].api_key`.
- If no currency is set via `--currency` or config, `usd` is used.

Notes:

- `[defaults].currency` sets the default quote currency for normal price lookup mode (for example `cryptoprice btc eth`).
- Conversion mode does not use `[defaults].currency` for the source currency; it uses the first argument (for example `100usd`).

## CLI Overview

`cryptoprice` supports two modes:

1. Price lookup mode: query one or more crypto symbols.
2. Conversion mode: provide `<amount><fiat>` as the first argument, then one or more target symbols/currencies.

Price lookup mode also supports chart output for historical prices.

### Price Lookup Mode

Examples:

```sh
cryptoprice --provider coingecko btc eth
cryptoprice -p cmc -c eur btc sol
cryptoprice --json -p coingecko btc eth
cryptoprice --chart --days 30 -p coingecko btc eth
cryptoprice --chart --days 14 --interval hourly -p cmc btc
cryptoprice --list-providers
```

Notes:

- `cmc` (CoinMarketCap) spot price lookup requires an API key via `--api-key`, `COINMARKETCAP_API_KEY`, or config file.
- `coingecko` works without an API key.
- `--list-providers` always includes both `coingecko` and `cmc`.
- Increase logging with `-v`, `-vv`, or `-vvv` (logs are written to stderr).

### Chart Mode (Price History)

Use `--chart` to render an ASCII trend chart from historical prices.

Examples:

```sh
cryptoprice --chart btc
cryptoprice --chart --days 30 --currency eur btc eth
cryptoprice --chart --json --days 14 btc
cryptoprice --chart --days 2 --interval hourly --provider cmc btc
cryptoprice --chart --days 30 usd eur gbp
```

Notes:

- `--days` controls the history window (`1..=365`, default `7`).
- `--interval` controls sampling (`auto`, `hourly`, `daily`; default `auto`).
- Chart mode works in price lookup mode, not conversion mode.
- Chart history is supported by both `coingecko` and `cmc` providers.
- CMC chart mode uses CoinMarketCap's public web chart endpoint for `USD` and falls back to the Pro API for other quote currencies.
- All providers use shared XDG file cache (`$XDG_CACHE_HOME/cryptoprice` or `~/.cache/cryptoprice`): CoinMarketCap coin catalog TTL is 24h, daily chart TTL is 12h; CoinGecko quote TTL is 30s and chart TTL is 1h (hourly) / 12h (daily); Frankfurter latest rates TTL is 10m and history TTL is 12h.

### Fiat Chart Mode (Frankfurter)

When `--chart` is enabled and all positional symbols are fiat codes, the first code is treated as the base currency and remaining codes are chart targets.

Examples:

```sh
cryptoprice --chart usd eur
cryptoprice --chart --days 90 usd eur gbp jpy
cryptoprice --chart --json usd eur
```

Notes:

- Fiat chart mode uses Frankfurter (ECB reference rates).
- Fiat history is daily; `--interval hourly` is not supported in fiat chart mode.

### Conversion Mode (Fiat to Crypto and Fiat)

When the first positional argument matches `<number><fiat_code>`, conversion mode is enabled.

Input rules:

- Use a single token with no spaces, like `100usd` or `3.5eur`.
- Fiat code must be one of the supported codes listed below.

Examples:

```sh
cryptoprice 100usd btc eth eur jpy
cryptoprice 250eur usd chf
cryptoprice --json -p coingecko 75gbp sol usd
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
cryptoprice --provider coingecko btc eth
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
cryptoprice 100usd btc eur
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
cryptoprice --json --provider coingecko btc eth
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
cryptoprice --json 100usd btc eur
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
