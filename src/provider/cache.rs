use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tracing::debug;

#[derive(Debug, Serialize, serde::Deserialize)]
struct CacheEnvelope<T> {
    fetched_at_unix: i64,
    value: T,
}

pub async fn read_json<T: DeserializeOwned>(provider: &str, key: &str, ttl_secs: i64) -> Option<T> {
    let path = cache_path(provider, key)?;
    let raw = tokio::fs::read_to_string(&path).await.ok()?;
    let envelope: CacheEnvelope<T> = serde_json::from_str(&raw).ok()?;

    let age_secs = chrono::Utc::now().timestamp() - envelope.fetched_at_unix;
    if age_secs < 0 || age_secs > ttl_secs {
        return None;
    }

    Some(envelope.value)
}

pub async fn write_json<T: Serialize>(provider: &str, key: &str, value: &T) {
    let Some(path) = cache_path(provider, key) else {
        return;
    };

    let Some(parent) = path.parent() else {
        return;
    };

    if let Err(err) = tokio::fs::create_dir_all(parent).await {
        debug!(path = %parent.display(), error = %err, "failed to create cache directory");
        return;
    }

    let envelope = CacheEnvelope {
        fetched_at_unix: chrono::Utc::now().timestamp(),
        value,
    };

    let serialized = match serde_json::to_string(&envelope) {
        Ok(v) => v,
        Err(err) => {
            debug!(path = %path.display(), error = %err, "failed to serialize cache payload");
            return;
        }
    };

    if let Err(err) = tokio::fs::write(&path, serialized).await {
        debug!(path = %path.display(), error = %err, "failed to write cache file");
    }
}

fn cache_path(provider: &str, key: &str) -> Option<PathBuf> {
    let root = cache_root()?;
    let provider_dir = sanitize_component(provider);
    let file = format!("{}.json", hash_key(key));
    Some(root.join("cryptoprice").join(provider_dir).join(file))
}

fn cache_root() -> Option<PathBuf> {
    if let Ok(xdg_cache_home) = std::env::var("XDG_CACHE_HOME")
        && !xdg_cache_home.trim().is_empty()
    {
        return Some(PathBuf::from(xdg_cache_home));
    }

    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".cache"))
}

fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hash_key(key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
