//! HTTP-level tests against a stub TMDB.
//!
//! These are the regression tests for the two Critical findings in `docs/REVIEW.md`:
//!
//! * C1 — a selection `kind` that is not a member of the enum can never reach a URL.
//! * C2 — an upstream failure or a bad request returns an error status; it does not take the
//!   service down. The old API crashed the process on both.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;

use same_actor::build_app;
use same_actor::config::{Config, TmdbAuth};
use same_actor::state::AppState;
use same_actor::store::Store;

// ---------------------------------------------------------------------------------------
// Stub upstream
// ---------------------------------------------------------------------------------------

#[derive(Clone)]
struct Stub {
    hits: Arc<AtomicUsize>,
}

async fn stub(State(stub): State<Stub>, request: Request<Body>) -> Response {
    stub.hits.fetch_add(1, Ordering::SeqCst);
    let path = request.uri().path().to_owned();

    match path.as_str() {
        "/3/search/multi" => Json(json!({
            "results": [
                {
                    "id": 603, "media_type": "movie", "title": "The Matrix",
                    "release_date": "1999-03-31", "overview": "Hackers.",
                    "poster_path": "/matrix.jpg"
                },
                {
                    "id": 604, "media_type": "movie", "title": "The Matrix Reloaded",
                    "release_date": "2003-05-15", "overview": "More hackers.",
                    "poster_path": "/reloaded.jpg"
                },
                {
                    // No date and no poster: must not become NaN or a fake URL.
                    "id": 605, "media_type": "movie", "title": "Undated",
                    "overview": null, "poster_path": null
                },
                {
                    // A person in a multi search must be filtered out.
                    "id": 6384, "media_type": "person", "name": "Keanu Reeves"
                },
                {
                    // Adult content is excluded upstream; assert we do not resurrect it.
                    "id": 606, "media_type": "movie", "title": "Not A Title At All"
                }
            ]
        }))
        .into_response(),

        "/3/search/person" => Json(json!({
            "results": [{
                "id": 6384, "name": "Keanu Reeves", "profile_path": "/keanu.jpg",
                "known_for": [
                    {"id": 603, "title": "The Matrix", "release_date": "1999-03-31"},
                    {"id": 1, "name": "Some Show", "first_air_date": "2019-01-01"}
                ]
            }]
        }))
        .into_response(),

        "/3/movie/603" => Json(json!({
            "id": 603,
            "title": "The Matrix",
            "release_date": "1999-03-31",
            "poster_path": "/matrix.jpg",
            "credits": {
                "cast": [
                    {"id": 6384, "name": "Keanu Reeves", "character": "Neo", "profile_path": "/k.jpg"},
                    {"id": 2975, "name": "Laurence Fishburne", "character": "Morpheus"},
                    {"id": 530, "name": "Carrie-Anne Moss", "character": "Trinity"}
                ]
            }
        }))
        .into_response(),

        "/3/movie/604" => Json(json!({
            "id": 604,
            "title": "The Matrix Reloaded",
            "release_date": "2003-05-15",
            "poster_path": "/reloaded.jpg",
            "credits": {
                "cast": [
                    {"id": 6384, "name": "Keanu Reeves", "character": "Neo", "profile_path": "/k.jpg"},
                    {"id": 530, "name": "Carrie-Anne Moss", "character": "Trinity"},
                    {"id": 1331, "name": "Hugo Weaving", "character": "Agent Smith"}
                ]
            }
        }))
        .into_response(),

        "/3/tv/1396" => Json(json!({
            "id": 1396,
            "name": "Breaking Bad",
            "first_air_date": "2008-01-20",
            "poster_path": "/bb.jpg",
            "aggregate_credits": {
                "cast": [
                    {"id": 17419, "name": "Bryan Cranston", "roles": [{"character": "Walter White"}]},
                    {"id": 6384, "name": "Keanu Reeves", "roles": [{"character": "Guest"}]}
                ]
            }
        }))
        .into_response(),

        // A title TMDB fails on. Used to prove a 5xx upstream does not kill the service.
        "/3/movie/999" => {
            (StatusCode::INTERNAL_SERVER_ERROR, "upstream is broken").into_response()
        }

        "/3/person/6384" => Json(json!({
            "id": 6384,
            "name": "Keanu Reeves",
            "profile_path": "/keanu.jpg",
            "combined_credits": {
                "cast": [
                    {"id": 603, "media_type": "movie", "title": "The Matrix",
                     "release_date": "1999-03-31", "character": "Neo", "poster_path": "/m.jpg"},
                    {"id": 604, "media_type": "movie", "title": "The Matrix Reloaded",
                     "release_date": "2003-05-15", "character": "Neo"},
                    {"id": 605, "media_type": "movie", "title": "Only Keanu",
                     "release_date": "2010-01-01", "character": "Himself"},
                    // Crew-only entries must be ignored.
                    {"id": 607, "media_type": "tv", "name": "Crew Only"}
                ]
            }
        }))
        .into_response(),

        "/3/person/530" => Json(json!({
            "id": 530,
            "name": "Carrie-Anne Moss",
            "profile_path": "/cam.jpg",
            "combined_credits": {
                "cast": [
                    {"id": 603, "media_type": "movie", "title": "The Matrix",
                     "release_date": "1999-03-31", "character": "Trinity"},
                    {"id": 604, "media_type": "movie", "title": "The Matrix Reloaded",
                     "release_date": "2003-05-15", "character": "Trinity"}
                ]
            }
        }))
        .into_response(),

        _ => (StatusCode::NOT_FOUND, "no stub for this path").into_response(),
    }
}

