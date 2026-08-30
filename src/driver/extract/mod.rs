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
use mssql_tds::datatypes::column_values::ColumnValues;
use mssql_tds::datatypes::sql_json::SqlJson;
use mssql_tds::datatypes::sql_vector::SqlVector;
use mssql_tds::datatypes::sqldatatypes::TdsDataType;
use mssql_tiberius_bridge::{ColumnType, Row};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

/// Resolve wire types before dispatching extraction.
///
/// The bridge's preview.3 normalizer handles nullable numeric widths but omits
/// the fixed `smallmoney`, `char(n)`, and `binary(n)` wire variants. Keep those
/// corrections local until the pinned bridge can be upgraded deliberately.
pub fn normalized_column_type(tds_type: TdsDataType, byte_length: usize) -> ColumnType {
    match tds_type {
        TdsDataType::Money4 => ColumnType::Money4,
        TdsDataType::BigChar => ColumnType::Char,
        TdsDataType::BigBinary => ColumnType::Binary,
        TdsDataType::DateTimeN if byte_length == 4 => ColumnType::Datetime4,
        other => ColumnType::from_tds_with_length(other, byte_length),
    }
}

/// Extract a single cell using the type exposed by the bridge row.
pub fn extract_value(row: &Row, idx: usize) -> Result<Value, String> {
    let Some(column) = row.columns().get(idx) else {
        return Ok(Value::Null);
    };
    extract_value_as(row, idx, column.column_type())
}

/// Extract a single cell using already-normalized result-set metadata.
///
/// Most decode mismatches retain the driver's historical `null` fallback.
/// Exact numerics are different: silently replacing an out-of-range
/// `decimal(38, s)` with null or an approximate float would corrupt data, so
/// those conversions return an error that aborts the query result instead.
pub fn extract_value_as(row: &Row, idx: usize, column_type: ColumnType) -> Result<Value, String> {
    let value = match column_type {
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

        ColumnType::Money | ColumnType::Money4 | ColumnType::Decimaln | ColumnType::Numericn => {
            return read_numeric_as_string(row, idx)
        }

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
        | ColumnType::Xml => read_string(row, idx),

        ColumnType::Json => match row.raw_value(idx) {
            Some(ColumnValues::Json(json)) => json_to_json(json)?,
            _ => Value::Null,
        },

        // Binary
        ColumnType::Image | ColumnType::Binary | ColumnType::VarBinary | ColumnType::BigVarBin => {
            read_binary_as_base64(row, idx)
        }

        ColumnType::Vector => match row.raw_value(idx) {
            Some(ColumnValues::Vector(vector)) => vector_to_json(vector),
            _ => Value::Null,
        },

        // sql_variant remains a best-effort textual fallback.
        ColumnType::Ssvariant => read_string(row, idx),
    };
    Ok(value)
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

fn read_numeric_as_string(row: &Row, idx: usize) -> Result<Value, String> {
    let value = row
        .raw_value(idx)
        .ok_or_else(|| format!("SQL Server numeric column index {idx} is out of bounds"))?;
    numeric_value_to_json(value)
}

fn numeric_value_to_json(value: &ColumnValues) -> Result<Value, String> {
    let decimal = match value {
        ColumnValues::Null => return Ok(Value::Null),
        ColumnValues::Decimal(parts) | ColumnValues::Numeric(parts) => {
            let raw = parts.to_string();
            raw.parse::<Decimal>().map_err(|error| {
                format!(
                    "SQL Server decimal({}, {}) value {raw} exceeds rust_decimal's exact range: {error}",
                    parts.precision, parts.scale
                )
            })?
        }
        ColumnValues::SmallMoney(money) => Decimal::new(i64::from(money.int_val), 4),
        ColumnValues::Money(money) => {
            let raw = (i64::from(money.msb_part) << 32) | i64::from(money.lsb_part as u32);
            Decimal::new(raw, 4)
        }
        other => {
            return Err(format!(
                "SQL Server returned a non-numeric value for an exact numeric column: {other:?}"
            ))
        }
    };
    Ok(Value::String(normalize_decimal_string(
        &decimal.to_string(),
    )))
}

fn json_to_json(json: &SqlJson) -> Result<Value, String> {
    String::from_utf8(json.bytes.clone())
        .map(Value::String)
        .map_err(|error| format!("SQL Server returned invalid UTF-8 for a JSON column: {error}"))
}

fn vector_to_json(vector: &SqlVector) -> Value {
    Value::Array(
        vector
            .as_f32()
            .unwrap_or_default()
            .iter()
            .map(|dimension| f64_to_json(f64::from(*dimension)))
            .collect(),
    )
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
