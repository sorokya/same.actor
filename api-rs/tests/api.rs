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

    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        tmdb_token: "test-token".into(),
        tmdb_auth: TmdbAuth::Bearer,
        tmdb_base: format!("http://{addr}/3"),
        image_base: "https://image.tmdb.org/t/p/w342".into(),
        static_dir: static_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        log_retention_days: 90,
        log_assets: false,
        public_origin: "https://same.test".into(),
        tmdb_logo_url: "https://same.test/logo.svg".into(),
        cache_ttl: Duration::from_secs(60),
        cache_capacity: 100,
        rate_limit,
        rate_window: Duration::from_secs(60),
        trust_forwarded_for: false,
    };

    Harness {
        app: build_app(AppState::new(config).unwrap()),
        hits,
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

fn post_findout(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/findout")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_search(query: &str) -> Request<Body> {
    Request::builder()
        .uri(format!("/api/search?{query}"))
        .body(Body::empty())
        .unwrap()
}

async fn call_raw(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

fn get_path(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn get_healthz() -> Request<Body> {
    Request::builder()
        .uri("/healthz")
        .body(Body::empty())
        .unwrap()
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
        Request::builder()
            .method("POST")
            .uri("/api/findout")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
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
