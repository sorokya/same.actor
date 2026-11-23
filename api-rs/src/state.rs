//! Shared application state.

use std::sync::Arc;

use moka::future::Cache;

use crate::config::Config;
use crate::error::AppError;
use crate::ratelimit::RateLimiter;
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
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let tmdb = Tmdb::new(&config)?;
        let limiter = RateLimiter::new(config.rate_limit, config.rate_window);

        let index_html =
            std::fs::read_to_string(std::path::Path::new(&config.static_dir).join("index.html"))
                .map(Arc::new)
                .ok();

        Ok(Self {
            config: Arc::new(config),
            tmdb: Arc::new(tmdb),
            limiter: Arc::new(limiter),
            og_cache: Cache::builder()
                .max_capacity(512)
                .time_to_live(std::time::Duration::from_secs(3600))
                .build(),
            index_html,
        })
    }
}
