//! Row-level value extraction for SQL Server.
//!
//! The dispatcher inspects the column's `ColumnType` (provided by the client)
//! and calls `Row::try_get::<T, _>(idx)` with the right `T`. Conversions that
//! need string formatting (dates, decimals, UUIDs, binary) are delegated to
//! pure helpers in sibling modules so they stay unit-testable without a live
//! server.

pub mod temporal;

use crate::common::i64_to_json;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use mssql_tiberius_bridge::{ColumnType, Row};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

/// Extract a single cell into the Tabularis wire-level `serde_json::Value`.
///
/// Returns `Value::Null` for:
/// - NULL SQL values
/// - columns whose `ColumnType` is `Null` (untyped)
/// - values that couldn't be decoded as any expected type
///
/// The function never panics; decoding errors log at debug level and fall
/// back to `Value::Null` so one malformed row doesn't break the whole query.
pub fn extract_value(row: &Row, idx: usize) -> Value {
    let Some(col) = row.columns().get(idx) else {
        return Value::Null;
    };
    let ct = col.column_type();

    match ct {
        ColumnType::Null => Value::Null,

        ColumnType::Bit => read_bool(row, idx),

        ColumnType::Int1 => match row.try_get::<u8, _>(idx) {
            Ok(Some(v)) => Value::from(v),
            _ => Value::Null,
        },
        ColumnType::Int2 => match row.try_get::<i16, _>(idx) {
            Ok(Some(v)) => Value::from(v),
            _ => Value::Null,
        },
        ColumnType::Int4 => match row.try_get::<i32, _>(idx) {
            Ok(Some(v)) => Value::from(v),
            _ => Value::Null,
        },
        ColumnType::Int8 => match row.try_get::<i64, _>(idx) {
            Ok(Some(v)) => i64_to_json(v),
            _ => Value::Null,
        },
        ColumnType::Float4 => match row.try_get::<f32, _>(idx) {
            Ok(Some(v)) => f64_to_json(v as f64),
            _ => Value::Null,
        },
        ColumnType::Float8 => match row.try_get::<f64, _>(idx) {
            Ok(Some(v)) => f64_to_json(v),
            _ => Value::Null,
        },

        ColumnType::Money | ColumnType::Money4 => read_numeric_as_string(row, idx),
        ColumnType::Decimaln | ColumnType::Numericn => read_numeric_as_string(row, idx),

        ColumnType::Guid => match row.try_get::<Uuid, _>(idx) {
            Ok(Some(u)) => Value::String(u.to_string()),
            _ => Value::Null,
        },

        // Temporal
        ColumnType::Datetime | ColumnType::Datetime4 => {
            match row.try_get::<NaiveDateTime, _>(idx) {
                Ok(Some(v)) => Value::String(temporal::format_datetime(&v)),
                _ => Value::Null,
            }
        }
        ColumnType::Datetime2 => match row.try_get::<NaiveDateTime, _>(idx) {
            Ok(Some(v)) => Value::String(temporal::format_datetime(&v)),
            _ => Value::Null,
        },
        ColumnType::DatetimeOffset => match row.try_get::<DateTime<FixedOffset>, _>(idx) {
            Ok(Some(v)) => Value::String(temporal::format_datetime_offset(&v)),
            _ => Value::Null,
        },
        ColumnType::Date => match row.try_get::<NaiveDate, _>(idx) {
            Ok(Some(v)) => Value::String(temporal::format_date(&v)),
            _ => Value::Null,
        },
        ColumnType::Time => match row.try_get::<NaiveTime, _>(idx) {
            Ok(Some(v)) => Value::String(temporal::format_time(&v)),
            _ => Value::Null,
        },

        // Strings
        ColumnType::Text
        | ColumnType::NText
        | ColumnType::Varchar
        | ColumnType::Char
        | ColumnType::NVarchar
        | ColumnType::NChar
        | ColumnType::Xml
        | ColumnType::Json => read_string(row, idx),

        // Binary
        ColumnType::Image | ColumnType::Binary | ColumnType::VarBinary | ColumnType::BigVarBin => {
            read_binary_as_base64(row, idx)
        }

        // Fallbacks: sql_variant and vector → best-effort string
        ColumnType::Ssvariant | ColumnType::Vector => read_string(row, idx),
    }
}

// --- Primitive readers ---------------------------------------------------

fn read_bool(row: &Row, idx: usize) -> Value {
    match row.try_get::<bool, _>(idx) {
        Ok(Some(b)) => Value::Bool(b),
        _ => Value::Null,
    }
}

fn read_string(row: &Row, idx: usize) -> Value {
    match row.try_get::<&str, _>(idx) {
        Ok(Some(s)) => Value::String(s.to_string()),
        _ => Value::Null,
    }
}

fn read_numeric_as_string(row: &Row, idx: usize) -> Value {
    // Prefer `rust_decimal::Decimal` (exact); fall back to `f64` for values
    // that fail to decode as a decimal.
    if let Ok(Some(d)) = row.try_get::<Decimal, _>(idx) {
        return Value::String(normalize_decimal_string(&d.to_string()));
    }
    if let Ok(Some(f)) = row.try_get::<f64, _>(idx) {
        return f64_to_json(f);
    }
    Value::Null
}

fn read_binary_as_base64(row: &Row, idx: usize) -> Value {
    use base64::Engine as _;
    match row.try_get::<&[u8], _>(idx) {
        Ok(Some(bytes)) => Value::String(format!(
            "base64:{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )),
        _ => Value::Null,
    }
}

// --- Pure helpers (testable) ---------------------------------------------

/// Convert a `f64` to a JSON number, falling back to string for non-finite
/// values (NaN / ±Inf are not valid JSON numbers).
pub fn f64_to_json(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(v.to_string()))
}

/// Normalise a decimal string representation by trimming insignificant
/// trailing zeros after the decimal point. `"3.1400"` -> `"3.14"`,
/// `"10.0"` -> `"10"`. Leaves integers without a dot alone.
pub fn normalize_decimal_string(raw: &str) -> String {
    // Preserve sign + whole/fractional split.
    if !raw.contains('.') {
        return raw.to_string();
    }
    let trimmed = raw.trim_end_matches('0');
    let trimmed = trimmed.trim_end_matches('.');
    trimmed.to_string()
}

#[cfg(test)]
mod tests;
