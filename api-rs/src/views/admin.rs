//! The dashboard, rendered.
//!
//! Same data as the JSON admin API, read from the same store methods, but delivered as pages:
//! the tables are in the first response, the charts are SVG the server drew, and every filter
//! is a link. There is no script on any of these pages.
//!
//! The one piece of geometry worth naming is [`line`]: a polyline needs its points mapped into
//! a box, and doing that in a template would be arithmetic in a string.

use askama::Template;

use crate::store::bans::Ban;
use crate::store::report::{
    ComparisonRow, CountryRow, Health, LogPage, Overview, PathRow, Suspicion,
};
use crate::store::searches::TrendingTerm;
use crate::store::RequestRow;

use super::{absolute_time, relative_time, Head};

#[derive(Template)]
#[template(path = "pages/admin/login.html")]
pub struct LoginPage {
    pub head: Head,
    /// Set after a failed attempt, so the page can say so without a session variable.
    pub error: String,
}

#[derive(Template)]
#[template(path = "pages/admin/overview.html")]
pub struct OverviewPage {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub requests: String,
    pub visitors: String,
    pub searches: String,
    pub errors: String,
    pub error_share: String,
    pub p95: String,
    pub chart: Line,
    pub error_chart: Line,
    pub paths: Vec<Bar>,
    pub countries: Vec<Bar>,
    pub comparisons: Vec<ComparisonRowView>,
    pub terms: Vec<TermView>,
    pub zero_terms: Vec<TermView>,
    pub footer: String,
}

#[derive(Template)]
#[template(path = "pages/admin/logs.html")]
pub struct LogsPage {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub subtitle: String,
    pub rows: Vec<LogRowView>,
    pub next: String,
    pub filter: String,
}

#[derive(Template)]
#[template(path = "pages/admin/log.html")]
pub struct LogPage2 {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub row: LogRowView,
    pub fields: Vec<(String, String)>,
}

#[derive(Template)]
#[template(path = "pages/admin/suspicious.html")]
pub struct SuspiciousPage {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub rows: Vec<SuspicionView>,
}

#[derive(Template)]
#[template(path = "pages/admin/bans.html")]
pub struct BansPage {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub rows: Vec<BanView>,
    pub error: String,
    pub message: String,
}

#[derive(Template)]
#[template(path = "pages/admin/health.html")]
pub struct HealthPage {
    pub head: Head,
    pub window: &'static str,
    pub windows: Vec<WindowLink>,
    pub facts: Vec<(String, String)>,
}

pub struct WindowLink {
    pub value: &'static str,
    pub label: &'static str,
    pub href: String,
    pub active: bool,
}

pub struct LogRowView {
    pub id: i64,
    pub when: String,
    pub relative: String,
    pub status: String,
    pub status_tone: &'static str,
    pub method: String,
    pub path: String,
    pub duration: String,
    pub ip: String,
    pub country: String,
    pub code: String,
}

pub struct SuspicionView {
    pub ip: String,
    pub requests: i64,
    pub errors: i64,
    pub limited: i64,
    pub reason: String,
}

pub struct BanView {
    pub id: i64,
    pub target: String,
    pub reason: String,
    pub created: String,
    pub expires: String,
}

pub struct ComparisonRowView {
    pub href: String,
    pub label: String,
    pub mode: String,
    pub views: i64,
    pub last_seen: String,
}

pub struct TermView {
    pub term: String,
    pub count: i64,
}

/// One bar in a bar list: the width is a percentage string, so the template never divides.
pub struct Bar {
    pub label: String,
    pub value: i64,
    pub width: String,
    pub href: String,
}

/// An SVG polyline, mapped into a box.
pub struct Line {
    /// `M x,y L x,y …`, or empty for no data.
    pub points: String,
    /// The same path closed to the baseline, for the fill.
    pub area: String,
    pub max: i64,
    pub last: i64,
    pub first_label: String,
    pub last_label: String,
}

