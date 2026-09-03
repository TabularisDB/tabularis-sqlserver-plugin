use super::*;
use mssql_tds::datatypes::column_values::{SqlMoney, SqlSmallMoney};
use mssql_tiberius_bridge::DecimalParts;

#[test]
fn column_type_normalization_resolves_nullable_widths() {
    let cases = [
        (TdsDataType::IntN, 1, ColumnType::Int1),
        (TdsDataType::IntN, 2, ColumnType::Int2),
        (TdsDataType::IntN, 4, ColumnType::Int4),
        (TdsDataType::IntN, 8, ColumnType::Int8),
        (TdsDataType::FltN, 4, ColumnType::Float4),
        (TdsDataType::FltN, 8, ColumnType::Float8),
        (TdsDataType::MoneyN, 4, ColumnType::Money4),
        (TdsDataType::MoneyN, 8, ColumnType::Money),
        (TdsDataType::DateTimeN, 4, ColumnType::Datetime4),
        (TdsDataType::DateTimeN, 8, ColumnType::Datetime),
    ];

    for (tds_type, length, expected) in cases {
        assert_eq!(normalized_column_type(tds_type, length), expected);
    }
}

#[test]
fn column_type_normalization_matches_the_replaced_tiberius_dispatch() {
    let cases = [
        (TdsDataType::Void, ColumnType::Null),
        (TdsDataType::Bit, ColumnType::Bit),
        (TdsDataType::Int1, ColumnType::Int1),
        (TdsDataType::Int2, ColumnType::Int2),
        (TdsDataType::Int4, ColumnType::Int4),
        (TdsDataType::Int8, ColumnType::Int8),
        (TdsDataType::Flt4, ColumnType::Float4),
        (TdsDataType::Flt8, ColumnType::Float8),
        (TdsDataType::DateTime, ColumnType::Datetime),
        (TdsDataType::DateTime2N, ColumnType::Datetime2),
        (TdsDataType::DateTim4, ColumnType::Datetime4),
        (TdsDataType::DateTimeOffsetN, ColumnType::DatetimeOffset),
        (TdsDataType::DateN, ColumnType::Date),
        (TdsDataType::TimeN, ColumnType::Time),
        (TdsDataType::Decimal, ColumnType::Decimaln),
        (TdsDataType::DecimalN, ColumnType::Decimaln),
        (TdsDataType::Numeric, ColumnType::Numericn),
        (TdsDataType::NumericN, ColumnType::Numericn),
        (TdsDataType::Money, ColumnType::Money),
        (TdsDataType::Guid, ColumnType::Guid),
        (TdsDataType::NVarChar, ColumnType::NVarchar),
        (TdsDataType::VarChar, ColumnType::Varchar),
        (TdsDataType::NChar, ColumnType::NChar),
        (TdsDataType::Char, ColumnType::Char),
        (TdsDataType::NText, ColumnType::NText),
        (TdsDataType::Text, ColumnType::Text),
        (TdsDataType::Binary, ColumnType::Binary),
        (TdsDataType::VarBinary, ColumnType::VarBinary),
        (TdsDataType::Image, ColumnType::Image),
        (TdsDataType::Xml, ColumnType::Xml),
        (TdsDataType::SsVariant, ColumnType::Ssvariant),
        // The bridge has no UDT ColumnType, but it decodes CLR payloads as
        // bytes, so the plugin exposes them losslessly as BLOB values.
        (TdsDataType::Udt, ColumnType::BigVarBin),
        (TdsDataType::None, ColumnType::Null),
    ];

    for (tds_type, expected) in cases {
        assert_eq!(normalized_column_type(tds_type, 8), expected);
    }
}

#[test]
fn column_type_normalization_covers_fixed_and_big_wire_names() {
    let cases = [
        (TdsDataType::BitN, ColumnType::Bit),
        (TdsDataType::Money4, ColumnType::Money4),
        (TdsDataType::BigVarChar, ColumnType::Varchar),
        (TdsDataType::BigChar, ColumnType::Char),
        (TdsDataType::BigVarBinary, ColumnType::VarBinary),
        (TdsDataType::BigBinary, ColumnType::Binary),
        (TdsDataType::Json, ColumnType::Json),
        (TdsDataType::Vector, ColumnType::Vector),
    ];

    for (tds_type, expected) in cases {
        assert_eq!(normalized_column_type(tds_type, 8), expected);
    }
}

