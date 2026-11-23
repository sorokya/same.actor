//! Admin endpoint handlers. Every one is behind [`super::authorize`].

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{invalid, AppError};
use crate::extract::{AppJson, AppQuery};
use crate::session::wants_secure;
use crate::state::AppState;
use crate::store::bans::NewBan;
use crate::store::report::{LogQuery, Metric, Window};

#[derive(Debug, Deserialize)]
pub struct SessionRequest {
    pub token: String,
}

/// Exchange the token for a cookie the browser will not hand to JavaScript.
pub async fn session(
    State(state): State<AppState>,
    AppJson(body): AppJson<SessionRequest>,
) -> Result<Response, AppError> {
    if !state.admin.enabled() {
        return Err(AppError::NotFound);
    }

    if !state.admin.token_matches(&body.token) {
        // Deliberately slow-ish and uniformly worded: this is the one endpoint an attacker can
        // hammer, and it must not distinguish "wrong" from "almost right".
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        return Err(AppError::Validation(
            "That token is not correct.".to_owned(),
        ));
    }

    let id = state.admin.open_session();

    Ok((
        [(
            header::SET_COOKIE,
            super::admin_cookie_header(&id, wants_secure(&state.config.public_origin)),
        )],
        Json(serde_json::json!({ "ok": true, "expires_in": super::SESSION_TTL })),
    )
        .into_response())
}

#[derive(Debug, Default, Deserialize)]
pub struct WindowQuery {
    #[serde(default)]
    pub window: Option<WindowArg>,
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Accepts `24h`, `7d`, `30d`, and also `day`/`week`/`month`, because a URL is typed by hand.
#[derive(Debug, Clone, Copy)]
pub struct WindowArg(Window);

impl<'de> Deserialize<'de> for WindowArg {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "24h" | "day" | "1d" => Ok(WindowArg(Window::Day)),
            "7d" | "week" => Ok(WindowArg(Window::Week)),
            "30d" | "month" => Ok(WindowArg(Window::Month)),
            other => Err(serde::de::Error::custom(format!(
                "window must be 24h, 7d or 30d, got {other:?}"
            ))),
        }
    }
}

impl WindowQuery {
    fn window(&self) -> Window {
        self.window.map(|w| w.0).unwrap_or(Window::Day)
    }
}

pub async fn overview(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<WindowQuery>,
) -> Result<Json<crate::store::report::Overview>, AppError> {
    let window = query.window();

    let (counts, p95_ms, top_paths, top_countries) = tokio::try_join!(
        state.store.counts(window),
        state.store.p95_ms(window),
        state.store.top_paths(window, 8),
        state.store.geo(window.days()),
    )?;

    let (log_rows, db_bytes) =
        tokio::try_join!(state.store.log_row_count(), state.store.db_bytes())?;

    Ok(Json(crate::store::report::Overview {
        window: window.as_str().to_owned(),
        counts,
        p95_ms,
        top_paths,
        top_countries,
        log_rows,
        db_bytes,
    }))
}

pub async fn logs(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<LogQuery>,
) -> Result<Json<crate::store::report::LogPage>, AppError> {
    Ok(Json(state.store.logs(query).await?))
}

