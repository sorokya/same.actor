//! One error type for the whole service, and one place that maps it to a response.
//!
//! Every handler returns `Result<_, AppError>`, so there is no path where a failed upstream
//! call becomes an unhandled rejection that takes the process down — which is exactly what
//! the old Express API did (review C2).

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The request was well-formed HTTP but the values are not acceptable.
    #[error("{0}")]
    Validation(String),

    /// Configuration is wrong. Only ever produced at startup.
    #[error("configuration error: {0}")]
    Config(String),

    /// TMDB said no, or did not answer. The detail is logged, never returned: it can contain
    /// upstream URLs and anything TMDB echoed back.
    #[error("upstream request failed: {0}")]
    Upstream(String),

    /// TMDB does not know this entity.
    #[error("not found")]
    NotFound,

    /// Our own rate limit, or TMDB's, tripped.
    #[error("rate limited")]
    RateLimited { retry_after_secs: u64 },

    #[error("internal error: {0}")]
    Internal(String),

    /// The database refused a read, a write, or a migration. Distinct from `Internal` so a
    /// storage fault is greppable in the logs instead of hiding among handler bugs.
    #[error("storage error: {0}")]
    Store(String),
}

/// Stable, documented error codes. The frontend switches on these, not on prose.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::Validation(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                msg.clone(),
            ),
            AppError::Config(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "config_error",
                "The service is misconfigured.".into(),
            ),
            AppError::Upstream(_) => (
                StatusCode::BAD_GATEWAY,
                "upstream_unavailable",
                "The movie database is not answering right now.".into(),
            ),
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "No such title or person.".into(),
            ),
            AppError::RateLimited { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many requests. Give it a second.".into(),
            ),
            AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Something went wrong on our side.".into(),
            ),
            AppError::Store(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "Something went wrong on our side.".into(),
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();

        // Anything that is not the client's fault gets logged in full. Client errors are
        // expected traffic and would only be noise.
        match &self {
            AppError::Upstream(detail) => tracing::warn!(error = %detail, "upstream failure"),
            AppError::Internal(detail) => tracing::error!(error = %detail, "internal failure"),
            AppError::Config(detail) => tracing::error!(error = %detail, "configuration failure"),
            AppError::Store(detail) => tracing::error!(error = %detail, "storage failure"),
            AppError::Validation(_) | AppError::NotFound | AppError::RateLimited { .. } => {
                tracing::debug!(%code, "client error")
            }
        }

        let mut response = (
            status,
            Json(ErrorBody {
                error: ErrorDetail { code, message },
            }),
        )
            .into_response();

        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );

        if let AppError::RateLimited { retry_after_secs } = &self {
            if let Ok(value) = header::HeaderValue::from_str(&retry_after_secs.to_string()) {
                headers.insert(header::RETRY_AFTER, value);
            }
        }

        response
    }
}

/// Convenience for the many places that just want a 422 with a fixed message.
pub fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_detail_never_reaches_the_client() {
        let err =
            AppError::Upstream("https://api.themoviedb.org/3/movie/1/credits returned 500".into());
        let (status, code, message) = err.parts();
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(code, "upstream_unavailable");
        assert!(
            !message.contains("themoviedb"),
            "upstream URL leaked: {message}"
        );
        assert!(
            !message.contains("500"),
            "upstream detail leaked: {message}"
        );
    }

    #[test]
    fn validation_messages_are_safe_to_echo() {
        let err = invalid("Selection must contain between 2 and 5 entries.");
        let (status, code, message) = err.parts();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(code, "validation_error");
        assert!(message.contains("2 and 5"));
    }
}
