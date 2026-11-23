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
    /// Absolute origin used for `og:url` and `og:image`. Open Graph requires absolute URLs,
    /// so this cannot be inferred from the request behind a proxy.
    pub public_origin: String,
    /// TMDB's own logo, fetched unmodified for the share card. Approved asset from
    /// <https://www.themoviedb.org/about/logos-attribution>.
    pub tmdb_logo_url: String,
    /// SQLite file. The container points this at a volume; the default is relative so a
    /// developer needs no configuration at all.
    pub db_path: String,
    /// Days of raw request rows kept before they are rolled into daily aggregates. The
    /// aggregates keep no IP and no session id, so this is the privacy cutover as well as the
    /// size one.
    pub log_retention_days: u32,
    /// Log static assets. Off by default: one page load is one row, not five.
    pub log_assets: bool,
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
            public_origin: env_str("PUBLIC_ORIGIN", "http://localhost:3001"),
            tmdb_logo_url: env_str(
                "TMDB_LOGO_URL",
                "https://www.themoviedb.org/assets/v4/logos/v2/blue_short-8e7b30f73a4020692ccca9c88bafe5dcb6f8a62a4c6bc55cd9ba82bb2cd95f6c.svg",
            ),
            db_path: env_str("DB_PATH", crate::store::DEFAULT_PATH),
            log_retention_days: env_num("LOG_RETENTION_DAYS", 90).clamp(1, 3650) as u32,
            log_assets: env_bool("LOG_ASSETS", false),
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
