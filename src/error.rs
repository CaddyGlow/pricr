use thiserror::Error;

/// Unified error type for the pricr application.
#[derive(Error, Debug)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: {0}")]
    Api(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("No results returned")]
    NoResults,
}

pub type Result<T> = std::result::Result<T, Error>;
