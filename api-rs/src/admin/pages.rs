//! The dashboard as pages.
//!
//! Same store calls as the JSON admin API, delivered as HTML: tables, SVG charts, and filters
//! that are links. Every page is behind the admin credential; the login form is the only
//! unguarded route, because presenting the token is what it does.
//!
//! Nothing here is cached — this is live operational data, and a proxy holding yesterday's log
//! table would be actively misleading.

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{middleware, Router};
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;
use crate::store::report::{LogQuery, Metric, Window};
use crate::views::admin::{
    comparison_rows, health_page, overview_page, suspicion_page, term_rows, BansPage, LogPage2,
    LoginPage, LogsPage, OverviewData,
};
use crate::views::{render, Head};

/// The login form and the exchange it posts to. Reachable without a credential.
pub fn open_router() -> Router<AppState> {
    Router::new()
        .route("/", get(login))
        .route("/session", post(session))
}

/// Everything else, behind the cookie.
pub fn page_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/logout", post(logout))
        .route("/overview", get(overview))
        .route("/logs", get(logs))
        .route("/logs/{id}", get(log))
        .route("/suspicious", get(suspicious))
        .route("/bans", get(bans).post(create_ban))
        .route("/bans/{id}/delete", post(delete_ban))
        .route("/health", get(health))
        .layer(middleware::from_fn_with_state(state, guard))
        .layer(middleware::from_fn(crate::admin::no_store))
}

/// A page request without a credential is a redirect to the form, not a 401 body: a browser
/// asking for a page wants a page.
async fn guard(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    if !state.admin.enabled() {
        // The same answer as a route that does not exist, because when the token is unset it
        // does not.
        return AppError::NotFound.into_response();
    }

    if crate::admin::cookie_session(request.headers())
        .is_some_and(|id| state.admin.session_valid(&id))
    {
        return next.run(request).await;
    }

    Redirect::to("/admin").into_response()
}

fn head(title: &str, description: &str, css: &Option<String>, active: &'static str) -> Head {
    let mut head = Head::new(title, description, "admin")
        .noindex()
        .with_active(active);
    if let Some(css) = css.as_deref() {
        head = head.with_css(css);
    }
    head
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if !state.admin.enabled() {
        return Err(AppError::NotFound);
    }

    if crate::admin::cookie_session(&headers).is_some_and(|id| state.admin.session_valid(&id)) {
        return Ok(Redirect::to("/admin/overview").into_response());
    }

    let page = LoginPage {
        head: head("Admin — same.actor", "Sign in.", &state.assets.css, ""),
        error: String::new(),
    };

    Ok(render(&page)?.into_response())
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub token: String,
}

pub async fn session(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    if !state.admin.enabled() {
        return Err(AppError::NotFound);
    }

    if !state.admin.token_matches(form.token.trim()) {
        // Re-render the form rather than redirecting: the token is not in the URL, so there is
        // nothing to clean up, and the error belongs beside the field.
        let page = LoginPage {
            head: head("Admin — same.actor", "Sign in.", &state.assets.css, ""),
            error: "That token is not valid.".to_owned(),
        };

        return Ok((StatusCode::UNAUTHORIZED, render(&page)?).into_response());
    }

    let id = state.admin.open_session();
    let secure = state.config.public_origin.starts_with("https://");
    let mut response = Redirect::to("/admin/overview").into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        crate::admin::admin_cookie_header(&id, secure),
    );

    Ok(response)
}

pub async fn logout(State(state): State<AppState>) -> Response {
    state.admin.close_all();

    let mut response = Redirect::to("/admin").into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_static(
            "sa_admin=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        ),
    );
    response
}

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    pub window: Option<String>,
    pub ip: Option<String>,
    pub path: Option<String>,
    pub before: Option<i64>,
}

impl WindowQuery {
    fn window(&self) -> Window {
        match self.window.as_deref() {
            Some("7d") => Window::Week,
            Some("30d") => Window::Month,
            _ => Window::Day,
        }
    }
}