/// Map values into a `width` × `height` box with a small inset, so strokes are not clipped.
///
/// A flat series is drawn along the bottom rather than at zero height, and a single value gets
/// one point — both keep "we have data, and it is flat" visually distinct from "we have none".
pub fn line(values: &[(String, i64)], width: f64, height: f64) -> Line {
    let max = values.iter().map(|(_, value)| *value).max().unwrap_or(0);
    let last = values.last().map(|(_, value)| *value).unwrap_or(0);

    let first_label = values
        .first()
        .map(|(label, _)| label.clone())
        .unwrap_or_default();
    let last_label = values
        .last()
        .map(|(label, _)| label.clone())
        .unwrap_or_default();

    if values.is_empty() {
        return Line {
            points: String::new(),
            area: String::new(),
            max,
            last,
            first_label,
            last_label,
        };
    }

    let inset = 2.0;
    let step = if values.len() > 1 {
        (width - inset * 2.0) / (values.len() - 1) as f64
    } else {
        0.0
    };
    // Scaled to the series' own maximum: a flat series is therefore drawn along the top of the
    // box, and the shape of the line — not its height — is what tells you nothing changed.
    let scale = |value: i64| -> f64 {
        if max == 0 {
            height - inset
        } else {
            height - inset - (value as f64 / max as f64) * (height - inset * 2.0)
        }
    };

    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(index, (_, value))| {
            format!("{:.1},{:.1}", inset + step * index as f64, scale(*value))
        })
        .collect();

    let path = format!("M{}", points.join(" L"));

    Line {
        area: format!(
            "{path} L{:.1},{:.1} L{:.1},{:.1} Z",
            inset + step * (values.len() - 1) as f64,
            height,
            inset,
            height
        ),
        points: path,
        max,
        last,
        first_label,
        last_label,
    }
}

fn bars<T>(
    rows: &[T],
    label: impl Fn(&T) -> String,
    value: impl Fn(&T) -> i64,
    href: impl Fn(&T) -> String,
) -> Vec<Bar> {
    let max = rows.iter().map(&value).max().unwrap_or(0).max(1);

    rows.iter()
        .map(|row| Bar {
            width: format!("{:.0}%", value(row) as f64 / max as f64 * 100.0),
            label: label(row),
            value: value(row),
            href: href(row),
        })
        .collect()
}

/// `24h`, `7d`, `30d` as links that keep whatever filter the page already had.
pub fn windows(active: &'static str, base: &str, extra: &str) -> Vec<WindowLink> {
    [("24h", "24 hours"), ("7d", "7 days"), ("30d", "30 days")]
        .into_iter()
        .map(|(value, label)| WindowLink {
            value,
            label,
            href: format!("{base}?window={value}{extra}"),
            active: value == active,
        })
        .collect()
}

