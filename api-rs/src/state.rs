//! Shared application state.

use std::sync::Arc;

use moka::future::Cache;

use crate::config::Config;
use crate::error::AppError;
use crate::geo::Geo;
use crate::ratelimit::RateLimiter;
use crate::store::{Store, StoreConfig};
use crate::tmdb::Tmdb;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub tmdb: Arc<Tmdb>,
    pub limiter: Arc<RateLimiter>,
    /// Rendered share cards, keyed by canonical comparison path. Rasterising is the most
    /// expensive thing this service does, and a crawler asks for the same card repeatedly.
    pub og_cache: Cache<String, Arc<Vec<u8>>>,
    /// `dist/index.html` held in memory so Open Graph tags can be injected per comparison.
    /// `None` when the deployment serves no SPA.
    pub index_html: Option<Arc<String>>,
    /// The request log and everything derived from it.
    pub store: Arc<Store>,
    /// Country lookups. Held here as well as inside the writer so the refresher and the health
    /// panel can reach it.
    pub geo: Arc<Geo>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let tmdb = Tmdb::new(&config)?;
        let limiter = RateLimiter::new(config.rate_limit, config.rate_window);

        // Opened here rather than in `main` so every caller — including tests — gets a working
        // store with no extra ceremony. A failing migration is a startup failure: serving with
        // no analytics is fine, serving with a *silently broken* one is not.
        let geo = Geo::new(
            std::path::PathBuf::from(&config.geo_db_path),
            config.geo_url.clone(),
            config.geo_enabled,
        );

        let store = Store::open(StoreConfig {
            path: std::path::PathBuf::from(&config.db_path),
            geo: Some(Arc::clone(&geo)),
            ..StoreConfig::default()
        })?;

        // A truncated or empty shell would render blank pages with a 200; treat it as absent
        // so the failure mode is a visible 404 rather than a mystery.
        let index_html =
            std::fs::read_to_string(std::path::Path::new(&config.static_dir).join("index.html"))
                .ok()
                .filter(|html| html.trim().len() > 32)
                .map(Arc::new);

        Ok(Self {
            config: Arc::new(config),
            tmdb: Arc::new(tmdb),
            limiter: Arc::new(limiter),
            og_cache: Cache::builder()
                .max_capacity(512)
                .time_to_live(std::time::Duration::from_secs(3600))
                .build(),
            index_html,
            store: Arc::new(store),
            geo,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(static_dir: &std::path::Path) -> Config {
        Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            tmdb_token: "test-token".into(),
            tmdb_auth: crate::config::TmdbAuth::Bearer,
            tmdb_base: "http://127.0.0.1:1/3".into(),
            image_base: "http://127.0.0.1:1/t/p/w342".into(),
            static_dir: static_dir.to_string_lossy().into_owned(),
            db_path: static_dir.join("test.db").to_string_lossy().into_owned(),
            geo_enabled: false,
            geo_db_path: static_dir.join("geo.mmdb").to_string_lossy().into_owned(),
            geo_url: "https://example.invalid/{month}.mmdb.gz".into(),
            log_retention_days: 90,
            log_assets: false,
            public_origin: "http://localhost:3001".into(),
            tmdb_logo_url: "http://127.0.0.1:1/logo.svg".into(),
            cache_ttl: std::time::Duration::from_secs(60),
            cache_capacity: 10,
            rate_limit: 10,
            rate_window: std::time::Duration::from_secs(1),
            trust_forwarded_for: false,
        }
    }

    fn dir_with(contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "same-actor-state-{}-{}",
            std::process::id(),
            contents.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), contents).unwrap();
        dir
    }

    #[test]
    fn a_real_shell_is_loaded() {
        let dir = dir_with("<!doctype html><html><head><!--og:start--><!--og:end--></head></html>");
        let state = AppState::new(config(&dir)).expect("state");
        assert!(state.index_html.is_some());
    }

    /// A truncated shell — the file being rewritten underneath us, or a bad image — must not
    /// become a blank page served with a 200.
    #[test]
    fn an_empty_shell_is_treated_as_absent() {
        let state = AppState::new(config(&dir_with(""))).expect("state");
        assert!(state.index_html.is_none());
    }

    #[test]
    fn a_missing_shell_is_absent_not_fatal() {
        let dir = std::env::temp_dir().join("same-actor-state-does-not-exist");
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::new(config(&dir)).expect("an API-only deployment must still start");
        assert!(state.index_html.is_none());
    }
}
