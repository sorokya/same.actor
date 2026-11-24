//! Shared application state.

use std::sync::Arc;

use moka::future::Cache;

use crate::admin::AdminAuth;
use crate::bans::{AutoBan, BanCache};
use crate::config::Config;
use crate::error::AppError;
use crate::geo::Geo;
use crate::pageviews::ViewDeduper;
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
    /// Hashed filenames of the built CSS and island, read once from the Vite manifest.
    pub assets: crate::assets::Assets,
    /// The request log and everything derived from it.
    pub store: Arc<Store>,
    /// Country lookups. Held here as well as inside the writer so the refresher and the health
    /// panel can reach it.
    pub geo: Arc<Geo>,
    /// Live ban list. Read on every request, so it is memory, not a query.
    pub bans: Arc<BanCache>,
    /// Automatic-ban rules. Disabled unless the thresholds are configured.
    pub autoban_rate: Arc<AutoBan>,
    pub autoban_errors: Arc<AutoBan>,
    /// Admin credentials. Disabled when `ADMIN_TOKEN` is unset.
    pub admin: Arc<AdminAuth>,
    /// Decides whether a page view is a new one.
    pub views: Arc<ViewDeduper>,
    /// Public trending terms. Cached for a minute so the strip costs one query a minute rather
    /// than one per page view.
    pub trending_cache: Cache<u8, Arc<Vec<crate::store::searches::TrendingTerm>>>,
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

        // Hashed filenames for the stylesheet and the island, read once. A missing manifest is
        // a warning, not a startup failure: an API-only deployment has no assets and must run.
        let assets = crate::assets::Assets::load(std::path::Path::new(&config.static_dir));

        let admin = Arc::new(AdminAuth::new(config.admin_token.clone()));
        if !admin.enabled() {
            tracing::info!("ADMIN_TOKEN is not set; the admin API is disabled");
        }

        let views = Arc::new(ViewDeduper::new());
        let bans = Arc::new(BanCache::new());
        let autoban_rate = Arc::new(AutoBan::new(config.autoban_requests, config.autoban_window));
        let autoban_errors = Arc::new(AutoBan::new(config.autoban_errors, config.autoban_window));

        Ok(Self {
            config: Arc::new(config),
            tmdb: Arc::new(tmdb),
            limiter: Arc::new(limiter),
            og_cache: Cache::builder()
                .max_capacity(512)
                .time_to_live(std::time::Duration::from_secs(3600))
                .build(),
            assets,
            store: Arc::new(store),
            geo,
            bans,
            autoban_rate,
            autoban_errors,
            admin,
            views,
            trending_cache: Cache::builder()
                .max_capacity(4)
                .time_to_live(std::time::Duration::from_secs(60))
                .build(),
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
            tmdb_region: "US".into(),
            admin_token: None,
            autoban_requests: 0,
            autoban_errors: 0,
            autoban_window: std::time::Duration::from_secs(60),
            autoban_ban_secs: 3600,
            trust_forwarded_for: false,
        }
    }

    /// An API-only deployment — no built assets — must still start. The pages then render
    /// without a stylesheet, which is degraded but not a failure to boot.
    #[test]
    fn a_missing_asset_manifest_is_not_fatal() {
        let dir = std::env::temp_dir().join(format!("same-actor-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let state = AppState::new(config(&dir)).expect("an API-only deployment must still start");
        assert!(state.assets.css.is_none());
    }
}