#[test]
fn exact_decimal_is_preserved() {
    let parts = DecimalParts::from_string("123.4500", 10, 4).unwrap();
    let value = numeric_value_to_json(&ColumnValues::Decimal(parts)).unwrap();

    assert_eq!(value, Value::String("123.45".into()));
}

#[test]
fn decimal_38_preserves_every_digit() {
    let parts = DecimalParts::from_string("99999999999999999999999999999999999999", 38, 0).unwrap();
    let value = numeric_value_to_json(&ColumnValues::Numeric(parts)).unwrap();

    assert_eq!(
        value,
        Value::String("99999999999999999999999999999999999999".into())
    );
}

#[test]
fn money_and_smallmoney_are_exact_decimal_strings() {
    let smallmoney = numeric_value_to_json(&ColumnValues::SmallMoney(SqlSmallMoney {
        int_val: -123_456,
    }))
    .unwrap();
    let money = numeric_value_to_json(&ColumnValues::Money(SqlMoney {
        msb_part: 0,
        lsb_part: 1_234_567,
    }))
    .unwrap();

    assert_eq!(smallmoney, Value::String("-12.3456".into()));
    assert_eq!(money, Value::String("123.4567".into()));
}

#[test]
fn binary_values_use_the_host_blob_wire_format() {
    assert_eq!(
        binary_to_json(&[0xde, 0xad, 0xbe, 0xef]),
        Value::String("BLOB:4:application/octet-stream:3q2+7w==".into())
    );
}

#[test]
fn json_values_have_a_defined_text_representation() {
    let json = SqlJson::new(br#"{"ok":true}"#.to_vec());

    assert_eq!(
        json_to_json(&json).unwrap(),
        Value::String(r#"{"ok":true}"#.into())
    );
}

#[test]
fn malformed_json_utf8_fails_loudly() {
    let json = SqlJson::new(vec![0xff]);
    let error = json_to_json(&json).unwrap_err();

    assert!(error.contains("invalid UTF-8"));
}

#[test]
fn vector_values_have_a_defined_json_array_representation() {
    let vector = SqlVector::try_from_f32(vec![1.0, -2.5, 3.25]).unwrap();
    let value = vector_to_json(&vector);

    assert_eq!(value, serde_json::json!([1.0, -2.5, 3.25]));
}

// --- f64_to_json ------------------------------------------------------

#[test]
fn f64_to_json_wraps_finite_numbers() {
    let v = f64_to_json(2.75);
    assert_eq!(
        v,
        Value::Number(serde_json::Number::from_f64(2.75).unwrap())
    );
    let v = f64_to_json(0.0);
    assert_eq!(v, Value::Number(serde_json::Number::from_f64(0.0).unwrap()));
    let v = f64_to_json(-1e10);
    assert_eq!(
        v,
        Value::Number(serde_json::Number::from_f64(-1e10).unwrap())
    );
}

#[test]
fn f64_to_json_stringifies_nan_and_infinity() {
    assert_eq!(f64_to_json(f64::NAN), Value::String("NaN".into()));
    assert_eq!(f64_to_json(f64::INFINITY), Value::String("inf".into()));
    assert_eq!(f64_to_json(f64::NEG_INFINITY), Value::String("-inf".into()));
}

// --- normalize_decimal_string -----------------------------------------

#[test]
fn normalize_decimal_trims_trailing_zeros() {
    assert_eq!(normalize_decimal_string("2.7500"), "2.75");
    assert_eq!(normalize_decimal_string("3.100"), "3.1");
    assert_eq!(normalize_decimal_string("0.50"), "0.5");
}

#[test]
fn normalize_decimal_drops_trailing_dot() {
    assert_eq!(normalize_decimal_string("10.0"), "10");
    assert_eq!(normalize_decimal_string("100.000"), "100");
    assert_eq!(normalize_decimal_string("-42.000"), "-42");
}

#[test]
fn normalize_decimal_leaves_integers_alone() {
    assert_eq!(normalize_decimal_string("10"), "10");
    assert_eq!(normalize_decimal_string("-42"), "-42");
    assert_eq!(normalize_decimal_string("0"), "0");
}

#[test]
fn normalize_decimal_preserves_significant_digits() {
    assert_eq!(normalize_decimal_string("2.75159"), "2.75159");
    assert_eq!(normalize_decimal_string("0.00001"), "0.00001");
    assert_eq!(normalize_decimal_string("1.23"), "1.23");
}

#[test]
fn normalize_decimal_handles_zero_with_fraction() {
    assert_eq!(normalize_decimal_string("0.0"), "0");
    assert_eq!(normalize_decimal_string("0.000"), "0");
}
