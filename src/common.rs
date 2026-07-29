//! Shared query-classification and JS-safe integer helpers.

use serde_json::Value as JsonValue;

/// Largest integer that round-trips exactly through a JavaScript `number`
/// (IEEE 754 double). Equal to `Number.MAX_SAFE_INTEGER` (`2^53 - 1`).
pub const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Serialize an `i64` as a JSON number when it fits in JavaScript's safe
/// integer range, and as a JSON string otherwise.
///
/// The frontend uses the standard `JSON.parse`, which loses precision for
/// integers outside ±(2^53 - 1). Returning a string for out-of-range values
/// keeps the exact decimal representation intact while leaving small ids,
/// counts and timestamps as ordinary numbers.
#[inline]
pub fn i64_to_json(value: i64) -> JsonValue {
    if !(-JS_MAX_SAFE_INTEGER..=JS_MAX_SAFE_INTEGER).contains(&value) {
        JsonValue::String(value.to_string())
    } else {
        JsonValue::from(value)
    }
}

/// Strip leading SQL comments (`-- …` line comments and `/* … */` block
/// comments) and whitespace so the first statement keyword is at position 0.
pub fn strip_leading_sql_comments(query: &str) -> &str {
    let mut s = query;
    loop {
        s = s.trim_start();
        if s.starts_with("--") {
            match s.find('\n') {
                Some(pos) => s = &s[pos + 1..],
                None => return "",
            }
        } else if s.starts_with("/*") {
            match s.find("*/") {
                Some(pos) => s = &s[pos + 2..],
                None => return "",
            }
        } else {
            break;
        }
    }
    s
}

/// Returns true if a statement's leading keyword produces a row stream.
///
/// Used to pick between the fetch-rows path and the
/// execute-and-collect-affected-rows path so INSERT/UPDATE/DELETE no
/// longer hardcode `affected_rows: 0`.
pub fn returns_result_set(query: &str) -> bool {
    let head = strip_leading_sql_comments(query)
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
        .to_uppercase();
    matches!(
        head.as_str(),
        "SELECT"
            | "WITH"
            | "SHOW"
            | "EXPLAIN"
            | "DESCRIBE"
            | "DESC"
            | "VALUES"
            | "TABLE"
            | "PRAGMA"
            | "CALL"
    )
}
