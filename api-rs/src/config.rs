//! Configuration, parsed once at startup, failing fast.

use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use crate::error::AppError;

/// How the TMDB credential is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdbAuth {
    /// `Authorization: Bearer <token>` — a TMDB **v4 read access token**.
    Bearer,
    /// `?api_key=<key>` — a legacy **v3 API key**.
    Query,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub tmdb_token: String,
    pub tmdb_auth: TmdbAuth,
    pub tmdb_base: String,
    pub image_base: String,
    pub static_dir: String,
    pub cache_ttl: Duration,
    pub cache_capacity: u64,
    pub rate_limit: u32,
    pub rate_window: Duration,
    /// Honour `X-Forwarded-For` for the client IP. Only enable when the app is actually
    /// behind a proxy you control — otherwise the header is trivially spoofed and the rate
    /// limiter becomes useless.
    pub trust_forwarded_for: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        let tmdb_token = env::var("TMDB_TOKEN")
            .or_else(|_| env::var("TMDB_KEY"))
            .map(|s| s.trim().to_owned())
            .map_err(|_| AppError::Config("TMDB_TOKEN (or TMDB_KEY) must be set".into()))?;

        if tmdb_token.is_empty() {
            return Err(AppError::Config("TMDB_TOKEN is empty".into()));
        }

        // Explicit, never sniffed. A v4 read token is long and JWT-shaped; a v3 key is 32
        // hex chars. Guessing here is how you get a production 401 nobody can explain.
        let tmdb_auth = match env::var("TMDB_AUTH").as_deref() {
            Ok("query") => TmdbAuth::Query,
            Ok("bearer") | Err(_) => TmdbAuth::Bearer,
            Ok(other) => {
                return Err(AppError::Config(format!(
                    "TMDB_AUTH must be 'bearer' or 'query', got {other:?}"
                )))
            }
        };

        let bind = match env::var("BIND") {
            Ok(v) => v.parse().map_err(|e| {
                AppError::Config(format!("BIND {v:?} is not a socket address: {e}"))
            })?,
            Err(_) => "0.0.0.0:3001"
                .parse()
                .expect("literal socket address is valid"),
        };

        Ok(Self {
            bind,
            tmdb_token,
            tmdb_auth,
            tmdb_base: env_str("TMDB_BASE", "https://api.themoviedb.org/3"),
            image_base: env_str("IMAGE_BASE", "https://image.tmdb.org/t/p/w342"),
            static_dir: env_str("STATIC_DIR", "dist"),
            cache_ttl: env_secs("CACHE_TTL_SECS", 600),
            cache_capacity: env_num("CACHE_CAPACITY", 10_000),
            rate_limit: env_num("RATE_LIMIT", 10) as u32,
            rate_window: env_secs("RATE_WINDOW_SECS", 1),
            trust_forwarded_for: env_bool("TRUST_FORWARDED_FOR", false),
        })
    }
}

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn env_num(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(env_num(key, default))
}

fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key).as_deref().map(str::trim) {
        Ok("1" | "true" | "yes") => true,
        Ok("0" | "false" | "no") => false,
        _ => default,
    }
}