pub async fn overview(
    State(state): State<AppState>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let window = query.window();

    let (counts, p95_ms, top_paths, top_countries) = tokio::try_join!(
        state.store.counts(window),
        state.store.p95_ms(window),
        state.store.top_paths(window, 8),
        state.store.geo(window.days()),
    )?;

    let (log_rows, db_bytes, requests, errors) = tokio::try_join!(
        state.store.log_row_count(),
        state.store.db_bytes(),
        state.store.timeseries(Metric::Requests, window),
        state.store.timeseries(Metric::Errors, window),
    )?;

    let (top, zero_results, comparisons) = tokio::try_join!(
        state.store.trending_terms(window, 8),
        state.store.zero_result_searches(window.days(), 8),
        state.store.comparisons(8),
    )?;

    let overview = crate::store::report::Overview {
        window: window.as_str().to_owned(),
        counts,
        p95_ms,
        top_paths,
        top_countries,
        log_rows,
        db_bytes,
    };

    let series: Vec<(String, i64)> = requests
        .into_iter()
        .map(|day| (day.day, day.value))
        .collect();
    let error_series: Vec<(String, i64)> =
        errors.into_iter().map(|day| (day.day, day.value)).collect();

    let page = overview_page(
        head(
            "Overview — same.actor admin",
            "Traffic, errors and searches.",
            &state.assets.css,
            "overview",
        ),
        window.as_str(),
        OverviewData {
            overview: &overview,
            series: &series,
            errors: &error_series,
            terms: term_rows(&top),
            zero_terms: term_rows(&zero_results),
            comparisons: comparison_rows(
                &comparisons,
                state.config.public_origin.trim_end_matches('/'),
            ),
        },
    );

    Ok(render(&page)?.into_response())
}

pub async fn logs(
    State(state): State<AppState>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let window = query.window();
    let now = crate::clock::now_unix();

    let log_query = LogQuery {
        since: Some(now - window.seconds()),
        ip: query.ip.clone(),
        path: query.path.clone(),
        before_id: query.before,
        limit: Some(100),
        ..LogQuery::default()
    };

    let page = state.store.logs(log_query).await?;
    let filter = query
        .ip
        .clone()
        .map(|ip| format!("&ip={ip}"))
        .or_else(|| query.path.clone().map(|path| format!("&path={path}")))
        .unwrap_or_default();

    let view = LogsPage::new(
        head(
            "Logs — same.actor admin",
            "Request log.",
            &state.assets.css,
            "logs",
        ),
        window.as_str(),
        page,
        filter,
        now,
    );

    Ok(render(&view)?.into_response())
}

pub async fn log(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let Some(row) = state.store.log_by_id(id).await? else {
        return Err(AppError::NotFound);
    };

    let view = LogPage2::new(
        head(
            &format!("Request {id} — same.actor admin"),
            "One request.",
            &state.assets.css,
            "logs",
        ),
        query.window().as_str(),
        row,
    );

    Ok(render(&view)?.into_response())
}

pub async fn suspicious(
    State(state): State<AppState>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let window = query.window();
    let rows = state.store.suspicious(window, 50).await?;

    let page = suspicion_page(
        head(
            "Suspicious — same.actor admin",
            "Clients worth a look.",
            &state.assets.css,
            "suspicious",
        ),
        window.as_str(),
        &rows,
    );

    Ok(render(&page)?.into_response())
}

pub async fn bans(
    State(state): State<AppState>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let rows = state.store.bans().await?;

    let page = BansPage::new(
        head(
            "Bans — same.actor admin",
            "Blocked addresses.",
            &state.assets.css,
            "bans",
        ),
        &rows,
        query.window().as_str(),
        String::new(),
        String::new(),
    );

    Ok(render(&page)?.into_response())
}

#[derive(Debug, Deserialize)]
pub struct BanForm {
    pub target: String,
    #[serde(default)]
    pub reason: String,
}

pub async fn create_ban(
    State(state): State<AppState>,
    Form(form): Form<BanForm>,
) -> Result<Response, AppError> {
    let reason = form.reason.trim();

    state
        .store
        .insert_ban(crate::store::bans::NewBan {
            kind: if form.target.contains('/') {
                "cidr".to_owned()
            } else {
                "ip".to_owned()
            },
            target: form.target.trim().to_owned(),
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
            created_by: "admin".to_owned(),
            expires_ts: None,
        })
        .await?;

    state.bans.reload(&state.store).await?;

    Ok(Redirect::to("/admin/bans").into_response())
}

pub async fn delete_ban(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if !state.store.delete_ban(id).await? {
        return Err(AppError::NotFound);
    }

    state.bans.reload(&state.store).await?;

    Ok(Redirect::to("/admin/bans").into_response())
}

pub async fn health(
    State(state): State<AppState>,
    Query(query): Query<WindowQuery>,
) -> Result<Response, AppError> {
    let health = state
        .store
        .health(state.bans.len(), state.geo.age_secs(), state.geo.enabled())
        .await?;

    let page = health_page(
        head(
            "Health — same.actor admin",
            "Store and writer health.",
            &state.assets.css,
            "health",
        ),
        query.window().as_str(),
        &health,
        None,
    );

    Ok(render(&page)?.into_response())
}
