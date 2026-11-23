//! Shared application state.

use std::sync::Arc;

use crate::config::Config;
use crate::error::AppError;
use crate::ratelimit::RateLimiter;
use crate::tmdb::Tmdb;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub tmdb: Arc<Tmdb>,
    pub limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let tmdb = Tmdb::new(&config)?;
        let limiter = RateLimiter::new(config.rate_limit, config.rate_window);

        Ok(Self {
            config: Arc::new(config),
            tmdb: Arc::new(tmdb),
            limiter: Arc::new(limiter),
        })
    }
}
