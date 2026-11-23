//! Extractors that route rejections through [`AppError`].
//!
//! axum's built-in `Json` and `Query` rejections produce plain-text bodies, which would mean
//! two error shapes on the wire. These wrappers keep the contract uniform: everything that
//! can go wrong comes back as `{"error":{"code","message"}}` (review H5).

use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use axum::Json;
use serde::de::DeserializeOwned;

use crate::error::AppError;

pub struct AppJson<T>(pub T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(|rejection| {
                // The detailed reason names serde internals ("unknown variant … at line 1
                // column 61"). Useful in the log, useless to a visitor.
                tracing::debug!(reason = %rejection.body_text(), "rejected request body");
                AppError::Validation("That request was not understood.".into())
            })?;
        Ok(AppJson(value))
    }
}

pub struct AppQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for AppQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) =
            Query::<T>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| {
                    tracing::debug!(reason = %rejection.body_text(), "rejected query string");
                    AppError::Validation("That request was not understood.".into())
                })?;
        Ok(AppQuery(value))
    }
}