pub fn count_label(count: usize, singular: &'static str, plural: &'static str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn log_row(row: &RequestRow, now: i64) -> LogRowView {
    let status = row.status.to_string();
    LogRowView {
        id: row.id,
        when: absolute_time(row.ts),
        relative: relative_time(row.ts, now),
        status_tone: match row.status {
            200..=299 => "text-paper",
            300..=399 => "text-dim",
            400..=499 => "text-accent",
            _ => "text-danger",
        },
        status,
        method: row.method.clone(),
        path: row.path.clone(),
        duration: format!("{} ms", row.duration_ms),
        ip: row.ip.clone().unwrap_or_default(),
        country: row.country.clone().unwrap_or_default(),
        code: row.code.clone().unwrap_or_default(),
    }
}

impl LogsPage {
    pub fn new(head: Head, window: &'static str, page: LogPage, filter: String, now: i64) -> Self {
        let rows: Vec<LogRowView> = page.rows.iter().map(|row| log_row(row, now)).collect();
        let count = rows.len();

        Self {
            head,
            window,
            windows: windows(window, "/admin/logs", &filter_url("", &filter)),
            subtitle: count_label(count, "request", "requests"),
            next: page
                .next_before_id
                .map(|id| {
                    format!(
                        "/admin/logs?window={window}{}&before={id}",
                        filter_url("", &filter)
                    )
                })
                .unwrap_or_default(),
            rows,
            filter,
        }
    }
}

impl LogPage2 {
    pub fn new(head: Head, window: &'static str, row: RequestRow) -> Self {
        let now = crate::clock::now_unix();
        let fields = vec![
            ("Time".to_owned(), absolute_time(row.ts)),
            ("Method".to_owned(), row.method.clone()),
            ("Path".to_owned(), row.path.clone()),
            ("Status".to_owned(), row.status.to_string()),
            ("Duration".to_owned(), format!("{} ms", row.duration_ms)),
            ("Address".to_owned(), row.ip.clone().unwrap_or_default()),
            (
                "Country".to_owned(),
                row.country.clone().unwrap_or_default(),
            ),
            (
                "Session".to_owned(),
                row.session.clone().unwrap_or_default(),
            ),
            ("Kind".to_owned(), row.kind.clone()),
            ("Agent".to_owned(), row.ua.clone().unwrap_or_default()),
            ("Code".to_owned(), row.code.clone().unwrap_or_default()),
        ];

        Self {
            head,
            window,
            windows: windows(window, "/admin/logs", ""),
            row: log_row(&row, now),
            fields,
        }
    }
}

impl BansPage {
    pub fn new(
        head: Head,
        bans: &[Ban],
        window: &'static str,
        error: String,
        message: String,
    ) -> Self {
        let rows = bans
            .iter()
            .map(|ban| BanView {
                id: ban.id,
                target: ban.target.clone(),
                reason: ban.reason.clone().unwrap_or_default(),
                created: absolute_time(ban.created_ts),
                expires: ban
                    .expires_ts
                    .map(|ts| relative_time(ts, crate::clock::now_unix()))
                    .unwrap_or_else(|| "never".to_owned()),
            })
            .collect();

        Self {
            head,
            window,
            windows: windows(window, "/admin/bans", ""),
            rows,
            error,
            message,
        }
    }
}

/// `?ip=1.2.3.4` for the log filter, or empty.
pub fn filter_url(key: &str, value: &str) -> String {
    if key.is_empty() || value.is_empty() {
        return String::new();
    }

    format!(
        "&{key}={}",
        value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_' | '/'))
            .collect::<String>()
    )
}

/// Everything the overview page renders, gathered so the constructor does not take eight
/// positional arguments whose order a reader has to memorise.
pub struct OverviewData<'a> {
    pub overview: &'a Overview,
    pub series: &'a [(String, i64)],
    pub errors: &'a [(String, i64)],
    pub terms: Vec<TermView>,
    pub zero_terms: Vec<TermView>,
    pub comparisons: Vec<ComparisonRowView>,
}

pub fn overview_page(head: Head, window: &'static str, data: OverviewData<'_>) -> OverviewPage {
    let OverviewData {
        overview,
        series,
        errors,
        terms,
        zero_terms,
        comparisons,
    } = data;
    let counts = &overview.counts;
    let error_share = if counts.requests > 0 {
        format!(
            "{:.1}% of requests",
            counts.errors as f64 / counts.requests as f64 * 100.0
        )
    } else {
        String::new()
    };

    OverviewPage {
        head,
        window,
        windows: windows(window, "/admin/overview", ""),
        requests: counts.requests.to_string(),
        visitors: counts.visitors.to_string(),
        searches: counts.searches.to_string(),
        errors: counts.errors.to_string(),
        error_share,
        p95: format!("p95 {} ms", overview.p95_ms),
        chart: line(series, 360.0, 90.0),
        error_chart: line(errors, 360.0, 26.0),
        paths: bars(
            &overview.top_paths,
            |row: &PathRow| row.path.clone(),
            |row| row.requests,
            |row| format!("/admin/logs?path={}", row.path),
        ),
        countries: bars(
            &overview.top_countries,
            |row: &CountryRow| row.country.clone(),
            |row| row.requests,
            |_| String::new(),
        ),
        comparisons,
        terms,
        zero_terms,
        footer: format!(
            "{} log rows retained at full detail · database {}",
            overview.log_rows,
            human_bytes(overview.db_bytes)
        ),
    }
}

