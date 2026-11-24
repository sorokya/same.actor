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
pub mod franchise;
pub mod not_found;
pub mod start_here;

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

/// Which island a page needs.
///
/// A page loads what it uses and nothing else: a browse page has no picker, a comparison page has
/// no matrix. The booster carries the difference — when a swap lands on a page whose script this
/// document never ran, it loads it before announcing the swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scripts {
    /// The comparison pages: searching, picking, sharing.
    Picker,
    /// The browse pages: hover, pinning, filter swaps.
    Synergy,
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
    /// The synergy island. Only the browse pages have a matrix to enhance.
    pub synergy: String,
    pub nav: &'static str,
    pub noindex: bool,
    /// Which admin tab is current. Empty on every public page.
    pub active: &'static str,
    /// The wide shell. A comparison page is a column of prose and reads worse wider than 48rem;
    /// a matrix of twenty columns needs the room, and squeezing it into the reading measure is
    /// what made the browse pages feel like they had margins to spare.
    pub wide: bool,
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
            wide: false,
            synergy: String::new(),
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

    /// What a public page loads: the stylesheet and the link booster always, and the island that
    /// page actually uses.
    ///
    /// Not everything everywhere: a browse page has no picker and a comparison page has no
    /// matrix. The booster carries the rest — when a swap lands on a page whose script this
    /// document never ran, it fetches and runs it before announcing the swap (`lib/nav.ts`).
    pub fn with_scripts(mut self, assets: &crate::assets::Assets, scripts: Scripts) -> Self {
        if let Some(css) = assets.css.as_deref() {
            self = self.with_css(css);
        }
        if let Some(boost) = assets.boost.as_deref() {
            self = self.with_boost(boost);
        }

        let island = match scripts {
            Scripts::Picker => assets.island.as_deref(),
            Scripts::Synergy => assets.synergy.as_deref(),
        };

        if let Some(script) = island {
            match scripts {
                Scripts::Picker => self = self.with_island(script),
                Scripts::Synergy => self = self.with_synergy(script),
            }
        }

        self
    }

    /// Ask for the wide shell, header included.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    pub fn with_css(mut self, css: &str) -> Self {
        self.css = css.to_owned();
        self
    }

    pub fn with_island(mut self, island: &str) -> Self {
        self.island = island.to_owned();
        self
    }

    /// Opt into the matrix island: hover highlighting, pinning and filter swaps.
    pub fn with_synergy(mut self, script: &str) -> Self {
        self.synergy = script.to_owned();
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

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-11-24` as `24 Nov 2026`.
///
/// The store keeps ISO dates because they sort and compare; a card should not show one. Anything
/// that is not a well-formed date returns `None` so the caller can fall back to a year rather
/// than print `0000-00-00` at somebody.
pub fn format_date(iso: &str) -> Option<String> {
    let bytes = iso.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }

    let year: i32 = iso[0..4].parse().ok()?;
    let month: usize = iso[5..7].parse().ok()?;
    let day: u32 = iso[8..10].parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1800 {
        return None;
    }

    Some(format!("{day} {} {year}", MONTHS[month - 1]))
}

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

    fn assets() -> crate::assets::Assets {
        crate::assets::Assets {
            css: Some("/assets/island.css".into()),
            island: Some("/assets/island.js".into()),
            boost: Some("/assets/boost.js".into()),
            synergy: Some("/assets/synergy.js".into()),
        }
    }

    /// A page loads the island it uses and not the other one: a browse page has no picker, and a
    /// comparison page has no matrix. The booster is what makes that safe — it fetches the
    /// missing script before announcing a swap (`lib/nav.ts`).
    #[test]
    fn a_page_loads_the_island_it_needs_and_not_the_other() {
        let picker = Head::new("t", "d", "title").with_scripts(&assets(), Scripts::Picker);
        assert_eq!(picker.island, "/assets/island.js");
        assert!(picker.synergy.is_empty(), "a comparison page has no matrix");
        assert!(
            !picker.boost.is_empty(),
            "a picker page still swaps pages in"
        );

        let browse = Head::new("t", "d", "upcoming").with_scripts(&assets(), Scripts::Synergy);
        assert_eq!(browse.synergy, "/assets/synergy.js");
        assert!(browse.island.is_empty(), "a browse page has no picker");
        assert!(!browse.boost.is_empty(), "and so does a browse page");
    }

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

#[cfg(test)]
mod date_tests {
    use super::format_date;

    #[test]
    fn an_iso_date_becomes_something_a_person_reads() {
        assert_eq!(format_date("2026-11-24").as_deref(), Some("24 Nov 2026"));
        assert_eq!(format_date("2027-01-02").as_deref(), Some("2 Jan 2027"));
    }

    /// TMDB sends `0000-00-00` for an unannounced release, and a card that prints it looks
    /// broken. Anything unparseable has to come back empty so the caller can show a year.
    #[test]
    fn a_date_that_is_not_a_date_is_nothing() {
        for bad in [
            "",
            "2026",
            "2026-11",
            "0000-00-00",
            "2026-13-01",
            "2026-11-32",
            "not-a-date",
        ] {
            assert_eq!(format_date(bad), None, "{bad} should not format");
        }
    }
}