pub async fn log(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<crate::store::RequestRow>, AppError> {
    state
        .store
        .log_by_id(id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct TimeseriesQuery {
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub window: Option<WindowArg>,
}

#[derive(Debug, Serialize)]
pub struct TimeseriesResponse {
    pub metric: String,
    pub window: String,
    pub points: Vec<crate::store::report::DayValue>,
}

pub async fn timeseries(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, AppError> {
    let metric = match query.metric.as_deref().unwrap_or("requests") {
        "requests" => Metric::Requests,
        "errors" => Metric::Errors,
        "visitors" => Metric::Visitors,
        "searches" => Metric::Searches,
        other => {
            return Err(invalid(format!(
                "metric must be requests, errors, visitors or searches, got {other:?}"
            )))
        }
    };
    let window = query.window.map(|w| w.0).unwrap_or(Window::Week);

    Ok(Json(TimeseriesResponse {
        metric: query.metric.unwrap_or_else(|| "requests".into()),
        window: window.as_str().to_owned(),
        points: state.store.timeseries(metric, window).await?,
    }))
}

#[derive(Debug, Serialize)]
pub struct GeoResponse {
    pub days: i64,
    pub countries: Vec<crate::store::report::CountryRow>,
}

pub async fn geo(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<WindowQuery>,
) -> Result<Json<GeoResponse>, AppError> {
    let days = query.days.unwrap_or(7).clamp(1, 400);
    Ok(Json(GeoResponse {
        days,
        countries: state.store.geo(days).await?,
    }))
}

#[derive(Debug, Serialize)]
pub struct SearchReport {
    pub window: String,
    pub top: Vec<crate::store::searches::TrendingTerm>,
    /// What people searched for and did not find. The most useful list on the page.
    pub zero_results: Vec<crate::store::searches::TrendingTerm>,
}

pub async fn searches(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<WindowQuery>,
) -> Result<Json<SearchReport>, AppError> {
    let window = query.window();
    let limit = query.limit.unwrap_or(25);

    let (top, zero_results) = tokio::try_join!(
        state.store.trending_terms(window, limit),
        state.store.zero_result_searches(window.days(), limit),
    )?;

    Ok(Json(SearchReport {
        window: window.as_str().to_owned(),
        top,
        zero_results,
    }))
}

pub async fn comparisons(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<WindowQuery>,
) -> Result<Json<Vec<crate::store::report::ComparisonRow>>, AppError> {
    Ok(Json(
        state.store.comparisons(query.limit.unwrap_or(25)).await?,
    ))
}

pub async fn suspicious(
    State(state): State<AppState>,
    AppQuery(query): AppQuery<WindowQuery>,
) -> Result<Json<Vec<crate::store::report::Suspicion>>, AppError> {
    Ok(Json(
        state
            .store
            .suspicious(query.window(), query.limit.unwrap_or(50))
            .await?,
    ))
}

pub async fn bans(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::store::bans::Ban>>, AppError> {
    Ok(Json(state.store.bans().await?))
}

#[derive(Debug, Deserialize)]
pub struct NewBanRequest {
    pub target: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// Seconds from now. Omitted means permanent.
    #[serde(default)]
    pub expires_in_secs: Option<i64>,
}

/// Create a ban.
///
/// The target is parsed before it is stored. A target that parses as neither an address nor a
/// range would be a ban that matches nothing, and storing one is worse than refusing it: the
/// operator would see it in the list and believe the client was handled.
pub async fn create_ban(
    State(state): State<AppState>,
    AppJson(body): AppJson<NewBanRequest>,
) -> Result<Response, AppError> {
    let target = body.target.trim().to_owned();

    let kind = if target.parse::<std::net::IpAddr>().is_ok() {
        "ip"
    } else if target.parse::<ipnet::IpNet>().is_ok() {
        "cidr"
    } else {
        return Err(invalid(
            "A ban target must be an address (203.0.113.7) or a range (203.0.113.0/24).".to_owned(),
        ));
    };

    let expires_ts = match body.expires_in_secs {
        Some(seconds) if seconds > 0 => Some(crate::clock::now_unix() + seconds),
        Some(_) => return Err(invalid("expires_in_secs must be positive.".to_owned())),
        None => None,
    };

    let id = crate::bans::apply(
        &state.store,
        &state.bans,
        NewBan {
            target,
            kind: kind.into(),
            reason: body
                .reason
                .map(|reason| reason.trim().to_owned())
                .filter(|reason| !reason.is_empty()),
            created_by: "admin".into(),
            expires_ts,
        },
    )
    .await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response())
}

pub async fn delete_ban(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !state.store.delete_ban(id).await? {
        return Err(AppError::NotFound);
    }

    state.bans.reload(&state.store).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn health(
    State(state): State<AppState>,
) -> Result<Json<crate::store::report::Health>, AppError> {
    Ok(Json(
        state
            .store
            .health(state.bans.len(), state.geo.age_secs(), state.geo.enabled())
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_parsed_from_the_url_forms_people_type() {
        for value in ["24h", "day", "1d"] {
            let window = serde_json::from_str::<WindowArg>(&format!("{value:?}")).expect(value);
            assert_eq!(window.0, Window::Day);
        }
        for value in ["7d", "week"] {
            let window = serde_json::from_str::<WindowArg>(&format!("{value:?}")).expect(value);
            assert_eq!(window.0, Window::Week);
        }
        assert_eq!(
            serde_json::from_str::<WindowArg>("\"30d\"").unwrap().0,
            Window::Month
        );
        assert!(
            serde_json::from_str::<WindowArg>("\"forever\"").is_err(),
            "an unknown window must be an error, not a silent default"
        );
    }
}