pub fn health_page(
    head: Head,
    window: &'static str,
    health: &Health,
    uptime: Option<i64>,
) -> HealthPage {
    let facts = vec![
        (
            "Database".to_owned(),
            format!(
                "{} ({} log rows)",
                human_bytes(health.db_bytes),
                health.log_rows
            ),
        ),
        ("Search rows".to_owned(), health.search_rows.to_string()),
        ("Active bans".to_owned(), health.bans.to_string()),
        (
            "Writer lag".to_owned(),
            format!(
                "{} rows written, {} commits, {} dropped",
                health.rows_written, health.commits, health.dropped
            ),
        ),
        (
            "Country data".to_owned(),
            match (health.geo_enabled, health.geo_age_secs) {
                (false, _) => "disabled".to_owned(),
                (true, Some(age)) => format!("loaded, {} old", human_age(age)),
                (true, None) => "enabled, database missing".to_owned(),
            },
        ),
        (
            "Uptime".to_owned(),
            uptime
                .map(human_age)
                .unwrap_or_else(|| "unknown".to_owned()),
        ),
    ];

    HealthPage {
        head,
        window,
        windows: windows(window, "/admin/health", ""),
        facts,
    }
}

pub fn human_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;

    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.0} kB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

pub fn human_age(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    if seconds < MINUTE {
        format!("{seconds}s")
    } else if seconds < HOUR {
        format!("{}m", seconds / MINUTE)
    } else if seconds < DAY {
        format!("{}h", seconds / HOUR)
    } else {
        format!("{}d", seconds / DAY)
    }
}

pub fn suspicion_page(head: Head, window: &'static str, rows: &[Suspicion]) -> SuspiciousPage {
    SuspiciousPage {
        head,
        window,
        windows: windows(window, "/admin/suspicious", ""),
        rows: rows
            .iter()
            .map(|row| SuspicionView {
                ip: row.ip.clone(),
                requests: row.requests,
                errors: row.errors,
                limited: row.limited,
                reason: reason_for(row),
            })
            .collect(),
    }
}

pub fn comparison_rows(rows: &[ComparisonRow], origin: &str) -> Vec<ComparisonRowView> {
    rows.iter()
        .map(|row| ComparisonRowView {
            href: format!("{origin}{}", path_from_key(&row.key)),
            label: row.label.clone(),
            mode: row.mode.clone(),
            views: row.views,
            last_seen: relative_time(row.last_seen, crate::clock::now_unix()),
        })
        .collect()
}

/// `title:movie-603,movie-604` → `/title/movie-603,movie-604`.
fn path_from_key(key: &str) -> String {
    match key.split_once(':') {
        Some((mode, picks)) => format!("/{mode}/{picks}"),
        None => "/".to_owned(),
    }
}

pub fn term_rows(terms: &[TrendingTerm]) -> Vec<TermView> {
    terms
        .iter()
        .map(|term| TermView {
            term: term.term.clone(),
            count: term.searches,
        })
        .collect()
}

