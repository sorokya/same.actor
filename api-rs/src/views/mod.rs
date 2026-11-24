//! Server-rendered pages.
//!
//! One `#[derive(Template)]` struct per page, in its own module. Everything a page needs is a
//! plain field: no globals, no `Option` juggling inside the template, and the shape of the data
//! is checked at compile time.
//!
//! The alternative — rendering JSON on the client — is what this replaced. A share link now
//! paints its content before a byte of JavaScript runs.

pub mod admin;
pub mod compare;
pub mod discover;
pub mod not_found;

use askama::Template;
use axum::response::Html;

use crate::error::AppError;

/// Render a template, turning a failure into a 500 rather than a panic.
///
/// Askama checks templates at compile time, so this can only fail if a writer fails or a filter
/// rejects a runtime value. It is still fallible and still has to be handled.
pub fn render<T: Template>(template: &T) -> Result<Html<String>, AppError> {
    template
        .render()
        .map(Html)
        .map_err(|error| AppError::Internal(format!("could not render a page: {error}")))
}

/// Everything the shared layout needs.
///
/// `nav` is the section key the header marks as current: `title`, `person`, `top`, `new`,
/// `upcoming`, `admin`, or empty for a page that belongs to none of them.
pub struct Head {
    pub title: String,
    pub description: String,
    /// Absolute URL, or empty. Empty drops the canonical and `og:url` tags.
    pub canonical: String,
    /// Absolute URL of the share card, or empty. Empty falls back to `twitter:card=summary`.
    pub image: String,
    pub css: String,
    /// The picker island, for the pages that have one.
    pub island: String,
    /// The link booster. Empty on the dashboard, which stays free of scripts.
    pub boost: String,
    pub nav: &'static str,
    pub noindex: bool,
    /// Which admin tab is current. Empty on every public page.
    pub active: &'static str,
    /// Copyright year, resolved once at render time instead of in every footer.
    pub year: i32,
}

impl Head {
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        nav: &'static str,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            canonical: String::new(),
            image: String::new(),
            css: String::new(),
            island: String::new(),
            boost: String::new(),
            nav,
            active: "",
            noindex: false,
            year: current_year(),
        }
    }

    pub fn with_css(mut self, css: &str) -> Self {
        self.css = css.to_owned();
        self
    }

    pub fn with_island(mut self, island: &str) -> Self {
        self.island = island.to_owned();
        self
    }

    /// Opt into instant navigation. Every public page does; the dashboard does not.
    pub fn with_boost(mut self, boost: &str) -> Self {
        self.boost = boost.to_owned();
        self
    }

    pub fn with_canonical(mut self, canonical: impl Into<String>) -> Self {
        self.canonical = canonical.into();
        self
    }

    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    pub fn with_active(mut self, active: &'static str) -> Self {
        self.active = active;
        self
    }

    pub fn noindex(mut self) -> Self {
        self.noindex = true;
        self
    }
}

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// `just now`, `12 minutes ago`, `3 hours ago`, `2 days ago`, then a date.
///
/// A refresh timestamp rendered as `2026-11-24 00:41:07` reads like log output. The exact moment
/// still goes in a `datetime`/`title`; this is the sentence a reader wants.
pub fn relative_time(unix: i64, now: i64) -> String {
    let seconds = (now - unix).max(0);

    if seconds < 45 {
        "just now".to_owned()
    } else if seconds < HOUR {
        let minutes = seconds / MINUTE;
        format!("{minutes} minute{} ago", plural(minutes))
    } else if seconds < DAY {
        let hours = seconds / HOUR;
        format!("{hours} hour{} ago", plural(hours))
    } else if seconds < 7 * DAY {
        let days = seconds / DAY;
        format!("{days} day{} ago", plural(days))
    } else {
        absolute_time(unix)
    }
}

/// `2026-11-24 00:41`, in UTC. Used for `title` attributes and admin tables.
pub fn absolute_time(unix: i64) -> String {
    let (days, seconds) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let date = crate::discovery::civil_from_days(days);
    format!(
        "{date} {:02}:{:02}",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

/// The UTC year right now, from the same calendar arithmetic the discovery dates use.
fn current_year() -> i32 {
    let days = crate::clock::now_unix().div_euclid(86_400);
    crate::discovery::civil_from_days(days)[..4]
        .parse()
        .unwrap_or(2026)
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_500_000;

    #[test]
    fn quiet_times_say_so() {
        assert_eq!(relative_time(NOW - 5, NOW), "just now");
        assert_eq!(relative_time(NOW - 44, NOW), "just now");
        assert_eq!(relative_time(NOW - 60, NOW), "1 minute ago");
        assert_eq!(relative_time(NOW - 120, NOW), "2 minutes ago");
        assert_eq!(relative_time(NOW - 7_200, NOW), "2 hours ago");
        assert_eq!(relative_time(NOW - 86_400, NOW), "1 day ago");
        assert_eq!(relative_time(NOW - 3 * 86_400, NOW), "3 days ago");
    }

    /// A timestamp from the future (a clock skew between two writers) reads as now, not as a
    /// negative age.
    #[test]
    fn a_future_timestamp_is_not_a_negative_age() {
        assert_eq!(relative_time(NOW + 600, NOW), "just now");
    }

    #[test]
    fn old_timestamps_become_dates() {
        let old = relative_time(NOW - 30 * 86_400, NOW);
        assert_eq!(old.len(), 16, "YYYY-MM-DD HH:MM, got {old}");
        assert!(old.starts_with("20"));
    }

    #[test]
    fn absolute_time_is_a_sortable_utc_stamp() {
        // 2026-11-24T06:01:40Z, checked against `datetime` rather than by hand.
        assert_eq!(absolute_time(1_795_500_100), "2026-11-24 06:01");
        assert_eq!(absolute_time(0), "1970-01-01 00:00");
    }
}
