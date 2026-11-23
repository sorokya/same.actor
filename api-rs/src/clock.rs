//! Time, without a date library.
//!
//! Everything stored is unix seconds in UTC, and every day bucket is a UTC day. That is one
//! decision rather than three: no local-offset lookup (which fails in containers with no
//! `/etc/localtime`), no DST hole where a day is 23 or 25 hours, and no dependency for what is
//! arithmetic.
//!
//! The civil-date conversion is Howard Hinnant's `civil_from_days`: shift the epoch to
//! 0000-03-01 so leap days land at the end of a 400-year cycle, then work in eras of 400
//! years. It is exact for every date this program will ever see.

/// Seconds since the unix epoch. A clock before 1970 yields 0 rather than panicking.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(_) => 0,
    }
}

/// Days since the unix epoch, floored. `div_euclid` so a pre-epoch timestamp floors downwards
/// instead of truncating towards zero.
fn days_from_unix(unix: i64) -> i64 {
    unix.div_euclid(86_400)
}

/// `(year, month, day)` for a count of days since 1970-01-01. Months are 1-based.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);

    // March-based month: 0..=364 maps onto Mar..Feb.
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };

    if month <= 2 {
        year += 1;
    }

    (year, month as u32, day)
}

/// `YYYY-MM-DD` in UTC, for a day bucket.
pub fn day_string(unix: i64) -> String {
    let (year, month, day) = civil_from_days(days_from_unix(unix));
    format!("{year:04}-{month:02}-{day:02}")
}

/// `YYYY-MM` in UTC. DB-IP publishes one file per month.
pub fn month_string(unix: i64) -> String {
    let (year, month, _) = civil_from_days(days_from_unix(unix));
    format!("{year:04}-{month:02}")
}

/// Unix seconds at the start of the UTC day containing `unix`.
pub fn day_start(unix: i64) -> i64 {
    days_from_unix(unix) * 86_400
}

/// Unix seconds at the start of the UTC day `days` days before `unix`.
pub fn day_start_days_ago(unix: i64, days: i64) -> i64 {
    day_start(unix) - days * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(unix: i64) -> (i64, u32, u32) {
        let (year, month, day) = civil_from_days(days_from_unix(unix));
        (year, month, day)
    }

    #[test]
    fn the_epoch_is_1970_01_01() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(month_string(0), "1970-01");
    }

    #[test]
    fn known_dates_convert() {
        // 2000-03-01 00:00:00 UTC — a leap day the day before, which is where naive
        // conversions go wrong.
        assert_eq!(day_string(951_868_800), "2000-03-01");

        // 2024-02-29 23:59:59 UTC — a leap day.
        assert_eq!(day_string(1_709_251_199), "2024-02-29");

        // 2026-11-23 12:00:00 UTC.
        assert_eq!(day_string(1_795_435_200), "2026-11-23");
        assert_eq!(month_string(1_795_435_200), "2026-11");
    }

    #[test]
    fn midnight_is_the_start_of_its_own_day() {
        let midnight = 1_795_392_000; // 2026-11-23 00:00:00 UTC
        assert_eq!(day_start(midnight), midnight);
        assert_eq!(day_string(midnight), "2026-11-23");
        assert_eq!(day_start(midnight + 86_399), midnight);
        assert_eq!(day_start(midnight - 1), midnight - 86_400);
    }

    #[test]
    fn every_day_of_a_leap_year_is_distinct_and_in_order() {
        let start = 1_704_067_200; // 2024-01-01
        let mut seen = Vec::new();

        for day in 0..366 {
            let name = day_string(start + day * 86_400);
            assert_eq!(name.len(), 10, "malformed day {name}");
            seen.push(name);
        }

        assert_eq!(seen[0], "2024-01-01");
        assert_eq!(seen[59], "2024-02-29", "the leap day is missing");
        assert_eq!(seen[365], "2024-12-31");

        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 366, "a day repeated");
        assert_eq!(sorted, seen, "days are out of order");
    }

    #[test]
    fn pre_epoch_timestamps_floor_downwards() {
        // -1 is 1969-12-31 23:59:59, so its day bucket is 1969-12-31, not the epoch.
        assert_eq!(day_string(-1), "1969-12-31");
        assert_eq!(parts(-1), (1969, 12, 31));
        assert_eq!(day_start(-1), -86_400);
    }

    #[test]
    fn a_day_days_ago_stays_on_a_midnight() {
        let noon = 1_795_435_200;
        let week = day_start_days_ago(noon, 7);
        assert_eq!(week % 86_400, 0);
        assert_eq!(day_string(week), "2026-11-16");
    }
}