struct Harness {
    app: Router,
    hits: Arc<AtomicUsize>,
    /// The same store the app writes to, so a test can assert on what was recorded rather
    /// than on what the handler happened to return.
    store: Arc<Store>,
    /// The app's state, so a test can reach the ban cache the middleware actually reads.
    state: AppState,
}

/// A minimal SPA shell with the Open Graph markers, so page rendering can be tested without
/// a real build. Created exactly once per test process.
fn test_static_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("same-actor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("index.html"),
            r#"<!doctype html><html><head><!--og:start--><title>default</title><!--og:end--></head><body><div id="root"></div></body></html>"#,
        )
        .unwrap();
        dir
    })
    .clone()
}

async fn harness(rate_limit: u32) -> Harness {
    harness_with(rate_limit, |_| {}).await
}

/// A harness whose config a test can adjust — needed to exercise rules that are off by default,
/// like the automatic bans.
async fn harness_with<F: FnOnce(&mut Config)>(rate_limit: u32, tweak: F) -> Harness {
    let hits = Arc::new(AtomicUsize::new(0));

    let stub_app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .fallback(stub)
        .with_state(Stub { hits: hits.clone() });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, stub_app).await.unwrap();
    });

    // A minimal SPA shell carrying the Open Graph markers, so page rendering can be tested
    // without a real build.
    // Written once per process. Tests run in parallel and each calls `harness()`, so
    // rewriting the file here meant a concurrent reader could catch it truncated — a reader
    // seeing an empty shell gets an empty body, and the flake looked like a renderer bug.
    let static_dir = test_static_dir();

    // Each harness gets its own database. Sharing one file would let a parallel test's rows
    // show up in this test's counts, which is the same class of flake as the shared shell.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let db_path = static_dir.join(format!(
        "harness-{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db_path.display()));
    }

    let mut config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        tmdb_token: "test-token".into(),
        tmdb_auth: TmdbAuth::Bearer,
        tmdb_base: format!("http://{addr}/3"),
        image_base: "https://image.tmdb.org/t/p/w342".into(),
        static_dir: static_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        log_retention_days: 90,
        log_assets: false,
        geo_enabled: false,
        geo_db_path: "unused.mmdb".into(),
        geo_url: "https://example.invalid/{month}.mmdb.gz".into(),
        public_origin: "https://same.test".into(),
        tmdb_logo_url: "https://same.test/logo.svg".into(),
        cache_ttl: Duration::from_secs(60),
        cache_capacity: 100,
        rate_limit,
        rate_window: Duration::from_secs(60),
        admin_token: None,
        autoban_requests: 0,
        autoban_errors: 0,
        autoban_window: Duration::from_secs(60),
        autoban_ban_secs: 3600,
        trust_forwarded_for: false,
    };

    tweak(&mut config);

    let state = AppState::new(config).unwrap();
    let store = Arc::clone(&state.store);

    Harness {
        app: build_app(state.clone()),
        hits,
        store,
        state,
    }
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();

    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };

    (status, headers, body)
}

/// The cookie the API now requires. Shape-checked by the server (22 base64url characters), so
/// a made-up value here has to look like a real one.
const SESSION: &str = "testsessionAAAAAAAAAAA";

/// The address the tests appear to come from. Without a `ConnectInfo` extension the server sees
/// `0.0.0.0`, which would make ban tests assert on a fabricated address rather than a realistic
/// one — and would log nonsense.
const CLIENT_IP: &str = "203.0.113.7";

