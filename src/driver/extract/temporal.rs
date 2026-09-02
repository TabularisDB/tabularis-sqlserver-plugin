//! Pure formatters for SQL Server temporal types.
//!
//! All functions here take a `chrono` value and return a `String`. They do
//! **not** touch the database client — the row-level extraction lives in
//! [`super::extract_value`], which calls into these helpers after pulling the
//! right chrono type out of the row.
//!
//! Format choices mirror the existing Tabularis drivers (MySQL / PostgreSQL)
//! so the UI doesn't have to switch on the source driver:
//!
//! | SQL Server type   | chrono type                    | Output format                     |
//! |-------------------|--------------------------------|-----------------------------------|
//! | `date`            | `NaiveDate`                    | `YYYY-MM-DD`                      |
//! | `time`            | `NaiveTime`                    | `HH:MM:SS` or `HH:MM:SS.fff`      |
//! | `datetime`        | `NaiveDateTime`                | `YYYY-MM-DD HH:MM:SS`             |
//! | `datetime2`       | `NaiveDateTime`                | `YYYY-MM-DD HH:MM:SS` or `.fff`   |
//! | `smalldatetime`   | `NaiveDateTime`                | `YYYY-MM-DD HH:MM:SS`             |
//! | `datetimeoffset`  | `DateTime<FixedOffset>`        | RFC3339 (`YYYY-MM-DDTHH:MM:SS+00:00`) |

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

pub fn format_date(d: &NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Format a `NaiveTime`. Includes a fractional-second suffix (up to 7 digits,
/// SQL Server's resolution for `time(7)`) only when it is non-zero, so
/// `time(0)` columns don't grow an unnecessary `.0000000`.
pub fn format_time(t: &NaiveTime) -> String {
    if t.nanosecond() == 0 {
        t.format("%H:%M:%S").to_string()
    } else {
        let full = t.format("%H:%M:%S%.f").to_string();
        trim_fractional_trailing_zeros(&full)
    }
}

/// Format a `NaiveDateTime`. Same fractional-second policy as [`format_time`].
pub fn format_datetime(dt: &NaiveDateTime) -> String {
    if dt.nanosecond() == 0 {
        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        let full = dt.format("%Y-%m-%d %H:%M:%S%.f").to_string();
        trim_fractional_trailing_zeros(&full)
    }
}

/// Decode SQL Server's legacy DATETIME ticks using its documented display
/// granularity (.000, .003, or .007 seconds). The bridge truncates 1/300
/// second ticks to milliseconds, which incorrectly renders .006 and .996.
pub fn format_legacy_datetime(days: i32, ticks: u32) -> Option<String> {
    let base = NaiveDate::from_ymd_opt(1900, 1, 1)?;
    let date = base.checked_add_signed(chrono::Duration::days(i64::from(days)))?;
    let total_millis = (u64::from(ticks) * 10 + 1) / 3;
    let seconds = u32::try_from(total_millis / 1_000).ok()?;
    let nanos = u32::try_from(total_millis % 1_000).ok()? * 1_000_000;
    let time = NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos)?;
    Some(format_datetime(&NaiveDateTime::new(date, time)))
}

/// Format a `DateTime<FixedOffset>` as RFC3339 with fractional seconds when
/// present. `datetimeoffset` is the only SQL Server temporal type that
/// carries a zone; we keep the zone explicit so round-tripping is safe.
pub fn format_datetime_offset(dt: &DateTime<FixedOffset>) -> String {
    // chrono's `to_rfc3339` keeps fractional seconds if present, and always
    // emits the zone. Both desirable.
    dt.to_rfc3339()
}

/// Remove trailing `0` characters from the fractional-seconds portion of a
/// timestamp string, and drop the `.` entirely if no fraction remains.
///
/// `"12:34:56.5000000"` -> `"12:34:56.5"`, `"12:34:56.0000000"` -> `"12:34:56"`.
fn trim_fractional_trailing_zeros(s: &str) -> String {
    // Find the last '.' after the last ':' (or start) to localise the fraction.
    let Some(dot_idx) = s.rfind('.') else {
        return s.to_string();
    };
    // Ensure the `.` belongs to the fractional-seconds tail (no TZ or space after).
    let frac = &s[dot_idx + 1..];
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return s.to_string();
    }
    let trimmed = frac.trim_end_matches('0');
    if trimmed.is_empty() {
        s[..dot_idx].to_string()
    } else {
        format!("{}.{}", &s[..dot_idx], trimmed)
    }
}

#[cfg(test)]
mod tests;