/// Why a client is on the suspicious list. The scoring already wrote the reasons down; this
/// joins them rather than inventing a second opinion.
pub fn reason_for(value: &Suspicion) -> String {
    if value.reasons.is_empty() {
        return format!("{} requests", value.requests);
    }

    value.reasons.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[i64]) -> Vec<(String, i64)> {
        values
            .iter()
            .enumerate()
            .map(|(index, value)| (format!("2026-11-{index:02}"), *value))
            .collect()
    }

    /// Every point has to land inside the box, whatever the values are.
    #[test]
    fn points_stay_inside_the_box() {
        for values in [vec![0, 0, 0], vec![1, 50, 100], vec![100, 50, 1], vec![7]] {
            let chart = line(&series(&values), 100.0, 50.0);

            for point in chart.points.trim_start_matches('M').split(" L") {
                let (x, y) = point.split_once(',').expect("x,y");
                let x: f64 = x.parse().expect("x");
                let y: f64 = y.parse().expect("y");
                assert!((0.0..=100.0).contains(&x), "x {x} out of box");
                assert!((0.0..=50.0).contains(&y), "y {y} out of box");
            }
        }
    }

    /// A series that never changes must be a horizontal line: the shape is the message, and a
    /// sloping line would say something happened.
    #[test]
    fn a_flat_series_is_drawn_flat() {
        let chart = line(&series(&[12, 12, 12]), 100.0, 50.0);
        let ys: Vec<f64> = chart
            .points
            .trim_start_matches('M')
            .split(" L")
            .map(|point| point.split(',').nth(1).expect("y").parse().expect("number"))
            .collect();

        assert_eq!(ys.len(), 3);
        assert!(
            ys.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "got {ys:?}"
        );
    }

    #[test]
    fn an_empty_series_has_no_path_and_no_labels() {
        let chart = line(&[], 100.0, 50.0);
        assert!(chart.points.is_empty());
        assert!(chart.first_label.is_empty());
        assert_eq!(chart.max, 0);
    }

    #[test]
    fn the_peak_is_the_top_of_the_box() {
        let chart = line(&series(&[1, 10]), 100.0, 50.0);
        assert_eq!(chart.max, 10);
        assert_eq!(chart.last, 10);

        let peak_y: f64 = chart
            .points
            .rsplit(" L")
            .next()
            .expect("point")
            .split(',')
            .nth(1)
            .expect("y")
            .parse()
            .expect("number");
        assert!(
            peak_y < 5.0,
            "the maximum should be near the top, got {peak_y}"
        );
    }

    #[test]
    fn bars_are_relative_to_the_largest_value() {
        let rows = vec![
            PathRow {
                path: "/".into(),
                requests: 10,
            },
            PathRow {
                path: "/x".into(),
                requests: 5,
            },
        ];

        let built = bars(
            &rows,
            |row: &PathRow| row.path.clone(),
            |row| row.requests,
            |_| String::new(),
        );
        assert_eq!(built[0].width, "100%");
        assert_eq!(built[1].width, "50%");
    }

    /// A filter value goes into a `href`; anything that could change the query's meaning is
    /// dropped rather than escaped, because these are addresses and paths, not prose.
    #[test]
    fn a_filter_only_survives_if_it_looks_like_an_address() {
        assert_eq!(filter_url("ip", "1.2.3.4"), "&ip=1.2.3.4");
        assert_eq!(filter_url("ip", "::1"), "&ip=::1");
        assert_eq!(filter_url("ip", "1.2.3.4&admin=1"), "&ip=1.2.3.4admin1");
        assert_eq!(filter_url("ip", ""), "");
        assert_eq!(filter_url("", "x"), "");
    }

    #[test]
    fn sizes_and_ages_read_like_a_person_wrote_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(131_656), "129 kB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");

        assert_eq!(human_age(42), "42s");
        assert_eq!(human_age(3_600), "1h");
        assert_eq!(human_age(2 * 86_400), "2d");
    }

    #[test]
    fn window_links_keep_the_pages_other_filters() {
        let links = windows("7d", "/admin/logs", "&ip=1.2.3.4");
        assert_eq!(links[1].href, "/admin/logs?window=7d&ip=1.2.3.4");
        assert!(links[1].active);
        assert!(!links[0].active);
    }

    #[test]
    fn a_suspicion_row_explains_itself() {
        let entry = Suspicion {
            ip: "10.0.0.1".into(),
            requests: 83,
            errors: 2,
            limited: 0,
            paths: 83,
            sessions: 1,
            score: 90,
            reasons: vec!["83 distinct paths — id walking".into()],
            first_seen: 0,
            last_seen: 0,
        };

        assert_eq!(reason_for(&entry), "83 distinct paths — id walking");
    }

    /// A row with no written-down reason still says something useful rather than nothing.
    #[test]
    fn a_row_without_reasons_falls_back_to_its_volume() {
        let entry = Suspicion {
            ip: "10.0.0.2".into(),
            requests: 12,
            errors: 0,
            limited: 0,
            paths: 3,
            sessions: 1,
            score: 5,
            reasons: Vec::new(),
            first_seen: 0,
            last_seen: 0,
        };

        assert_eq!(reason_for(&entry), "12 requests");
    }
}