fn with_client(request: Request<Body>) -> Request<Body> {
    let (mut parts, body) = request.into_parts();
    parts.extensions.insert(axum::extract::ConnectInfo(
        format!("{CLIENT_IP}:50000")
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    Request::from_parts(parts, body)
}

/// Every request the tests make carries a session, because `/api/search` and `/api/findout`
/// refuse one that does not. The `*_anonymous` helpers exist so the refusal itself is tested.
fn with_session(request: Request<Body>) -> Request<Body> {
    let (mut parts, body) = with_client(request).into_parts();
    parts.headers.insert(
        axum::http::header::COOKIE,
        format!("sa_sid={SESSION}").parse().expect("cookie header"),
    );
    Request::from_parts(parts, body)
}

fn post_findout_anonymous(body: Value) -> Request<Body> {
    with_client(
        Request::builder()
            .method("POST")
            .uri("/api/findout")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
}

fn get_search_anonymous(query: &str) -> Request<Body> {
    with_client(
        Request::builder()
            .uri(format!("/api/search{query}"))
            .body(Body::empty())
            .unwrap(),
    )
}

fn post_findout(body: Value) -> Request<Body> {
    with_session(post_findout_anonymous(body))
}

fn get_search(query: &str) -> Request<Body> {
    with_session(get_search_anonymous(&format!("?{query}")))
}

async fn call_raw(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

fn get_path(path: &str) -> Request<Body> {
    // Share cards and comparison pages stay anonymous on purpose; adding the cookie here keeps
    // those tests representative of a browser rather than of an unfurler.
    with_session(Request::builder().uri(path).body(Body::empty()).unwrap())
}

fn get_healthz() -> Request<Body> {
    with_client(
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
}

fn error_code(body: &Value) -> &str {
    body.get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no error.code in {body}"))
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn healthz_is_ok() {
    let h = harness(100).await;
    let (status, _, body) = call(&h.app, get_healthz()).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn search_returns_titles_and_drops_people() {
    let h = harness(100).await;
    let (status, _, body) = call(&h.app, get_search("mode=title&term=matrix")).await;

    assert_eq!(status, StatusCode::OK);
    let results = body["results"].as_array().unwrap();

    assert!(
        results.iter().all(|r| r["kind"] != "person"),
        "a person must never appear in a title search"
    );
    assert_eq!(results[0]["kind"], "movie");
    assert_eq!(results[0]["year"], 1999);
    assert_eq!(
        results[0]["image"],
        "https://image.tmdb.org/t/p/w342/matrix.jpg"
    );

    // An entry with no date or artwork must be null, not NaN or a fabricated URL.
    let undated = results.iter().find(|r| r["id"] == 605).unwrap();
    assert!(undated["year"].is_null());
    assert!(undated["image"].is_null());
}

#[tokio::test]
async fn person_search_builds_known_for() {
    let h = harness(100).await;
    let (status, _, body) = call(&h.app, get_search("mode=person&term=keanu")).await;

    assert_eq!(status, StatusCode::OK);
    let results = body["results"].as_array().unwrap();
    assert_eq!(results[0]["kind"], "person");
    assert_eq!(
        results[0]["overview"],
        "Known for: The Matrix (1999), Some Show (2019)"
    );
}

#[tokio::test]
async fn a_short_search_term_is_rejected() {
    let h = harness(100).await;
    let (status, _, body) = call(&h.app, get_search("mode=title&term=ab")).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(&body), "validation_error");
}

#[tokio::test]
async fn findout_title_mode_returns_shared_cast() {
    let h = harness(100).await;
    let request = post_findout(json!({
        "mode": "title",
        "selection": [
            {"id": 603, "kind": "movie"},
            {"id": 604, "kind": "movie"}
        ]
    }));

    let (status, _, body) = call(&h.app, request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "title");

    let matches = body["matches"].as_array().unwrap();
    let ids: Vec<u64> = matches.iter().map(|m| m["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, vec![530, 6384], "both shared actors, ordered by name");

    let keanu = matches.iter().find(|m| m["id"] == 6384).unwrap();
    assert_eq!(keanu["name"], "Keanu Reeves");
    assert_eq!(keanu["credits"].as_array().unwrap().len(), 2);
    assert_eq!(keanu["credits"][0]["selection"], 603);
    assert_eq!(keanu["credits"][0]["character"][0], "Neo");

    // Someone in only one of the two titles must not appear.
    assert!(!ids.contains(&1331), "Hugo Weaving is only in the sequel");
    assert!(
        !ids.contains(&2975),
        "Laurence Fishburne is only in the first film"
    );

    // The response describes its own selections, so a shared URL renders without any
    // client-side state.
    let selections = body["selections"].as_array().unwrap();
    assert_eq!(selections[0]["name"], "The Matrix");
    assert_eq!(selections[0]["kind"], "movie");
    assert_eq!(selections[0]["year"], 1999);
    assert_eq!(
        selections[0]["image"],
        "https://image.tmdb.org/t/p/w342/matrix.jpg"
    );
    assert_eq!(selections[1]["name"], "The Matrix Reloaded");
}

/// A movie and a tv show can be compared against each other.
#[tokio::test]
async fn findout_handles_mixed_movie_and_tv() {
    let h = harness(100).await;
    let request = post_findout(json!({
        "mode": "title",
        "selection": [
            {"id": 603, "kind": "movie"},
            {"id": 1396, "kind": "tv"}
        ]
    }));

    let (status, _, body) = call(&h.app, request).await;
    assert_eq!(status, StatusCode::OK);

    let selections = body["selections"].as_array().unwrap();
    assert_eq!(selections[1]["name"], "Breaking Bad");
    assert_eq!(selections[1]["kind"], "tv");
    assert_eq!(selections[1]["year"], 2008);

    // Keanu is a guest in the show and in the film; Bryan Cranston is only in one.
    let matches = body["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["id"], 6384);
    // A tv character came from `roles[].character`.
    assert_eq!(matches[0]["credits"][1]["character"][0], "Guest");
}

#[tokio::test]
async fn findout_person_mode_returns_shared_titles() {
    let h = harness(100).await;
    let request = post_findout(json!({
        "mode": "person",
        "selection": [
            {"id": 6384, "kind": "person"},
            {"id": 530, "kind": "person"}
        ]
    }));

    let (status, _, body) = call(&h.app, request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "person");

    let matches = body["matches"].as_array().unwrap();
    let ids: Vec<u64> = matches.iter().map(|m| m["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&603) && ids.contains(&604));
    assert!(
        !ids.contains(&605),
        "a title only one person was in is not shared"
    );
}

/// Review C1. The exact payload that used to be interpolated into the TMDB URL and take the
/// service down.
#[tokio::test]
async fn a_non_enum_kind_never_reaches_the_upstream() {
    let h = harness(100).await;

    for kind in [
        "../../account",
        "../../../../",
        "Movie",
        "",
        "movie ", /* trailing space */
    ] {
        let request = post_findout(json!({
            "mode": "title",
            "selection": [
                {"id": 603, "kind": kind},
                {"id": 604, "kind": "movie"}
            ]
        }));

        let (status, _, body) = call(&h.app, request).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "kind {kind:?} must be rejected by the deserialiser"
        );
        assert_eq!(error_code(&body), "validation_error");
    }

    assert_eq!(
        h.hits.load(Ordering::SeqCst),
        0,
        "no malformed selection may produce an upstream request"
    );
}

#[tokio::test]
async fn a_non_numeric_id_never_reaches_the_upstream() {
    let h = harness(100).await;

    for id in [json!("../../account"), json!("603abc"), json!(0), json!(-1)] {
        let request = post_findout(json!({
            "mode": "title",
            "selection": [
                {"id": id, "kind": "movie"},
                {"id": 604, "kind": "movie"}
            ]
        }));

        let (status, _, _) = call(&h.app, request).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "id {id} must be rejected"
        );
    }

    assert_eq!(h.hits.load(Ordering::SeqCst), 0);
}

/// Review C2, second half: a single selection used to be dereferenced as if it were a pair
/// of credits, throwing an unhandled rejection that terminated the Node process.
#[tokio::test]
async fn a_single_selection_is_a_422_not_a_crash() {
    let h = harness(100).await;
    let request = post_findout(json!({
        "mode": "person",
        "selection": [{"id": 603, "kind": "movie"}]
    }));

    let (status, _, body) = call(&h.app, request).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(&body), "validation_error");
    assert_eq!(h.hits.load(Ordering::SeqCst), 0);
}

/// Review C2, first half: an upstream 5xx must be a 502 for that request and nothing more.
#[tokio::test]
async fn an_upstream_failure_is_a_502_and_the_service_keeps_serving() {
    let h = harness(100).await;

    let request = post_findout(json!({
        "mode": "title",
        "selection": [
            {"id": 603, "kind": "movie"},
            {"id": 999, "kind": "movie"}
        ]
    }));

    let (status, _, body) = call(&h.app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error_code(&body), "upstream_unavailable");
    assert!(
        !body.to_string().contains("upstream is broken"),
        "the upstream's own error text must not reach the client: {body}"
    );

    // The old API restarted its container here. The process must still answer.
    let (status, _, body) = call(&h.app, get_healthz()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    // And a good request must still work.
    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_id_is_a_404() {
    let h = harness(100).await;
    let request = post_findout(json!({
        "mode": "title",
        "selection": [
            {"id": 603, "kind": "movie"},
            {"id": 123456, "kind": "movie"}
        ]
    }));

    let (status, _, body) = call(&h.app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&body), "not_found");
}

#[tokio::test]
async fn the_rate_limit_returns_429_with_retry_after() {
    let h = harness(2).await;

    assert_eq!(
        call(&h.app, get_search("mode=title&term=matrix")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&h.app, get_search("mode=title&term=matrix")).await.0,
        StatusCode::OK
    );

    let (status, headers, body) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "rate_limited");
    assert!(
        headers.get(header::RETRY_AFTER).is_some(),
        "a 429 without Retry-After leaves the client guessing"
    );
}

#[tokio::test]
async fn repeated_searches_are_served_from_cache() {
    let h = harness(100).await;

    call(&h.app, get_search("mode=title&term=matrix")).await;
    let after_first = h.hits.load(Ordering::SeqCst);
    assert_eq!(after_first, 1);

    call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(
        h.hits.load(Ordering::SeqCst),
        after_first,
        "the second identical search must not hit the upstream"
    );
}

#[tokio::test]
async fn successful_responses_carry_a_cache_lifetime() {
    let h = harness(100).await;
    let (_, headers, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    let value = headers
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(value.contains("max-age="), "got {value:?}");
}

#[tokio::test]
async fn errors_are_always_json() {
    let h = harness(100).await;

    let cases = [
        get_search("mode=title&term=ab"),
        get_search("mode=title"),                // term missing
        get_search("mode=nonsense&term=matrix"), // bad enum
        with_session(
            Request::builder()
                .method("POST")
                .uri("/api/findout")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        ),
    ];

    for request in cases {
        let (status, headers, body) = call(&h.app, request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "error responses must be JSON, not text/html"
        );
        assert!(
            body.get("error").is_some(),
            "missing error envelope: {body}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Open Graph
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_comparison_page_carries_its_own_open_graph_tags() {
    let h = harness(100).await;
    let (status, headers, body) = call(&h.app, get_path("/title/movie-603,movie-604")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );

    let html = body.as_str().expect("html body");

    assert!(
        html.contains(r#"<meta property="og:title" content="The Matrix vs The Matrix Reloaded — same.actor" />"#),
        "og:title missing or wrong:\n{html}"
    );
    assert!(
        html.contains(r#"<meta property="og:image" content="https://same.test/og/title/movie-603,movie-604.png" />"#),
        "og:image missing or wrong:\n{html}"
    );
    assert!(html.contains(r#"<meta name="twitter:card" content="summary_large_image" />"#));
    assert!(html.contains("credited in at least two"));

    // Exactly one <title>: the default must be replaced, not appended to.
    assert_eq!(
        html.matches("<title>").count(),
        1,
        "duplicate <title>:\n{html}"
    );
    assert!(!html.contains("<title>default</title>"));
}

#[tokio::test]
async fn the_share_card_is_served_as_a_png() {
    let h = harness(100).await;
    let (status, headers, body) =
        call_raw(&h.app, get_path("/og/title/movie-603,movie-604.png")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
    assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

    // IHDR carries the dimensions the meta tags advertise.
    let width = u32::from_be_bytes([body[16], body[17], body[18], body[19]]);
    let height = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
    assert_eq!((width, height), (1200, 630));
}

#[tokio::test]
async fn the_card_is_cached_and_the_second_request_does_not_refetch() {
    let h = harness(100).await;

    call_raw(&h.app, get_path("/og/title/movie-603,movie-604.png")).await;
    let after_first = h.hits.load(Ordering::SeqCst);
    assert!(after_first > 0, "the first request must hit the stub");

    call_raw(&h.app, get_path("/og/title/movie-603,movie-604.png")).await;
    assert_eq!(
        h.hits.load(Ordering::SeqCst),
        after_first,
        "the second card request must come from the cache"
    );
}

#[tokio::test]
async fn a_short_selector_gets_the_plain_page_not_tags() {
    let h = harness(100).await;
    let (status, _, body) = call(&h.app, get_path("/title/movie-603")).await;

    assert_eq!(status, StatusCode::OK);
    let html = body.as_str().unwrap();
    assert!(
        html.contains("<title>default</title>"),
        "default title should survive"
    );
    assert!(!html.contains("og:image"));
}

#[tokio::test]
async fn an_unknown_mode_is_not_a_page() {
    let h = harness(100).await;
    let (status, _, _) = call(&h.app, get_path("/nonsense/movie-603,movie-604")).await;
    // Not a route of ours, so it falls through to the SPA fallback: still a page, but the
    // static handler owns it rather than the Open Graph renderer.
    assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);

    let (status, _, _) = call(&h.app, get_path("/og/nonsense/movie-603")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_single_pick_has_no_card() {
    let h = harness(100).await;
    let (status, _, _) = call_raw(&h.app, get_path("/og/title/movie-603.png")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn html_escaping_survives_a_hostile_path() {
    let h = harness(100).await;
    // Not a valid pick, so the page is served plain — the point is that nothing is echoed.
    let (status, _, body) = call(&h.app, get_path("/title/movie-603%3Cscript%3E")).await;
    assert_eq!(status, StatusCode::OK);
    let html = body.as_str().unwrap();
    assert!(!html.contains("<script>alert"));
}

/// The API refuses a request without a session, and hands back the cookie to retry with. This
/// is the contract the plan settled on: the two endpoints that call TMDB cost money and
/// represent a visit, so they are not served anonymously.
#[tokio::test]
async fn search_without_a_session_is_refused_with_a_cookie() {
    let h = harness(100).await;

    let (status, headers, body) =
        call(&h.app, get_search_anonymous("?mode=title&term=matrix")).await;

    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(error_code(&body), "session_required");

    let cookie = headers
        .get(header::SET_COOKIE)
        .expect("a refusal must hand back a cookie")
        .to_str()
        .unwrap();
    assert!(cookie.starts_with("sa_sid="), "unexpected cookie: {cookie}");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
}

#[tokio::test]
async fn findout_without_a_session_is_refused() {
    let h = harness(100).await;

    let (status, _, body) = call(
        &h.app,
        post_findout_anonymous(serde_json::json!({
            "mode": "title",
            "selection": [
                {"id": 603, "kind": "movie"},
                {"id": 604, "kind": "movie"}
            ]
        })),
    )
    .await;

    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(error_code(&body), "session_required");
}

/// The refusals are specific, and the exemptions are the part that breaks embeds if they
/// regress — so each is asserted rather than one standing in for the rest.
#[tokio::test]
async fn the_endpoints_unfurlers_and_crawlers_use_stay_anonymous() {
    let h = harness(100).await;

    let (status, _, _) = call(&h.app, get_path("/og/title/movie-603,movie-604.png")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a share card must not require a cookie"
    );

    let (status, _, _) = call(&h.app, get_path("/title/movie-603,movie-604")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a comparison page must not require a cookie"
    );

    let (status, _, _) = call(&h.app, get_healthz()).await;
    assert_eq!(status, StatusCode::OK, "liveness must not require a cookie");
}

/// A page view mints the cookie, so the SPA's very first API call already has one.
#[tokio::test]
async fn a_page_view_hands_out_a_session() {
    let h = harness(100).await;

    let response = h
        .app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("a first page view must set a session")
        .to_str()
        .unwrap();

    assert!(cookie.starts_with("sa_sid="));
    assert!(
        cookie.contains("HttpOnly"),
        "the session must not be readable by script"
    );
    // This harness declares an https origin, so `Secure` is expected here. The plain-http case
    // — where `Secure` would make the cookie vanish — is covered by the session unit tests.
    assert!(cookie.contains("Secure"));
}

// ---------------------------------------------------------------------------------------------
// Request logging
// ---------------------------------------------------------------------------------------------

/// Rows are written by a background thread, so every assertion here flushes first. `flush` is a
/// barrier — it returns only once everything offered before it is committed — which is why
/// these tests need no sleeps and cannot flake on a slow machine.
async fn logged(h: &Harness) -> Vec<same_actor::store::RequestRow> {
    h.store.flush().await.expect("flush");
    h.store.recent_requests(100).await.expect("read")
}

#[tokio::test]
async fn a_request_is_recorded_with_its_session_and_address() {
    let h = harness(100).await;
    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::OK);

    let rows = logged(&h).await;
    assert_eq!(rows.len(), 1, "one request should be one row: {rows:?}");

    let row = &rows[0];
    assert_eq!(row.path, "/api/search");
    assert_eq!(row.method, "GET");
    assert_eq!(row.status, 200);
    assert_eq!(row.kind, "api");
    assert_eq!(row.session.as_deref(), Some(SESSION));
    assert!(row.ip.is_some(), "the client address should be recorded");
    assert!(row.code.is_none(), "a success carries no error code");
}

/// The upstream detail is never in the response body, but it is exactly what an operator needs
/// in the log when a request 502s.
#[tokio::test]
async fn a_failure_is_recorded_with_a_code_and_a_message() {
    let h = harness(100).await;

    // Title 999 is the stub's broken upstream: any request that touches it 502s.
    let (status, _, _) = call(
        &h.app,
        post_findout(json!({
            "mode": "title",
            "selection": [
                {"id": 603, "kind": "movie"},
                {"id": 999, "kind": "movie"}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let rows = logged(&h).await;
    let row = rows
        .iter()
        .find(|row| row.status == 502)
        .expect("the 502 row");

    assert_eq!(row.code.as_deref(), Some("upstream_unavailable"));
    let message = row.message.as_deref().unwrap_or_default();
    assert!(!message.is_empty(), "a failure should say why");
    // The body stays generic; the log is where the detail lives, because the operator is the
    // only one who should see which upstream call failed.
    assert!(
        message.contains("upstream is broken") || message.contains("999"),
        "the log should carry the upstream detail, got {message:?}"
    );
}

#[tokio::test]
async fn a_refusal_is_recorded() {
    let h = harness(100).await;

    let (status, _, _) = call(&h.app, get_search_anonymous("?mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);

    let rows = logged(&h).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, 428);
    assert_eq!(rows[0].code.as_deref(), Some("session_required"));
    assert!(
        rows[0].session.is_some(),
        "a refused request still gets a session, or the retry cannot be matched to it"
    );
}

#[tokio::test]
async fn a_rate_limited_request_is_recorded() {
    let h = harness(1).await;

    let _ = call(&h.app, get_search("mode=title&term=matrix")).await;
    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let rows = logged(&h).await;
    assert!(
        rows.iter()
            .any(|row| row.status == 429 && row.code.as_deref() == Some("rate_limited")),
        "the 429 is missing from the log: {rows:?}"
    );
}

/// One page load is one row, not five. This is the difference between 8 MB/day and 32 MB/day.
#[tokio::test]
async fn assets_and_health_checks_are_not_recorded_by_default() {
    let h = harness(100).await;

    let _ = call(&h.app, get_healthz()).await;
    let _ = call(&h.app, get_path("/assets/index-abc123.js")).await;
    let _ = call(&h.app, get_path("/favicon.ico")).await;

    assert!(
        logged(&h).await.is_empty(),
        "an asset or a probe should not produce a log row"
    );
}

#[tokio::test]
async fn a_share_card_is_recorded_as_a_card() {
    let h = harness(100).await;

    let (status, _, _) = call(&h.app, get_path("/og/title/movie-603,movie-604.png")).await;
    assert_eq!(status, StatusCode::OK);

    let rows = logged(&h).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "card");
}

/// The log must survive store trouble, not amplify it: a failed write is counted and the
/// response is unaffected.
#[tokio::test]
async fn a_request_still_succeeds_when_the_writer_is_stopped() {
    let h = harness(100).await;
    h.store.close().await.expect("close");

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::OK, "analytics must not be load-bearing");
    assert!(h.store.stats().dropped > 0, "the drop should be counted");
}

// ---------------------------------------------------------------------------------------------
// Bans
// ---------------------------------------------------------------------------------------------

/// Install a ban through the same path the admin API will use, then make it live.
async fn ban(h: &Harness, target: &str, expires_ts: Option<i64>) -> i64 {
    let id = same_actor::bans::apply(
        &h.store,
        &h.state.bans,
        same_actor::store::bans::NewBan {
            target: target.into(),
            kind: if target.contains('/') { "cidr" } else { "ip" }.into(),
            reason: Some("test ban".into()),
            created_by: "admin".into(),
            expires_ts,
        },
    )
    .await
    .expect("ban");

    id
}

#[tokio::test]
async fn a_banned_address_is_refused_with_a_reason() {
    let h = harness(100).await;
    let id = ban(&h, CLIENT_IP, None).await;

    let (status, _, body) = call(&h.app, get_search("mode=title&term=matrix")).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&body), "banned");
    assert!(
        body.to_string().contains("test ban"),
        "the operator's reason should reach the client: {body}"
    );

    // The hit is counted, so the dashboard can show which ban is doing work.
    h.store.flush().await.expect("flush");
    let bans = h.store.bans().await.expect("bans");
    assert_eq!(bans.iter().find(|b| b.id == id).map(|b| b.hits), Some(1));
}

#[tokio::test]
async fn a_ban_on_a_range_catches_an_address_inside_it() {
    let h = harness(100).await;
    ban(&h, "203.0.113.0/24", None).await;

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_ban_that_has_expired_no_longer_applies() {
    let h = harness(100).await;
    ban(&h, CLIENT_IP, Some(same_actor::clock::now_unix() - 1)).await;

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an expired ban must not be enforced, and must not need a restart to stop"
    );
}

#[tokio::test]
async fn a_temporary_ban_says_when_to_come_back() {
    let h = harness(100).await;
    ban(&h, CLIENT_IP, Some(same_actor::clock::now_unix() + 600)).await;

    let (status, headers, _) = call(&h.app, get_search("mode=title&term=matrix")).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    let retry_after: u64 = headers
        .get(header::RETRY_AFTER)
        .expect("a temporary ban should say when it ends")
        .to_str()
        .unwrap()
        .parse()
        .expect("seconds");
    assert!(retry_after > 0 && retry_after <= 600);
}

/// A ban must never be able to make the container look unhealthy — that would turn one abusive
/// visitor into a restart loop.
#[tokio::test]
async fn a_ban_never_blocks_the_health_check() {
    let h = harness(100).await;
    ban(&h, CLIENT_IP, None).await;

    let (status, _, _) = call(&h.app, get_healthz()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn removing_a_ban_lets_the_client_back_in() {
    let h = harness(100).await;
    let id = ban(&h, CLIENT_IP, None).await;

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    assert!(h.store.delete_ban(id).await.expect("delete"));
    h.state.bans.reload(&h.store).await.expect("reload");

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::OK, "an unbanned client is served again");
}

/// The error-counting rule is off by default. Configured, a client that keeps failing earns a
/// ban — and the ban is labelled as automatic so an operator can tell it from their own.
#[tokio::test]
async fn an_automatic_rule_bans_a_client_that_keeps_failing() {
    let h = harness_with(100, |config| {
        config.autoban_errors = 2;
        config.autoban_ban_secs = 600;
    })
    .await;

    // Two 422s from the same address, which is what probing with bad parameters produces.
    // (An unknown path is not a 4xx here: the SPA fallback serves index.html for it.)
    for _ in 0..2 {
        let (status, _, _) = call(&h.app, get_search("mode=nonsense&term=matrix")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    // The ban is created in the background, so wait for the row rather than sleeping blindly.
    let mut bans = Vec::new();
    for _ in 0..100 {
        h.store.flush().await.expect("flush");
        bans = h.store.bans().await.expect("bans");
        if !bans.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(bans.len(), 1, "the automatic rule did not fire");
    assert_eq!(bans[0].created_by, "auto:errors");
    assert_eq!(bans[0].target, CLIENT_IP);
    assert!(
        bans[0].expires_ts.is_some(),
        "an automatic ban must expire on its own; a machine should not be able to ban forever"
    );
}

/// A search is recorded as an event, not just as a request row: the term and its result count
/// are what trending and the zero-result report read.
#[tokio::test]
async fn a_search_is_recorded_with_its_term_and_result_count() {
    let h = harness(100).await;

    let (status, _, _) = call(&h.app, get_search("mode=title&term=The%20Matrix")).await;
    assert_eq!(status, StatusCode::OK);

    h.store.flush().await.expect("flush");
    let searches = h.store.recent_searches(10).await.expect("read");

    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].term, "The Matrix");
    assert_eq!(searches[0].term_norm, "the matrix");
    assert_eq!(searches[0].mode, "title");
    assert!(searches[0].results > 0);

    // The address stays on the request row only; this table deliberately has no copy of it.
    assert!(
        searches[0].term != CLIENT_IP,
        "a search row must not carry the address"
    );
}

// ---------------------------------------------------------------------------------------------
// Admin API
// ---------------------------------------------------------------------------------------------

const ADMIN_TOKEN: &str = "test-admin-token-2b7f";

fn with_bearer(request: Request<Body>, token: &str) -> Request<Body> {
    let (mut parts, body) = request.into_parts();
    parts.headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().expect("auth header"),
    );
    Request::from_parts(parts, body)
}

fn admin_get(path: &str) -> Request<Body> {
    with_bearer(with_client(get_path_raw(path)), ADMIN_TOKEN)
}

fn get_path_raw(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

async fn with_admin() -> Harness {
    harness_with(100, |config| {
        config.admin_token = Some(ADMIN_TOKEN.to_owned())
    })
    .await
}

#[tokio::test]
async fn the_admin_api_refuses_a_request_with_no_credential() {
    let h = with_admin().await;

    let (status, _, body) = call(&h.app, with_client(get_path_raw("/api/admin/overview"))).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&body), "unauthorized");
}

#[tokio::test]
async fn the_admin_api_refuses_the_wrong_token() {
    let h = with_admin().await;

    let (status, _, _) = call(
        &h.app,
        with_bearer(
            with_client(get_path_raw("/api/admin/overview")),
            "not-the-token",
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A forgotten env var must not mean an open dashboard — and must not look like a route that
/// exists either.
#[tokio::test]
async fn an_unset_admin_token_makes_the_whole_subtree_not_found() {
    let h = harness(100).await;

    for path in ["/api/admin/overview", "/api/admin/logs", "/api/admin/bans"] {
        let (status, _, _) = call(&h.app, with_bearer(with_client(get_path_raw(path)), "")).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} should not exist when the admin API is disabled"
        );
    }
}

#[tokio::test]
async fn a_bearer_token_reaches_the_dashboard_data() {
    let h = with_admin().await;

    let (status, headers, body) = call(&h.app, admin_get("/api/admin/overview?window=24h")).await;

    assert_eq!(status, StatusCode::OK);
    let cache = headers
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert_eq!(cache, "no-store", "admin data must never be cached");

    assert_eq!(body["counts"]["requests"].as_i64(), Some(0));
    assert_eq!(body["window"], "24h");
}

#[tokio::test]
async fn the_token_can_be_exchanged_for_a_cookie() {
    let h = with_admin().await;

    let (status, headers, _) = call(
        &h.app,
        with_client(
            Request::builder()
                .method("POST")
                .uri("/api/admin/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!("{{\"token\":\"{ADMIN_TOKEN}\"}}")))
                .unwrap(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let cookie = headers
        .get(header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .unwrap()
        .to_owned();

    assert!(cookie.starts_with("sa_admin="));
    assert!(
        cookie.contains("HttpOnly"),
        "the token must not be readable by script"
    );
    assert!(cookie.contains("SameSite=Strict"));
    assert!(
        !cookie.contains(ADMIN_TOKEN),
        "the cookie must hold a session id, never the token itself"
    );

    // And that cookie is accepted.
    let id = cookie.split(';').next().unwrap().to_owned();
    let (status, _, _) = call(
        &h.app,
        Request::builder()
            .uri("/api/admin/health")
            .header(header::COOKIE, id)
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_wrong_token_is_refused_at_the_login() {
    let h = with_admin().await;

    let (status, _, body) = call(
        &h.app,
        with_client(
            Request::builder()
                .method("POST")
                .uri("/api/admin/session")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"token\":\"wrong\"}"))
                .unwrap(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code(&body), "validation_error");
    assert!(
        body.to_string().contains("not correct"),
        "the refusal should be plain about what happened"
    );
}

/// The token is a credential. It must not end up in the log it is used to read.
#[tokio::test]
async fn the_admin_token_never_reaches_the_log() {
    let h = with_admin().await;

    let _ = call(&h.app, admin_get("/api/admin/overview")).await;
    let _ = call(
        &h.app,
        with_bearer(with_client(get_path_raw("/api/admin/logs")), "wrong-token"),
    )
    .await;

    h.store.flush().await.expect("flush");
    let rows = h.store.recent_requests(50).await.expect("read");

    assert!(!rows.is_empty());
    for row in &rows {
        let haystack = format!(
            "{} {} {} {}",
            row.path,
            row.ua.clone().unwrap_or_default(),
            row.message.clone().unwrap_or_default(),
            row.session.clone().unwrap_or_default()
        );
        assert!(
            !haystack.contains(ADMIN_TOKEN) && !haystack.contains("wrong-token"),
            "a token leaked into the log: {haystack}"
        );
        assert_eq!(row.kind, "admin", "admin traffic is classified as such");
    }
}

#[tokio::test]
async fn a_ban_can_be_created_listed_and_removed_through_the_api() {
    let h = with_admin().await;

    // A ban that is not an address or a range must be refused, not stored: it would show in the
    // list while matching nothing.
    let (status, _, _) = call(
        &h.app,
        with_bearer(
            with_client(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/bans")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{\"target\":\"everything\"}"))
                    .unwrap(),
            ),
            ADMIN_TOKEN,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A real one.
    let (status, _, body) = call(
        &h.app,
        with_bearer(
            with_client(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/bans")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"target":"203.0.113.0/24","reason":"scraping","expires_in_secs":600}"#,
                    ))
                    .unwrap(),
            ),
            ADMIN_TOKEN,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_i64().expect("an id");

    // It is listed.
    let (status, _, body) = call(&h.app, admin_get("/api/admin/bans")).await;
    assert_eq!(status, StatusCode::OK);
    let bans = body.as_array().expect("an array");
    assert_eq!(bans.len(), 1);
    assert_eq!(bans[0]["kind"], "cidr");
    assert_eq!(bans[0]["created_by"], "admin");
    assert!(bans[0]["expires_ts"].as_i64().is_some());

    // And it is enforced immediately, on this very client.
    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a ban created through the API must be live on the next request, not at the next refresh"
    );

    // Removing it lets the client back in.
    let (status, _, _) = call(
        &h.app,
        with_bearer(
            with_client(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/admin/bans/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
            ADMIN_TOKEN,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(&h.app, get_search("mode=title&term=matrix")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_log_endpoint_filters() {
    let h = with_admin().await;

    let _ = call(&h.app, get_search("mode=title&term=matrix")).await;
    h.store.flush().await.expect("flush");

    let (status, _, body) = call(&h.app, admin_get("/api/admin/logs?kind=api&limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["path"], "/api/search");

    // A filter that matches nothing returns nothing, rather than everything.
    let (_, _, body) = call(&h.app, admin_get("/api/admin/logs?kind=page")).await;
    assert!(body["rows"].as_array().unwrap().is_empty());

    // A bad window is an error, not a silent default.
    let (status, _, _) = call(&h.app, admin_get("/api/admin/overview?window=forever")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_stats_endpoints_answer_their_own_shapes() {
    let h = with_admin().await;
    let _ = call(&h.app, get_search("mode=title&term=matrix")).await;
    h.store.flush().await.expect("flush");

    // Objects with a named field.
    for (path, key) in [
        (
            "/api/admin/stats/timeseries?metric=requests&window=7d",
            "points",
        ),
        ("/api/admin/stats/geo?days=7", "countries"),
        ("/api/admin/stats/searches?window=7d", "top"),
        ("/api/admin/health", "db_bytes"),
    ] {
        let (status, _, body) = call(&h.app, admin_get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path} -> {body}");
        assert!(body.get(key).is_some(), "{path} is missing {key}: {body}");
    }

    // Bare arrays.
    for path in [
        "/api/admin/stats/comparisons",
        "/api/admin/suspicious?window=24h",
    ] {
        let (status, _, body) = call(&h.app, admin_get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path} -> {body}");
        assert!(body.is_array(), "{path} should be a list: {body}");
    }
}
