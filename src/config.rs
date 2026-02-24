use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Default fiat currency for price display.
pub const DEFAULT_CURRENCY: &str = "usd";

/// File name used in the XDG config directory.
pub const CONFIG_FILE_NAME: &str = "pricr.toml";

/// Application configuration loaded from `$XDG_CONFIG_HOME/pricr.toml`
/// or `~/.config/pricr.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub defaults: DefaultsConfig,
    pub coinmarketcap: CoinMarketCapConfig,
    pub watchlists: HashMap<String, Vec<String>>,
}

/// General defaults used when CLI flags are not provided.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub currency: Option<String>,
    pub provider_order: Option<Vec<String>>,
}

/// CoinMarketCap provider-specific configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CoinMarketCapConfig {
    pub api_key: Option<String>,
}

/// Resolve the configuration file path based on XDG conventions.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME")
        && !xdg_config_home.trim().is_empty()
    {
        return Some(PathBuf::from(xdg_config_home).join(CONFIG_FILE_NAME));
    }

    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join(CONFIG_FILE_NAME))
}

/// Load config from disk. Returns defaults when the file does not exist.
pub fn load() -> Result<AppConfig> {
    let Some(path) = config_path() else {
        return Ok(AppConfig::default());
    };

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(err) => {
            return Err(read_config_error(&path, err));
        }
    };

    parse(&raw).map_err(|err| parse_config_error(&path, err))
}

/// Load config from an explicit path.
///
/// Unlike [`load`], this returns an error when the file is missing.
pub fn load_from_path(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path).map_err(|err| read_config_error(path, err))?;
    parse(&raw).map_err(|err| parse_config_error(path, err))
}

fn parse(raw: &str) -> std::result::Result<AppConfig, toml::de::Error> {
    toml::from_str(raw)
}

fn read_config_error(path: &Path, err: std::io::Error) -> Error {
    Error::Config(format!(
        "failed to read config file '{}': {}",
        path.display(),
        err
    ))
}

fn parse_config_error(path: &Path, err: toml::de::Error) -> Error {
    Error::Config(format!(
        "failed to parse config file '{}': {}",
        path.display(),
        err
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_config_uses_defaults() {
        let cfg = parse("").unwrap();
        assert!(cfg.defaults.currency.is_none());
        assert!(cfg.defaults.provider_order.is_none());
        assert!(cfg.coinmarketcap.api_key.is_none());
        assert!(cfg.watchlists.is_empty());
    }

    #[test]
    fn parse_coinmarketcap_api_key() {
        let cfg = parse(
            r#"
            [coinmarketcap]
            api_key = "abc123"
            "#,
        )
        .unwrap();

        assert_eq!(cfg.coinmarketcap.api_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_default_currency() {
        let cfg = parse(
            r#"
            [defaults]
            currency = "eur"
            "#,
        )
        .unwrap();

        assert_eq!(cfg.defaults.currency.as_deref(), Some("eur"));
        assert!(cfg.defaults.provider_order.is_none());
    }

    #[test]
    fn parse_provider_order() {
        let cfg = parse(
            r#"
            [defaults]
            provider_order = ["yahoo", "coingecko", "stooq"]
            "#,
        )
        .unwrap();

        assert_eq!(
            cfg.defaults.provider_order,
            Some(vec![
                "yahoo".to_string(),
                "coingecko".to_string(),
                "stooq".to_string()
            ])
        );
    }

    #[test]
    fn parse_watchlists() {
        let cfg = parse(
            r#"
            [watchlists]
            commodities = ["GC=F", "SI=F", "CL=F"]
            metals = ["GC=F", "SI=F"]
            "#,
        )
        .unwrap();

        assert_eq!(
            cfg.watchlists.get("commodities"),
            Some(&vec![
                "GC=F".to_string(),
                "SI=F".to_string(),
                "CL=F".to_string()
            ])
        );
        assert_eq!(
            cfg.watchlists.get("metals"),
            Some(&vec!["GC=F".to_string(), "SI=F".to_string()])
        );
    }
}
