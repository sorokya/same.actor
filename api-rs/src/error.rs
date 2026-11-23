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

    /// The session cookie is required on this endpoint and was absent. Carries a `Set-Cookie`
    /// from the middleware, so the caller can retry and succeed.
    #[error("session required")]
    SessionRequired,

    /// The client's address is banned. The reason is admin-authored, so it is safe to show.
    #[error("banned: {0}")]
    Banned(String),

    /// The database refused a read, a write, or a migration. Distinct from `Internal` so a
    /// storage fault is greppable in the logs instead of hiding among handler bugs.
    #[error("storage error: {0}")]
    Store(String),
}

/// The error code and message attached to a failed response.
///
/// The logging middleware only ever sees a `Response`, so the error is put on the response
/// rather than re-parsed out of its body. Server errors carry the internal detail — that is
/// what makes an admin log row worth reading — while the body keeps the generic message the
/// client is allowed to see.
#[derive(Debug, Clone)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
}

/// Stable, documented error codes. The frontend switches on these, not on prose.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: BodyDetail,
}

#[derive(Debug, Serialize)]
struct BodyDetail {
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
            AppError::SessionRequired => (
                StatusCode::PRECONDITION_REQUIRED,
                "session_required",
                "This endpoint needs a session cookie. Retry with the cookie from this response."
                    .into(),
            ),
            AppError::Banned(reason) => (
                StatusCode::FORBIDDEN,
                "banned",
                if reason.is_empty() {
                    "This client is blocked.".into()
                } else {
                    format!("This client is blocked: {reason}")
                },
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
            AppError::Validation(_)
            | AppError::NotFound
            | AppError::RateLimited { .. }
            | AppError::SessionRequired
            | AppError::Banned(_) => tracing::debug!(%code, "client error"),
        }

        let mut response = (
            status,
            Json(ErrorBody {
                error: BodyDetail {
                    code,
                    message: message.clone(),
                },
            }),
        )
            .into_response();

        // The body is safe to show; the log row is for the operator, so a server error records
        // the detail that explains it. Upstream detail is never in the body, but it is exactly
        // what an admin needs when a request 502s.
        let logged = if status.is_server_error() {
            self.to_string()
        } else {
            message.clone()
        };
        response.extensions_mut().insert(ErrorDetail {
            code,
            message: logged,
        });

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
