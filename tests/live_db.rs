//! Live SQL Server integration coverage for the plugin's JSON-RPC boundary.
//!
//! Every call in this file is sent to the compiled plugin over stdin/stdout,
//! exactly as Tabularis sends it. The suite creates its own database, schema,
//! and scratch tables, so it does not depend on `just seed-sqlserver`.
//!
//! Run against the container started by `just run-sqlserver`:
//!
//! ```bash
//! cargo test --test live_db -- --test-threads=1
//! ```
//!
//! `SQLSERVER_TEST_HOST`, `SQLSERVER_TEST_PORT`, `SQLSERVER_TEST_USER`,
//! `SQLSERVER_TEST_PASSWORD`, `SQLSERVER_TEST_DATABASE`, and
//! `SQLSERVER_PLUGIN_BIN` override the local defaults.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use base64::Engine as _;
use serde_json::{json, Value};

const TEST_SCHEMA: &str = "ss003";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn test_database() -> String {
    env_or("SQLSERVER_TEST_DATABASE", "tabularis_test")
}

fn connection_params_for(database: &str, connection_id: &str) -> Value {
    json!({
        "driver": "sqlserver",
        "host": env_or("SQLSERVER_TEST_HOST", "127.0.0.1"),
        "port": env_or("SQLSERVER_TEST_PORT", "1433")
            .parse::<u16>()
            .expect("SQLSERVER_TEST_PORT must be a valid port"),
        "username": env_or("SQLSERVER_TEST_USER", "sa"),
        "password": env_or("SQLSERVER_TEST_PASSWORD", "Str0ng!Passw0rd"),
        "database": database,
        "ssl_mode": "require",
        "connection_id": connection_id,
    })
}

fn connection_params() -> Value {
    connection_params_for(&test_database(), "ss003-live")
}

fn bracket_quote(identifier: &str) -> String {
    format!("[{}]", identifier.replace(']', "]]"))
}

fn string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn url_encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn brace_connection_value(value: &str) -> String {
    format!("{{{}}}", value.replace('}', "}}"))
}

/// A running plugin process driven through real newline-delimited JSON-RPC.
struct Plugin {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Plugin {
    fn spawn() -> Self {
        let bin = std::env::var("SQLSERVER_PLUGIN_BIN")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_sqlserver-plugin").to_string());
        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn plugin binary");
        let stdin = child.stdin.take().expect("plugin stdin was not piped");
        let stdout = BufReader::new(child.stdout.take().expect("plugin stdout was not piped"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn with_scratch_database() -> Self {
        let mut plugin = Self::spawn();
        let database = test_database();

        if !database.eq_ignore_ascii_case("master") {
            let create_database = format!(
                "IF DB_ID(N'{}') IS NULL EXEC(N'CREATE DATABASE {}')",
                string_literal(&database),
                bracket_quote(&database),
            );
            plugin.call_ok(
                "execute_query",
                json!({
                    "params": connection_params_for("master", "ss003-master"),
                    "query": create_database,
                }),
            );
        }

        plugin.call_ok(
            "execute_query",
            json!({
                "params": connection_params(),
                "query": format!(
                    "IF SCHEMA_ID(N'{TEST_SCHEMA}') IS NULL EXEC(N'CREATE SCHEMA [{TEST_SCHEMA}]')"
                ),
            }),
        );
        plugin
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });
        let mut line = serde_json::to_string(&request).expect("serialize JSON-RPC request");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("write request to plugin stdin");
        self.stdin.flush().expect("flush plugin stdin");
        id
    }

    fn read_response(&mut self) -> Value {
        let mut response_line = String::new();
        self.stdout
            .read_line(&mut response_line)
            .expect("read response from plugin stdout");
        assert!(
            !response_line.is_empty(),
            "plugin exited without a response"
        );
        serde_json::from_str(response_line.trim()).expect("parse JSON-RPC response")
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let response = self.read_response();
        assert_eq!(
            response.get("id").and_then(Value::as_u64),
            Some(id),
            "response id must match its request"
        );
        response
    }

    fn call_ok(&mut self, method: &str, params: Value) -> Value {
        let response = self.call(method, params);
        assert!(
            response.get("error").is_none(),
            "{method} returned an error: {:?}",
            response.get("error")
        );
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} returned neither result nor error"))
    }

    fn call_error(&mut self, method: &str, params: Value) -> String {
        let response = self.call(method, params);
        response
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{method} unexpectedly succeeded: {response}"))
            .to_string()
    }

    fn execute_with(&mut self, params: &Value, query: impl Into<String>) -> Value {
        self.call_ok(
            "execute_query",
            json!({ "params": params, "query": query.into() }),
        )
    }

    fn execute(&mut self, query: impl Into<String>) -> Value {
        self.execute_with(&connection_params(), query)
    }

    fn reset_table(&mut self, table: &str, definition: &str) {
        self.execute(format!(
            "DROP TABLE IF EXISTS [{TEST_SCHEMA}].[{table}]; \
             CREATE TABLE [{TEST_SCHEMA}].[{table}] ({definition})"
        ));
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn result_rows(result: &Value) -> &Vec<Value> {
    result
        .get("rows")
        .and_then(Value::as_array)
        .expect("query result must contain a rows array")
}

fn blob_wire(bytes: &[u8]) -> Value {
    json!(format!(
        "BLOB:{}:application/octet-stream:{}",
        bytes.len(),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn raw_sql(expression: &str) -> Value {
    json!({ "value": expression, "is_raw": true })
}

#[derive(Clone)]
enum ExpectedCell {
    Exact(Value),
    Approx(f64),
    Blob { exact_size: Option<usize> },
}

struct TypeCase {
    advertised_name: &'static str,
    ddl: &'static str,
    insert: Value,
    inserted: ExpectedCell,
    boundary: Value,
    bounded: ExpectedCell,
    semantic_check: Option<(&'static str, Value)>,
}

impl TypeCase {
    fn exact(
        advertised_name: &'static str,
        ddl: &'static str,
        insert: Value,
        inserted: Value,
        boundary: Value,
        bounded: Value,
    ) -> Self {
        Self {
            advertised_name,
            ddl,
            insert,
            inserted: ExpectedCell::Exact(inserted),
            boundary,
            bounded: ExpectedCell::Exact(bounded),
            semantic_check: None,
        }
    }
}

fn assert_cell(case: &TypeCase, label: &str, actual: &Value, expected: &ExpectedCell) {
    assert_ne!(
        actual,
        &Value::Null,
        "{} {label} silently decoded as null",
        case.advertised_name
    );
    match expected {
        ExpectedCell::Exact(expected) => assert_eq!(
            actual, expected,
            "{} {label} representation",
            case.advertised_name
        ),
        ExpectedCell::Approx(expected) => {
            let actual = actual
                .as_f64()
                .unwrap_or_else(|| panic!("{} {label} must be numeric", case.advertised_name));
            let relative_error = ((actual - expected) / expected).abs();
            assert!(
                relative_error <= f64::from(f32::EPSILON),
                "{} {label}: expected approximately {expected}, got {actual}",
                case.advertised_name
            );
        }
        ExpectedCell::Blob { exact_size } => {
            let wire = actual.as_str().unwrap_or_else(|| {
                panic!("{} {label} must be a BLOB string", case.advertised_name)
            });
            let mut fields = wire.splitn(4, ':');
            assert_eq!(
                fields.next(),
                Some("BLOB"),
                "{} {label}",
                case.advertised_name
            );
            let size = fields
                .next()
                .and_then(|size| size.parse::<usize>().ok())
                .unwrap_or_else(|| {
                    panic!("{} {label} has invalid BLOB size", case.advertised_name)
                });
            assert!(
                size > 0,
                "{} {label} BLOB must not be empty",
                case.advertised_name
            );
            assert_eq!(
                fields.next(),
                Some("application/octet-stream"),
                "{} {label} MIME type",
                case.advertised_name
            );
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(fields.next().expect("BLOB payload"))
                .expect("BLOB base64");
            assert_eq!(
                decoded.len(),
                size,
                "{} {label} byte count",
                case.advertised_name
            );
            if let Some(expected_size) = exact_size {
                assert_eq!(
                    size, *expected_size,
                    "{} {label} size",
                    case.advertised_name
                );
            }
        }
    }
}

fn generated_create_table_sql(
    plugin: &mut Plugin,
    table_name: &str,
    columns: Value,
) -> Vec<String> {
    plugin
        .call_ok(
            "get_create_table_sql",
            json!({ "table_name": table_name, "schema": TEST_SCHEMA, "columns": columns }),
        )
        .as_array()
        .expect("DDL result must be an array")
        .iter()
        .map(|statement| {
            statement
                .as_str()
                .expect("DDL statement must be a string")
                .to_string()
        })
        .collect()
}

#[test]
fn test_connection_and_ping_succeed_with_tls_required() {
    let mut plugin = Plugin::with_scratch_database();
    let test_result = plugin.call_ok("test_connection", json!({ "params": connection_params() }));
    assert_eq!(test_result, json!({ "success": true }));

    let ping_result = plugin.call_ok("ping", json!({ "params": connection_params() }));
    assert_eq!(ping_result, Value::Null);
}

#[test]
fn ddl_creates_identity_composite_and_all_data_type_categories() {
    let mut plugin = Plugin::with_scratch_database();
    for table in ["ddl_identity", "ddl_composite", "ddl_categories"] {
        plugin.execute(format!("DROP TABLE IF EXISTS [{TEST_SCHEMA}].[{table}]"));
    }

    let identity_sql = generated_create_table_sql(
        &mut plugin,
        "ddl_identity",
        json!([
            {
                "name": "id", "data_type": "INT", "is_nullable": false,
                "is_pk": true, "is_auto_increment": true, "default_value": null
            },
            {
                "name": "name", "data_type": "NVARCHAR(100)", "is_nullable": false,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            }
        ]),
    );
    for statement in identity_sql {
        plugin.execute(statement);
    }

    let composite_sql = generated_create_table_sql(
        &mut plugin,
        "ddl_composite",
        json!([
            {
                "name": "tenant_id", "data_type": "INT", "is_nullable": false,
                "is_pk": true, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "record_id", "data_type": "INT", "is_nullable": false,
                "is_pk": true, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "value", "data_type": "NVARCHAR(100)", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            }
        ]),
    );
    for statement in composite_sql {
        plugin.execute(statement);
    }

    // One representative from every manifest category: numeric, text,
    // binary, datetime, boolean, other, and spatial.
    let categories_sql = generated_create_table_sql(
        &mut plugin,
        "ddl_categories",
        json!([
            {
                "name": "numeric_value", "data_type": "DECIMAL(18,2)", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "text_value", "data_type": "NVARCHAR(100)", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "binary_value", "data_type": "VARBINARY(100)", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "datetime_value", "data_type": "DATETIME2", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "boolean_value", "data_type": "BIT", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "other_value", "data_type": "UNIQUEIDENTIFIER", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            },
            {
                "name": "spatial_value", "data_type": "GEOGRAPHY", "is_nullable": true,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            }
        ]),
    );
    for statement in categories_sql {
        plugin.execute(statement);
    }

    let identity_columns = plugin.call_ok(
        "get_columns",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "ddl_identity"
        }),
    );
    assert_eq!(identity_columns[0]["is_pk"], true);
    assert_eq!(identity_columns[0]["is_auto_increment"], true);

    let composite_columns = plugin.call_ok(
        "get_columns",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "ddl_composite"
        }),
    );
    assert_eq!(
        composite_columns
            .as_array()
            .expect("columns array")
            .iter()
            .filter(|column| column["is_pk"] == true)
            .count(),
        2
    );

    let category_columns = plugin.call_ok(
        "get_columns",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "ddl_categories"
        }),
    );
    let names: BTreeSet<&str> = category_columns
        .as_array()
        .expect("columns array")
        .iter()
        .filter_map(|column| column["name"].as_str())
        .collect();
    assert_eq!(names.len(), 7);
}

fn advertised_type_cases() -> Vec<TypeCase> {
    vec![
        TypeCase::exact(
            "TINYINT",
            "TINYINT",
            json!(42),
            json!(42),
            json!(255),
            json!(255),
        ),
        TypeCase::exact(
            "SMALLINT",
            "SMALLINT",
            json!(-123),
            json!(-123),
            json!(-32768),
            json!(-32768),
        ),
        TypeCase::exact(
            "INT",
            "INT",
            json!(123456),
            json!(123456),
            json!(2147483647),
            json!(2147483647),
        ),
        TypeCase::exact(
            "BIGINT",
            "BIGINT",
            json!("9007199254740992"),
            json!("9007199254740992"),
            json!("-9223372036854775808"),
            json!("-9223372036854775808"),
        ),
        TypeCase::exact(
            "DECIMAL",
            "DECIMAL(38,10)",
            json!("1234567890123456789012345678.1234567890"),
            json!("1234567890123456789012345678.123456789"),
            json!("-9999999999999999999999999999.9999999999"),
            json!("-9999999999999999999999999999.9999999999"),
        ),
        TypeCase::exact(
            "NUMERIC",
            "NUMERIC(38,0)",
            json!("90071992547409931234567890123456789012"),
            json!("90071992547409931234567890123456789012"),
            json!("99999999999999999999999999999999999999"),
            json!("99999999999999999999999999999999999999"),
        ),
        TypeCase::exact(
            "SMALLMONEY",
            "SMALLMONEY",
            json!("12.3456"),
            json!("12.3456"),
            json!("-214748.3648"),
            json!("-214748.3648"),
        ),
        TypeCase::exact(
            "MONEY",
            "MONEY",
            json!("-12.3400"),
            json!("-12.34"),
            json!("922337203685477.5807"),
            json!("922337203685477.5807"),
        ),
        TypeCase::exact(
            "FLOAT",
            "FLOAT",
            json!(1.25),
            json!(1.25),
            json!(1.7976931348623157e308),
            json!(1.7976931348623157e308),
        ),
        TypeCase {
            advertised_name: "REAL",
            ddl: "REAL",
            insert: json!(1.25),
            inserted: ExpectedCell::Exact(json!(1.25)),
            boundary: json!(3.4028235e38),
            bounded: ExpectedCell::Approx(3.4028235e38),
            semantic_check: None,
        },
        TypeCase::exact(
            "CHAR",
            "CHAR(5)",
            json!("abc"),
            json!("abc  "),
            json!("12345"),
            json!("12345"),
        ),
        TypeCase::exact(
            "VARCHAR",
            "VARCHAR(8)",
            json!("plain"),
            json!("plain"),
            json!("edge'123"),
            json!("edge'123"),
        ),
        TypeCase::exact(
            "VARCHAR(MAX)",
            "VARCHAR(MAX)",
            json!("max text"),
            json!("max text"),
            json!("boundary text"),
            json!("boundary text"),
        ),
        TypeCase::exact(
            "TEXT",
            "TEXT",
            json!("legacy text"),
            json!("legacy text"),
            json!("legacy boundary"),
            json!("legacy boundary"),
        ),
        TypeCase::exact(
            "NCHAR",
            "NCHAR(4)",
            json!("猫"),
            json!("猫   "),
            json!("猫犬鳥魚"),
            json!("猫犬鳥魚"),
        ),
        TypeCase::exact(
            "NVARCHAR",
            "NVARCHAR(16)",
            json!("Grüße 🦀"),
            json!("Grüße 🦀"),
            json!("東京"),
            json!("東京"),
        ),
        TypeCase::exact(
            "NVARCHAR(MAX)",
            "NVARCHAR(MAX)",
            json!("Unicode Ω"),
            json!("Unicode Ω"),
            json!("boundary 🦀"),
            json!("boundary 🦀"),
        ),
        TypeCase::exact(
            "NTEXT",
            "NTEXT",
            json!("legacy Ω"),
            json!("legacy Ω"),
            json!("旧式"),
            json!("旧式"),
        ),
        TypeCase::exact(
            "BINARY",
            "BINARY(4)",
            blob_wire(&[1, 2]),
            blob_wire(&[1, 2, 0, 0]),
            blob_wire(&[0xde, 0xad, 0xbe, 0xef]),
            blob_wire(&[0xde, 0xad, 0xbe, 0xef]),
        ),
        TypeCase::exact(
            "VARBINARY",
            "VARBINARY(8)",
            blob_wire(&[1, 2, 3]),
            blob_wire(&[1, 2, 3]),
            blob_wire(&[0, 1, 2, 3, 4, 5, 6, 7]),
            blob_wire(&[0, 1, 2, 3, 4, 5, 6, 7]),
        ),
        TypeCase::exact(
            "VARBINARY(MAX)",
            "VARBINARY(MAX)",
            blob_wire(&[0xca, 0xfe]),
            blob_wire(&[0xca, 0xfe]),
            blob_wire(&[0xde, 0xad, 0xbe, 0xef]),
            blob_wire(&[0xde, 0xad, 0xbe, 0xef]),
        ),
        TypeCase::exact(
            "IMAGE",
            "IMAGE",
            blob_wire(&[9, 8, 7]),
            blob_wire(&[9, 8, 7]),
            blob_wire(&[6, 5, 4, 3]),
            blob_wire(&[6, 5, 4, 3]),
        ),
        TypeCase::exact(
            "DATE",
            "DATE",
            json!("2024-02-29"),
            json!("2024-02-29"),
            json!("0001-01-01"),
            json!("0001-01-01"),
        ),
        TypeCase::exact(
            "TIME",
            "TIME(7)",
            json!("12:34:56.1234567"),
            json!("12:34:56.1234567"),
            json!("23:59:59.9999999"),
            json!("23:59:59.9999999"),
        ),
        TypeCase::exact(
            "DATETIME",
            "DATETIME",
            json!("2024-01-02 03:04:05.006"),
            json!("2024-01-02 03:04:05.007"),
            json!("9999-12-31 23:59:59.997"),
            json!("9999-12-31 23:59:59.997"),
        ),
        TypeCase::exact(
            "DATETIME2",
            "DATETIME2(7)",
            json!("2024-01-02 03:04:05.1234567"),
            json!("2024-01-02 03:04:05.1234567"),
            json!("9999-12-31 23:59:59.9999999"),
            json!("9999-12-31 23:59:59.9999999"),
        ),
        TypeCase::exact(
            "SMALLDATETIME",
            "SMALLDATETIME",
            json!("2024-01-02 12:34:31"),
            json!("2024-01-02 12:35:00"),
            json!("1900-01-01 00:00:00"),
            json!("1900-01-01 00:00:00"),
        ),
        TypeCase::exact(
            "DATETIMEOFFSET",
            "DATETIMEOFFSET(7)",
            json!("2024-01-02 03:04:05.1234567 +05:30"),
            json!("2024-01-02T03:04:05.123456700+05:30"),
            json!("9999-12-31 23:59:59.9999999 +14:00"),
            json!("9999-12-31T23:59:59.999999900+14:00"),
        ),
        TypeCase::exact(
            "BIT",
            "BIT",
            json!(true),
            json!(true),
            json!(false),
            json!(false),
        ),
        TypeCase::exact(
            "UNIQUEIDENTIFIER",
            "UNIQUEIDENTIFIER",
            json!("00112233-4455-6677-8899-aabbccddeeff"),
            json!("00112233-4455-6677-8899-aabbccddeeff"),
            json!("ffffffff-ffff-ffff-ffff-ffffffffffff"),
            json!("ffffffff-ffff-ffff-ffff-ffffffffffff"),
        ),
        TypeCase::exact(
            "XML",
            "XML",
            json!("<root attr=\"x\">text</root>"),
            json!("<root attr=\"x\">text</root>"),
            json!("<r />"),
            json!("<r/>"),
        ),
        TypeCase::exact(
            "SQL_VARIANT",
            "SQL_VARIANT",
            json!("variant text"),
            json!("variant text"),
            raw_sql("CAST(2147483647 AS INT)"),
            json!(2147483647),
        ),
        TypeCase {
            advertised_name: "HIERARCHYID",
            ddl: "HIERARCHYID",
            insert: raw_sql("hierarchyid::Parse('/1/3/')"),
            inserted: ExpectedCell::Blob { exact_size: None },
            boundary: raw_sql("hierarchyid::Parse('/9/')"),
            bounded: ExpectedCell::Blob { exact_size: None },
            semantic_check: Some(("value.ToString()", json!("/9/"))),
        },
        TypeCase {
            advertised_name: "GEOGRAPHY",
            ddl: "GEOGRAPHY",
            insert: raw_sql("geography::STGeomFromText('POINT (-122.35 47.65)', 4326)"),
            inserted: ExpectedCell::Blob { exact_size: None },
            boundary: raw_sql("geography::STGeomFromText('POINT (180 90)', 4326)"),
            bounded: ExpectedCell::Blob { exact_size: None },
            semantic_check: Some(("value.STSrid", json!(4326))),
        },
        TypeCase {
            advertised_name: "GEOMETRY",
            ddl: "GEOMETRY",
            insert: raw_sql("geometry::STGeomFromText('LINESTRING (0 0, 3 4)', 0)"),
            inserted: ExpectedCell::Blob { exact_size: None },
            boundary: raw_sql("geometry::STGeomFromText('POINT (1 2)', 0)"),
            bounded: ExpectedCell::Blob { exact_size: None },
            semantic_check: Some(("value.ToString()", json!("POINT (1 2)"))),
        },
    ]
}

#[test]
fn advertised_types_round_trip_through_query_insert_update_and_null() {
    let manifest: Value = serde_json::from_str(include_str!("../.tabularium"))
        .expect(".tabularium must be valid JSON");
    let advertised: BTreeSet<String> = manifest["data_types"]
        .as_array()
        .expect("manifest data_types")
        .iter()
        .map(|data_type| data_type["name"].as_str().expect("type name").to_string())
        .collect();
    let cases = advertised_type_cases();
    let covered: BTreeSet<String> = cases
        .iter()
        .map(|case| case.advertised_name.to_string())
        .chain(["ROWVERSION".to_string(), "TIMESTAMP".to_string()])
        .collect();
    assert_eq!(
        covered, advertised,
        "live matrix must cover every advertised type"
    );
    assert!(
        !advertised.contains("JSON") && !advertised.contains("VECTOR"),
        "native JSON and VECTOR require a SQL Server version newer than the 2022 release baseline"
    );

    let mut plugin = Plugin::with_scratch_database();
    for (index, case) in cases.iter().enumerate() {
        let table = format!("type_{index:02}");
        plugin.reset_table(
            &table,
            &format!("id INT PRIMARY KEY, value {} NULL", case.ddl),
        );
        for (id, value) in [
            (1, case.insert.clone()),
            (2, Value::Null),
            (3, case.insert.clone()),
        ] {
            assert_eq!(
                plugin.call_ok(
                    "insert_record",
                    json!({
                        "params": connection_params(), "schema": TEST_SCHEMA, "table": table,
                        "data": { "id": id, "value": value }
                    }),
                ),
                json!(1),
                "{} insert id {id}",
                case.advertised_name
            );
        }
        assert_eq!(
            plugin.call_ok(
                "update_record",
                json!({
                    "params": connection_params(), "schema": TEST_SCHEMA, "table": table,
                    "pk_map": { "id": 3 }, "col_name": "value",
                    "new_val": case.boundary.clone()
                }),
            ),
            json!(1),
            "{} update",
            case.advertised_name
        );

        let result = plugin.execute(format!(
            "SELECT value FROM [{TEST_SCHEMA}].[{table}] ORDER BY id"
        ));
        let rows = result_rows(&result);
        assert_eq!(rows.len(), 3, "{} row count", case.advertised_name);
        assert_cell(case, "representative", &rows[0][0], &case.inserted);
        assert_eq!(
            rows[1][0],
            Value::Null,
            "{} SQL NULL representation",
            case.advertised_name
        );
        assert_cell(case, "boundary", &rows[2][0], &case.bounded);

        if let Some((expression, expected)) = &case.semantic_check {
            let semantic = plugin.execute(format!(
                "SELECT {expression} FROM [{TEST_SCHEMA}].[{table}] WHERE id = 3"
            ));
            assert_eq!(
                semantic["rows"][0][0], *expected,
                "{} raw-expression write semantics",
                case.advertised_name
            );
        }
    }

    // ROWVERSION and its TIMESTAMP synonym are generated concurrency tokens:
    // they have a defined eight-byte read representation but deliberately no
    // NULL, insert-value, or update-value direction.
    for (index, type_name) in ["ROWVERSION", "TIMESTAMP"].iter().enumerate() {
        let table = format!("type_readonly_{index}");
        plugin.reset_table(&table, &format!("id INT PRIMARY KEY, value {type_name}"));
        for id in [1, 2] {
            plugin.call_ok(
                "insert_record",
                json!({
                    "params": connection_params(), "schema": TEST_SCHEMA, "table": table,
                    "data": { "id": id }
                }),
            );
        }
        let result = plugin.execute(format!(
            "SELECT value FROM [{TEST_SCHEMA}].[{table}] ORDER BY id"
        ));
        for row in result_rows(&result) {
            let case = TypeCase {
                advertised_name: type_name,
                ddl: type_name,
                insert: Value::Null,
                inserted: ExpectedCell::Blob {
                    exact_size: Some(8),
                },
                boundary: Value::Null,
                bounded: ExpectedCell::Blob {
                    exact_size: Some(8),
                },
                semantic_check: None,
            };
            assert_cell(&case, "generated value", &row[0], &case.inserted);
        }
        let insert_error = plugin.call_error(
            "insert_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": table,
                "data": { "id": 3, "value": blob_wire(&[0; 8]) }
            }),
        );
        assert!(
            insert_error.to_ascii_lowercase().contains("timestamp"),
            "{type_name} must reject explicit host inserts: {insert_error}"
        );
        let update_error = plugin.call_error(
            "update_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": table,
                "pk_map": { "id": 1 }, "col_name": "value",
                "new_val": blob_wire(&[0; 8])
            }),
        );
        assert!(
            update_error.to_ascii_lowercase().contains("timestamp"),
            "{type_name} must reject host updates: {update_error}"
        );
    }
}

#[test]
fn crud_insert_update_and_delete_support_single_and_composite_primary_keys() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "crud_single",
        "id INT IDENTITY(1,1) PRIMARY KEY, value NVARCHAR(100) NOT NULL",
    );
    plugin.reset_table(
        "crud_composite",
        "tenant_id INT NOT NULL, record_id INT NOT NULL, value NVARCHAR(100) NOT NULL, \
         PRIMARY KEY (tenant_id, record_id)",
    );

    let inserted = plugin.call_ok(
        "insert_record",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "crud_single",
            "data": { "value": "before" }
        }),
    );
    assert_eq!(inserted, json!(1));
    let single_row = plugin.execute(format!(
        "SELECT id, value FROM [{TEST_SCHEMA}].[crud_single]"
    ));
    let single_id = single_row["rows"][0][0].as_i64().expect("identity id");

    assert_eq!(
        plugin.call_ok(
            "update_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": "crud_single",
                "pk_map": { "id": single_id }, "col_name": "value", "new_val": "after"
            }),
        ),
        json!(1)
    );
    assert_eq!(
        plugin.call_ok(
            "delete_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": "crud_single",
                "pk_map": { "id": single_id }
            }),
        ),
        json!(1)
    );

    assert_eq!(
        plugin.call_ok(
            "insert_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA,
                "table": "crud_composite",
                "data": { "tenant_id": 7, "record_id": 9, "value": "before" }
            }),
        ),
        json!(1)
    );
    assert_eq!(
        plugin.call_ok(
            "update_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA,
                "table": "crud_composite",
                "pk_map": { "record_id": 9, "tenant_id": 7 },
                "col_name": "value", "new_val": "after"
            }),
        ),
        json!(1)
    );
    let updated = plugin.execute(format!(
        "SELECT value FROM [{TEST_SCHEMA}].[crud_composite] \
         WHERE tenant_id = 7 AND record_id = 9"
    ));
    assert_eq!(updated["rows"][0], json!(["after"]));
    assert_eq!(
        plugin.call_ok(
            "delete_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA,
                "table": "crud_composite", "pk_map": { "tenant_id": 7, "record_id": 9 }
            }),
        ),
        json!(1)
    );
}

#[test]
fn hostile_identifiers_survive_ddl_and_crud_round_trip() {
    const TABLE: &str = "[weird\"name]]";
    const KEY_COLUMN: &str = "order";
    const VALUE_COLUMN: &str = "9Δ\"value]";

    let mut plugin = Plugin::with_scratch_database();
    let table_ref = format!("{}.{}", bracket_quote(TEST_SCHEMA), bracket_quote(TABLE));
    plugin.execute(format!("DROP TABLE IF EXISTS {table_ref}"));

    let create = generated_create_table_sql(
        &mut plugin,
        TABLE,
        json!([
            {
                "name": KEY_COLUMN, "data_type": "INT", "is_nullable": false,
                "is_pk": true, "is_auto_increment": false, "default_value": null
            },
            {
                "name": VALUE_COLUMN, "data_type": "NVARCHAR(100)", "is_nullable": false,
                "is_pk": false, "is_auto_increment": false, "default_value": null
            }
        ]),
    );
    for statement in create {
        plugin.execute(statement);
    }

    assert_eq!(
        plugin.call_ok(
            "insert_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": TABLE,
                "data": { "order": 7, "9Δ\"value]": "before" }
            }),
        ),
        json!(1)
    );
    assert_eq!(
        plugin.call_ok(
            "update_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": TABLE,
                "pk_map": { "order": 7 }, "col_name": VALUE_COLUMN, "new_val": "after"
            }),
        ),
        json!(1)
    );

    let selected = plugin.execute(format!(
        "SELECT {} FROM {table_ref} WHERE {} = 7",
        bracket_quote(VALUE_COLUMN),
        bracket_quote(KEY_COLUMN),
    ));
    assert_eq!(selected["columns"], json!([VALUE_COLUMN]));
    assert_eq!(selected["rows"], json!([["after"]]));

    assert_eq!(
        plugin.call_ok(
            "delete_record",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA, "table": TABLE,
                "pk_map": { "order": 7 }
            }),
        ),
        json!(1)
    );
    plugin.execute(format!("DROP TABLE {table_ref}"));
}

#[test]
fn zero_row_select_preserves_column_headers() {
    let mut plugin = Plugin::with_scratch_database();
    let result = plugin
        .execute("SELECT CAST(1 AS INT) AS id, CAST(N'x' AS NVARCHAR(10)) AS label WHERE 1 = 0");
    assert_eq!(result["columns"], json!(["id", "label"]));
    assert!(result_rows(&result).is_empty());
}

#[test]
fn multi_statement_and_batch_rpc_preserve_result_sets_and_temp_table_session() {
    let mut plugin = Plugin::with_scratch_database();
    let multi = plugin.execute(
        "SELECT CAST(1 AS INT) AS first_value; \
         CREATE TABLE #ss003_multi (value INT NOT NULL); \
         INSERT INTO #ss003_multi VALUES (2), (3); \
         SELECT value FROM #ss003_multi ORDER BY value",
    );
    assert_eq!(multi["columns"], json!(["first_value"]));
    assert_eq!(multi["rows"], json!([[1]]));
    assert_eq!(multi["additional_results"][0]["columns"], json!(["value"]));
    assert_eq!(multi["additional_results"][0]["rows"], json!([[2], [3]]));

    let batch = plugin.call_ok(
        "execute_query_batch",
        json!({
            "params": connection_params(),
            "queries": [
                "CREATE TABLE #ss003_batch (value INT NOT NULL)",
                "INSERT INTO #ss003_batch VALUES (10), (20)",
                "SELECT value FROM #ss003_batch ORDER BY value"
            ]
        }),
    );
    assert_eq!(batch[1]["result"]["affected_rows"], 2);
    assert_eq!(batch[2]["result"]["columns"], json!(["value"]));
    assert_eq!(batch[2]["result"]["rows"], json!([[10], [20]]));
}

#[test]
fn affected_rows_cover_plain_multi_statement_and_output_dml() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table("affected_rows", "id INT PRIMARY KEY, value INT NOT NULL");
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[affected_rows] VALUES (1, 0), (2, 0), (3, 0)"
    ));

    let plain = plugin.execute(format!(
        "UPDATE [{TEST_SCHEMA}].[affected_rows] SET value = 1 WHERE id <= 2"
    ));
    assert_eq!(plain["affected_rows"], 2);
    assert_eq!(plain["rows"], json!([]));

    let multi = plugin.execute(format!(
        "UPDATE [{TEST_SCHEMA}].[affected_rows] SET value = 2; \
         DELETE FROM [{TEST_SCHEMA}].[affected_rows] WHERE id = 3"
    ));
    assert_eq!(
        multi["affected_rows"], 1,
        "a batch reports the final DML statement's @@ROWCOUNT"
    );

    let output = plugin.execute(format!(
        "UPDATE [{TEST_SCHEMA}].[affected_rows] SET value = 3 \
         OUTPUT inserted.id WHERE id <= 2"
    ));
    assert_eq!(output["affected_rows"], 2);
    assert_eq!(output["columns"], json!(["id"]));
    let mut ids: Vec<i64> = result_rows(&output)
        .iter()
        .map(|row| row[0].as_i64().expect("OUTPUT id must be an integer"))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 2]);
    assert!(output.get("additional_results").is_none());
}

#[test]
fn identity_insert_succeeds_and_failure_restores_session_state() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "identity_first",
        "id INT IDENTITY(1,1) PRIMARY KEY, value NVARCHAR(100) NOT NULL",
    );
    plugin.reset_table(
        "identity_second",
        "id INT IDENTITY(1,1) PRIMARY KEY, value NVARCHAR(100) NOT NULL",
    );

    let explicit = plugin.call_ok(
        "insert_record",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "identity_first",
            "data": { "id": 100, "value": "explicit" }
        }),
    );
    assert_eq!(explicit, json!(1));

    let duplicate_error = plugin.call_error(
        "insert_record",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "identity_first",
            "data": { "id": 100, "value": "duplicate" }
        }),
    );
    assert!(!duplicate_error.is_empty());

    // SQL Server permits IDENTITY_INSERT ON for only one table per session.
    // An explicit insert into a second table therefore proves the failed
    // first-table batch turned the session-scoped setting back off.
    let recovered = plugin.call_ok(
        "insert_record",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA, "table": "identity_second",
            "data": { "id": 200, "value": "session-recovered" }
        }),
    );
    assert_eq!(recovered, json!(1));
}

#[test]
fn pagination_and_batch_semantics_cover_ordered_unordered_cte_and_dml() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "pagination",
        "id INT PRIMARY KEY, touched BIT NOT NULL DEFAULT 0",
    );
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[pagination] (id) VALUES (1), (2), (3), (4), (5)"
    ));
    let ordered_query = format!("SELECT id FROM [{TEST_SCHEMA}].[pagination] ORDER BY id");

    let page_one = plugin.call_ok(
        "execute_query",
        json!({ "params": connection_params(), "query": ordered_query, "limit": 2, "page": 1 }),
    );
    assert_eq!(page_one["rows"], json!([[1], [2]]));
    assert_eq!(page_one["pagination"]["page"], 1);
    assert_eq!(page_one["pagination"]["page_size"], 2);
    assert_eq!(page_one["pagination"]["has_more"], true);
    assert_eq!(page_one["pagination"]["total_rows"], Value::Null);
    assert_eq!(page_one["truncated"], true);
    assert!(page_one.get("additional_results").is_none());

    let final_page = plugin.call_ok(
        "execute_query",
        json!({ "params": connection_params(), "query": ordered_query, "limit": 2, "page": 3 }),
    );
    assert_eq!(final_page["rows"], json!([[5]]));
    assert_eq!(final_page["pagination"]["has_more"], false);
    assert_eq!(final_page["truncated"], false);

    let unordered = plugin.call_ok(
        "execute_query",
        json!({
            "params": connection_params(),
            "query": format!("SELECT id FROM [{TEST_SCHEMA}].[pagination]"),
            "limit": 2,
            "page": 1
        }),
    );
    assert_eq!(result_rows(&unordered).len(), 2);
    assert_eq!(unordered["pagination"]["has_more"], true);
    assert_eq!(unordered["pagination"]["total_rows"], Value::Null);

    let cte = plugin.call_ok(
        "execute_query",
        json!({
            "params": connection_params(),
            "query": format!(
                "WITH source AS (SELECT id FROM [{TEST_SCHEMA}].[pagination] WHERE id >= 2) \
                 SELECT id FROM source ORDER BY id DESC"
            ),
            "limit": 2,
            "page": 2
        }),
    );
    assert_eq!(cte["rows"], json!([[3], [2]]));
    assert_eq!(cte["pagination"]["has_more"], false);

    let batch = plugin.call_ok(
        "execute_query_batch",
        json!({
            "params": connection_params(),
            "queries": [
                format!("UPDATE [{TEST_SCHEMA}].[pagination] SET touched = 1 WHERE id = 1"),
                ordered_query,
                format!(
                    "SELECT id INTO #ss041_selected FROM [{TEST_SCHEMA}].[pagination] WHERE id <= 3"
                ),
                "SELECT id FROM #ss041_selected ORDER BY id"
            ],
            "limit": 2,
            "page": 1
        }),
    );
    assert_eq!(batch[0]["result"]["affected_rows"], 1);
    assert!(batch[0]["result"].get("additional_results").is_none());
    assert_eq!(batch[1]["result"]["rows"], json!([[1], [2]]));
    assert_eq!(batch[1]["result"]["pagination"]["has_more"], true);
    assert_eq!(batch[2]["result"]["affected_rows"], 3);
    assert_eq!(batch[2]["result"]["rows"], json!([]));
    assert!(batch[2]["result"].get("additional_results").is_none());
    assert_eq!(batch[3]["result"]["rows"], json!([[1], [2]]));
    assert_eq!(batch[3]["result"]["pagination"]["has_more"], true);
}

#[test]
fn syntax_and_constraint_errors_keep_server_details_and_pool_recovery() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "errors",
        "id INT PRIMARY KEY, unique_value INT NOT NULL UNIQUE",
    );
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[errors] VALUES (1, 10)"
    ));

    let syntax_error = plugin.call_error(
        "execute_query",
        json!({
            "params": connection_params(),
            "query": "\nSELECT 1 +"
        }),
    );
    assert!(
        syntax_error.starts_with("SQL Server error 102:"),
        "{syntax_error}"
    );
    assert!(syntax_error.contains("syntax error"), "{syntax_error}");
    assert!(syntax_error.contains("line 2"), "{syntax_error}");
    let after_syntax = plugin.execute("SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(after_syntax["rows"], json!([[1]]));

    let constraint_error = plugin.call_error(
        "execute_query",
        json!({
            "params": connection_params(),
            "query": format!(
                "EXEC(N'INSERT INTO [{TEST_SCHEMA}].[errors] VALUES (2, 10)')"
            )
        }),
    );
    assert!(
        constraint_error.starts_with("SQL Server error 2627:"),
        "{constraint_error}"
    );
    assert!(
        constraint_error.contains("constraint violation"),
        "{constraint_error}"
    );
    let after_constraint = plugin.execute(format!(
        "SELECT COUNT(*) AS row_count FROM [{TEST_SCHEMA}].[errors]"
    ));
    assert_eq!(after_constraint["rows"], json!([[1]]));
}

#[test]
fn connection_authentication_and_tls_errors_are_actionable_and_redacted() {
    let mut plugin = Plugin::with_scratch_database();
    let valid = connection_params();
    let username = valid["username"].as_str().expect("username");
    let password = valid["password"].as_str().expect("password");
    let host = valid["host"].as_str().expect("host");
    let port = valid["port"].as_u64().expect("port");
    let database = valid["database"].as_str().expect("database");

    let connection_secret = "Ss043!ConnectionSecret9";
    let failed_connection_string = format!(
        "sqlserver://{}:{}@{}:1/{}?Encrypt=true&TrustServerCertificate=true",
        url_encode_component(username),
        url_encode_component(connection_secret),
        host,
        url_encode_component(database),
    );
    let connection_error = plugin.call_error(
        "test_connection",
        json!({
            "params": {
                "connection_string": failed_connection_string,
                "connection_id": "ss043-connection-recovery"
            }
        }),
    );
    assert!(
        connection_error.contains("SQL Server connection failure"),
        "{connection_error}"
    );
    assert!(!connection_error.contains(connection_secret));
    assert!(!connection_error.contains(password));
    assert!(!connection_error.contains(&failed_connection_string));

    let mut recovered_connection = valid.clone();
    recovered_connection["connection_id"] = json!("ss043-connection-recovery");
    assert_eq!(
        plugin.call_ok("test_connection", json!({ "params": recovered_connection })),
        json!({ "success": true })
    );

    let authentication_secret = "Ss043!WrongPassword9";
    let failed_auth_string = format!(
        "sqlserver://{}:{}@{}:{}/{}?Encrypt=true&TrustServerCertificate=true",
        url_encode_component(username),
        url_encode_component(authentication_secret),
        host,
        port,
        url_encode_component(database),
    );
    let authentication_error = plugin.call_error(
        "test_connection",
        json!({
            "params": {
                "connection_string": failed_auth_string,
                "connection_id": "ss043-auth-recovery"
            }
        }),
    );
    assert!(
        authentication_error.starts_with("SQL Server error 18456:"),
        "{authentication_error}"
    );
    assert!(
        authentication_error.contains("authentication failure"),
        "{authentication_error}"
    );
    assert!(!authentication_error.contains(authentication_secret));
    assert!(!authentication_error.contains(password));
    assert!(!authentication_error.contains(&failed_auth_string));

    let mut recovered_auth = valid.clone();
    recovered_auth["connection_id"] = json!("ss043-auth-recovery");
    assert_eq!(
        plugin.call_ok("test_connection", json!({ "params": recovered_auth })),
        json!({ "success": true })
    );

    let mut verify_full = valid;
    verify_full["connection_id"] = json!("ss043-tls");
    verify_full["ssl_mode"] = json!("verify-full");
    let tls_error = plugin.call_error("test_connection", json!({ "params": verify_full }));
    assert!(
        tls_error.contains("SQL Server TLS negotiation failure"),
        "{tls_error}"
    );
    assert!(tls_error.contains("ssl_mode 'verify-full'"), "{tls_error}");
    assert!(tls_error.contains("ssl_mode 'require'"), "{tls_error}");

    // A setup failure has no physical session to recycle, but it must not
    // poison healthy pools in the same plugin process.
    let after_tls = plugin.execute("SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(after_tls["rows"], json!([[1]]));
}

#[test]
fn permission_denial_keeps_number_and_pool_recovers() {
    const LOGIN: &str = "ss043_denied_login";
    const USER: &str = "ss043_denied_user";
    const PASSWORD: &str = "Ss043!DeniedPassword9";

    let mut plugin = Plugin::with_scratch_database();
    plugin.execute(format!(
        "IF DATABASE_PRINCIPAL_ID(N'{USER}') IS NOT NULL DROP USER [{USER}]; \
         IF SUSER_ID(N'{LOGIN}') IS NOT NULL DROP LOGIN [{LOGIN}]; \
         DROP TABLE IF EXISTS [{TEST_SCHEMA}].[permission_error]; \
         CREATE TABLE [{TEST_SCHEMA}].[permission_error] (id INT PRIMARY KEY); \
         CREATE LOGIN [{LOGIN}] WITH PASSWORD = N'{PASSWORD}', CHECK_POLICY = OFF; \
         CREATE USER [{USER}] FOR LOGIN [{LOGIN}]; \
         DENY SELECT ON OBJECT::[{TEST_SCHEMA}].[permission_error] TO [{USER}]"
    ));

    let mut denied_params = connection_params();
    denied_params["username"] = json!(LOGIN);
    denied_params["password"] = json!(PASSWORD);
    denied_params["connection_id"] = json!("ss043-permission");
    let permission_error = plugin.call_error(
        "execute_query",
        json!({
            "params": denied_params,
            "query": format!("SELECT id FROM [{TEST_SCHEMA}].[permission_error]")
        }),
    );
    assert!(
        permission_error.starts_with("SQL Server error 229:"),
        "{permission_error}"
    );
    assert!(
        permission_error.contains("permission denial"),
        "{permission_error}"
    );
    assert!(!permission_error.contains(PASSWORD));
    let recovered = plugin.execute_with(&denied_params, "SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(recovered["rows"], json!([[1]]));

    plugin.call_ok("shutdown", json!({}));
    plugin.execute(format!(
        "DROP TABLE IF EXISTS [{TEST_SCHEMA}].[permission_error]; \
         IF DATABASE_PRINCIPAL_ID(N'{USER}') IS NOT NULL DROP USER [{USER}]; \
         IF SUSER_ID(N'{LOGIN}') IS NOT NULL DROP LOGIN [{LOGIN}]"
    ));
}

#[test]
fn timeout_is_named_and_pool_replaces_the_cancelled_session() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.call_ok(
        "initialize",
        json!({ "settings": { "query_timeout_seconds": 1 } }),
    );
    let mut params = connection_params();
    params["connection_id"] = json!("ss043-timeout");

    let timeout_error = plugin.call_error(
        "execute_query",
        json!({
            "params": params,
            "query": "WAITFOR DELAY '00:00:03'; SELECT CAST(1 AS INT) AS too_late"
        }),
    );
    assert!(
        timeout_error.contains("SQL Server timeout"),
        "{timeout_error}"
    );
    let recovered = plugin.execute_with(&params, "SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(recovered["rows"], json!([[1]]));
}

#[test]
fn recycle_clears_identity_showplan_transaction_and_temp_table_state() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "recycle_identity_first",
        "id INT IDENTITY(1,1) PRIMARY KEY, value INT NOT NULL",
    );
    plugin.reset_table(
        "recycle_identity_second",
        "id INT IDENTITY(1,1) PRIMARY KEY, value INT NOT NULL",
    );
    let mut params = connection_params();
    params["connection_id"] = json!("ss043-session-state");
    let before = plugin.execute_with(&params, "SELECT @@SPID AS session_id");
    let session_id = before["rows"][0][0].as_i64().expect("session id");

    // Leave identity and temp-table state deliberately active after a
    // successful RPC. The next checkout must run the manager's reset on the
    // same physical session.
    plugin.execute_with(
        &params,
        format!(
            "SET IDENTITY_INSERT [{TEST_SCHEMA}].[recycle_identity_first] ON; \
             CREATE TABLE #ss043_temp (id INT)"
        ),
    );
    let reset_state = plugin.execute_with(
        &params,
        "SELECT @@SPID AS session_id, @@TRANCOUNT AS transaction_count, \
         CASE WHEN OBJECT_ID('tempdb..#ss043_temp') IS NULL THEN 0 ELSE 1 END AS temp_exists",
    );
    assert_eq!(reset_state["rows"], json!([[session_id, 0, 0]]));
    let identity_recovered = plugin.call_ok(
        "insert_record",
        json!({
            "params": params, "schema": TEST_SCHEMA, "table": "recycle_identity_second",
            "data": { "id": 43, "value": 1 }
        }),
    );
    assert_eq!(identity_recovered, json!(1));

    plugin.execute_with(&params, "SET SHOWPLAN_XML ON");
    let reset_showplan = plugin.execute_with(
        &params,
        "SELECT @@SPID AS session_id, CAST(1 AS INT) AS connection_ok",
    );
    assert_eq!(reset_showplan["rows"], json!([[session_id, 1]]));

    // Open transactions are discarded without issuing commands into their
    // session. Closing the socket rolls back and drops local temp objects.
    plugin.execute_with(
        &params,
        "BEGIN TRANSACTION; CREATE TABLE #ss043_open_transaction_temp (id INT)",
    );
    let reset_transaction = plugin.execute_with(
        &params,
        "SELECT @@TRANCOUNT AS transaction_count, \
         CASE WHEN OBJECT_ID('tempdb..#ss043_open_transaction_temp') IS NULL THEN 0 ELSE 1 END AS temp_exists",
    );
    assert_eq!(reset_transaction["rows"], json!([[0, 0]]));

    // Server errors are conservatively discarded because an error token can
    // leave unread protocol state. Replacement must still be immediate and
    // must roll back the transaction and remove its temp table.
    let identity_error = plugin.call_error(
        "execute_query",
        json!({
            "params": params,
            "query": format!(
                "SET IDENTITY_INSERT [{TEST_SCHEMA}].[recycle_identity_first] ON; \
                 THROW 50043, 'identity cleanup fixture', 1"
            )
        }),
    );
    assert!(identity_error.starts_with("SQL Server error 50043:"));
    let after_identity_error = plugin.call_ok(
        "insert_record",
        json!({
            "params": params, "schema": TEST_SCHEMA, "table": "recycle_identity_second",
            "data": { "id": 44, "value": 1 }
        }),
    );
    assert_eq!(after_identity_error, json!(1));

    let state_error = plugin.call_error(
        "execute_query",
        json!({
            "params": params,
            "query": "BEGIN TRANSACTION; CREATE TABLE #ss043_error_temp (id INT); SELECT 1 / 0"
        }),
    );
    assert!(
        state_error.starts_with("SQL Server error 8134:"),
        "{state_error}"
    );
    let state = plugin.execute_with(
        &params,
        "SELECT @@TRANCOUNT AS transaction_count, \
         CASE WHEN OBJECT_ID('tempdb..#ss043_error_temp') IS NULL THEN 0 ELSE 1 END AS temp_exists",
    );
    assert_eq!(state["rows"], json!([[0, 0]]));

    let showplan_error = plugin.call_error(
        "explain_query",
        json!({
            "params": params,
            "query": "SELECT missing_column FROM definitely_missing_table",
            "analyze": false
        }),
    );
    assert!(
        showplan_error.starts_with("SQL Server error 208:"),
        "{showplan_error}"
    );
    let after_showplan = plugin.execute_with(&params, "SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(after_showplan["rows"], json!([[1]]));
}

#[test]
fn deadlock_victim_is_named_and_its_pool_recovers() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table("deadlock_error", "id INT PRIMARY KEY, value INT NOT NULL");
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[deadlock_error] VALUES (1, 0), (2, 0)"
    ));

    let mut params_a = connection_params();
    params_a["connection_id"] = json!("ss043-deadlock-a");
    let mut params_b = connection_params();
    params_b["connection_id"] = json!("ss043-deadlock-b");
    let query_a = format!(
        "SET DEADLOCK_PRIORITY LOW; BEGIN TRANSACTION; \
         UPDATE [{TEST_SCHEMA}].[deadlock_error] SET value = value + 1 WHERE id = 1; \
         WAITFOR DELAY '00:00:01'; \
         UPDATE [{TEST_SCHEMA}].[deadlock_error] SET value = value + 1 WHERE id = 2; COMMIT"
    );
    let query_b = format!(
        "BEGIN TRANSACTION; \
         UPDATE [{TEST_SCHEMA}].[deadlock_error] SET value = value + 1 WHERE id = 2; \
         WAITFOR DELAY '00:00:01'; \
         UPDATE [{TEST_SCHEMA}].[deadlock_error] SET value = value + 1 WHERE id = 1; COMMIT"
    );
    let id_a = plugin.send(
        "execute_query",
        json!({ "params": params_a, "query": query_a }),
    );
    let id_b = plugin.send(
        "execute_query",
        json!({ "params": params_b, "query": query_b }),
    );
    let first = plugin.read_response();
    let second = plugin.read_response();
    let responses = [first, second];
    let (victim_id, deadlock_error) = responses
        .iter()
        .find_map(|response| {
            let message = response["error"]["message"].as_str()?;
            message
                .starts_with("SQL Server error 1205:")
                .then(|| (response["id"].as_u64().expect("response id"), message))
        })
        .expect("one concurrent transaction must be the deadlock victim");
    assert!(
        deadlock_error.starts_with("SQL Server error 1205:"),
        "{deadlock_error}"
    );
    assert!(
        deadlock_error.contains("deadlock victim"),
        "{deadlock_error}"
    );
    assert_eq!(responses.len(), 2);

    let victim_params = if victim_id == id_a {
        &params_a
    } else {
        &params_b
    };
    assert!(victim_id == id_a || victim_id == id_b);
    let recovered = plugin.execute_with(
        victim_params,
        "SELECT @@TRANCOUNT AS transaction_count, CAST(1 AS INT) AS connection_ok",
    );
    assert_eq!(recovered["rows"], json!([[0, 1]]));
}

#[test]
fn killed_pooled_connection_is_detected_and_replaced() {
    let mut plugin = Plugin::with_scratch_database();
    let mut victim_params = connection_params();
    victim_params["connection_id"] = json!("ss043-killed-victim");
    let mut killer_params = connection_params();
    killer_params["connection_id"] = json!("ss043-killer");

    let before = plugin.execute_with(&victim_params, "SELECT @@SPID AS session_id");
    let killed_session = before["rows"][0][0].as_i64().expect("session id");
    plugin.execute_with(&killer_params, format!("KILL {killed_session}"));

    let after = plugin.execute_with(
        &victim_params,
        "SELECT @@SPID AS session_id, @@TRANCOUNT AS transaction_count, \
         CAST(1 AS INT) AS connection_ok",
    );
    assert_eq!(after["rows"][0][1], json!(0));
    assert_eq!(after["rows"][0][2], json!(1));
}

#[test]
fn explain_query_returns_raw_showplan_xml_for_estimate_and_analyze() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table("explain", "id INT PRIMARY KEY, value INT NOT NULL");
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[explain] VALUES (1, 10), (2, 20)"
    ));
    let query = format!("SELECT value FROM [{TEST_SCHEMA}].[explain] WHERE id = 1");

    for analyze in [false, true] {
        let raw = plugin.call_ok(
            "explain_query",
            json!({
                "params": connection_params(),
                "query": query,
                "analyze": analyze
            }),
        );
        let object = raw
            .as_object()
            .expect("raw EXPLAIN result must be an object");
        assert_eq!(object.len(), 4, "raw EXPLAIN shape changed: {raw}");
        assert_eq!(raw["engine"], "sqlserver");
        assert_eq!(raw["format"], "sqlserver-showplan-xml");
        assert_eq!(raw["original_query"], query);

        let payload = raw["payload"]
            .as_str()
            .expect("raw EXPLAIN payload must be a SHOWPLAN XML string");
        assert!(
            payload.trim_start().starts_with("<ShowPlanXML ")
                && payload.trim_end().ends_with("</ShowPlanXML>"),
            "analyze={analyze}: {payload}"
        );
    }
}

#[test]
fn blob_png_round_trip_supports_composite_keys_image_and_clean_null_errors() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "blob_round_trip",
        "tenant_id INT NOT NULL, record_id INT NOT NULL, \
         png VARBINARY(MAX) NOT NULL, legacy IMAGE NULL, nullable VARBINARY(MAX) NULL, \
         version ROWVERSION, PRIMARY KEY (tenant_id, record_id)",
    );
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .expect("valid PNG fixture");
    let png_hex: String = png.iter().map(|byte| format!("{byte:02X}")).collect();
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[blob_round_trip] \
         (tenant_id, record_id, png, legacy, nullable) \
         VALUES (7, 9, 0x{png_hex}, 0x{png_hex}, NULL)"
    ));
    let row = json!({ "tenant_id": 7, "record_id": 9 });

    let wire = plugin.call_ok(
        "fetch_blob_as_data_url",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_round_trip", "col_name": "png", "pk_map": row,
            "max_blob_size": png.len()
        }),
    );
    assert_eq!(
        wire,
        json!(format!(
            "BLOB:{}:image/png:{}",
            png.len(),
            base64::engine::general_purpose::STANDARD.encode(&png)
        ))
    );

    let legacy_wire = plugin.call_ok(
        "fetch_blob_as_data_url",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_round_trip", "col_name": "legacy", "pk_map": row,
            "max_blob_size": png.len()
        }),
    );
    assert!(legacy_wire
        .as_str()
        .expect("IMAGE preview wire string")
        .starts_with(&format!("BLOB:{}:image/png:", png.len())));

    let export_path = std::env::temp_dir().join(format!(
        "tabularis-sqlserver-ss012-png-{}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&export_path);
    assert_eq!(
        plugin.call_ok(
            "save_blob_to_file",
            json!({
                "params": connection_params(), "schema": TEST_SCHEMA,
                "table": "blob_round_trip", "col_name": "png", "pk_map": row,
                "file_path": export_path.to_string_lossy()
            }),
        ),
        Value::Null
    );
    assert_eq!(std::fs::read(&export_path).expect("exported PNG"), png);
    std::fs::remove_file(&export_path).expect("remove exported PNG");

    let null_path = std::env::temp_dir().join(format!(
        "tabularis-sqlserver-ss012-null-{}.bin",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&null_path);
    let null_error = plugin.call_error(
        "save_blob_to_file",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_round_trip", "col_name": "nullable", "pk_map": row,
            "file_path": null_path.to_string_lossy()
        }),
    );
    assert!(null_error.contains("NULL"), "{null_error}");
    assert!(!null_path.exists(), "NULL must not create a zero-byte file");

    let rowversion_error = plugin.call_error(
        "fetch_blob_as_data_url",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_round_trip", "col_name": "version", "pk_map": row,
            "max_blob_size": 8
        }),
    );
    assert!(rowversion_error.contains("concurrency token"));
}

#[test]
fn varbinary_max_preview_ceiling_rejects_before_encoding_but_export_still_works() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table(
        "blob_ceiling",
        "id INT PRIMARY KEY, payload VARBINARY(MAX) NOT NULL",
    );
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[blob_ceiling] (id, payload) \
         VALUES (1, CONVERT(VARBINARY(MAX), REPLICATE(CAST('x' AS VARCHAR(MAX)), 4096)))"
    ));

    let error = plugin.call_error(
        "fetch_blob_as_data_url",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_ceiling", "col_name": "payload", "pk_map": { "id": 1 },
            "max_blob_size": 1024
        }),
    );
    assert!(error.contains("4096 bytes"), "{error}");
    assert!(error.contains("max_blob_size of 1024 bytes"), "{error}");

    let export_path = std::env::temp_dir().join(format!(
        "tabularis-sqlserver-ss012-large-{}.bin",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&export_path);
    plugin.call_ok(
        "save_blob_to_file",
        json!({
            "params": connection_params(), "schema": TEST_SCHEMA,
            "table": "blob_ceiling", "col_name": "payload", "pk_map": { "id": 1 },
            "file_path": export_path.to_string_lossy()
        }),
    );
    assert_eq!(
        std::fs::metadata(&export_path)
            .expect("exported large VARBINARY(MAX)")
            .len(),
        4096
    );
    std::fs::remove_file(export_path).expect("remove large BLOB export");
}

#[test]
fn startup_script_runs_on_pooled_connections() {
    let mut plugin = Plugin::with_scratch_database();
    let mut params = connection_params();
    params["connection_id"] = json!("ss003-startup-script");
    params["startup_script"] = json!("SET DATEFIRST 3");

    plugin.call_ok("test_connection", json!({ "params": params }));
    let result = plugin.call_ok(
        "execute_query",
        json!({ "params": params, "query": "SELECT @@DATEFIRST AS date_first" }),
    );
    assert_eq!(result["rows"], json!([[3]]));
}

#[test]
fn connection_string_only_connects_for_url_and_keyword_syntaxes() {
    let mut plugin = Plugin::with_scratch_database();
    let params = connection_params();
    let username = params["username"].as_str().expect("username");
    let password = params["password"].as_str().expect("password");
    let host = params["host"].as_str().expect("host");
    let port = params["port"].as_u64().expect("port");
    let database = params["database"].as_str().expect("database");

    let url = format!(
        "sqlserver://{}:{}@{}:{}/{}?Encrypt=true&TrustServerCertificate=true",
        url_encode_component(username),
        url_encode_component(password),
        host,
        port,
        url_encode_component(database),
    );
    let keyword = format!(
        "Server=tcp:{host},{port};Database={};User Id={};Password={};Encrypt=true;TrustServerCertificate=true;",
        brace_connection_value(database),
        brace_connection_value(username),
        brace_connection_value(password),
    );

    for (syntax, connection_string) in [("URL", url), ("keyword", keyword)] {
        let result = plugin.call_ok(
            "test_connection",
            json!({ "params": { "connection_string": connection_string } }),
        );
        assert_eq!(result, json!({ "success": true }), "{syntax} syntax");
    }
}

#[test]
fn database_user_lifecycle_privilege_diff_roles_and_ownership_guard() {
    const LOGIN: &str = "ss014_login";
    const USER: &str = "ss014_user";
    const ROLE: &str = "ss014_role";
    const OWNED_SCHEMA: &str = "ss014_owned";
    const PASSWORD_1: &str = "Ss014!InitialPass9";
    const PASSWORD_2: &str = "Ss014!ChangedPass9";

    let mut plugin = Plugin::with_scratch_database();
    plugin.execute(format!(
        "IF SCHEMA_ID(N'{OWNED_SCHEMA}') IS NOT NULL BEGIN \
             ALTER AUTHORIZATION ON SCHEMA::[{OWNED_SCHEMA}] TO [dbo]; \
             DROP SCHEMA [{OWNED_SCHEMA}]; \
         END; \
         IF DATABASE_PRINCIPAL_ID(N'{ROLE}') IS NOT NULL \
            AND DATABASE_PRINCIPAL_ID(N'{USER}') IS NOT NULL \
             ALTER ROLE [{ROLE}] DROP MEMBER [{USER}]; \
         IF DATABASE_PRINCIPAL_ID(N'{USER}') IS NOT NULL DROP USER [{USER}]; \
         IF DATABASE_PRINCIPAL_ID(N'{ROLE}') IS NOT NULL DROP ROLE [{ROLE}]; \
         IF SUSER_ID(N'{LOGIN}') IS NOT NULL DROP LOGIN [{LOGIN}]; \
         DROP TABLE IF EXISTS [{TEST_SCHEMA}].[ss014_permissions]; \
         CREATE TABLE [{TEST_SCHEMA}].[ss014_permissions] \
             (id INT PRIMARY KEY, value NVARCHAR(20) NOT NULL)"
    ));

    let catalog = plugin.call_ok("get_db_privilege_catalog", json!({}));
    assert!(catalog["database"]
        .as_array()
        .expect("database catalog")
        .contains(&json!("SELECT")));
    assert!(catalog["global"]
        .as_array()
        .expect("database-only catalog")
        .contains(&json!("SHOWPLAN")));
    assert!(catalog["table"]
        .as_array()
        .expect("object catalog")
        .contains(&json!("UPDATE")));

    plugin.call_ok(
        "create_db_user",
        json!({
            "params": connection_params(), "user": USER, "host": LOGIN,
            "password": PASSWORD_1
        }),
    );
    let users = plugin.call_ok("get_db_users", json!({ "params": connection_params() }));
    assert!(users
        .as_array()
        .expect("users array")
        .iter()
        .any(|account| { account == &json!({ "user": USER, "host": LOGIN, "locked": false }) }));

    plugin.call_ok(
        "set_db_user_password",
        json!({
            "params": connection_params(), "user": USER, "host": LOGIN,
            "password": PASSWORD_2
        }),
    );
    plugin.execute(format!(
        "CREATE ROLE [{ROLE}]; \
         GRANT UPDATE ON OBJECT::[{TEST_SCHEMA}].[ss014_permissions] TO [{ROLE}]; \
         ALTER ROLE [{ROLE}] ADD MEMBER [{USER}]"
    ));
    for (database, table, privileges) in [
        (Value::Null, Value::Null, vec!["SELECT"]),
        (json!(TEST_SCHEMA), Value::Null, vec!["EXECUTE"]),
        (
            json!(TEST_SCHEMA),
            json!("ss014_permissions"),
            vec!["SELECT", "INSERT"],
        ),
    ] {
        let request = json!({
            "params": connection_params(), "user": USER, "host": LOGIN,
            "database": database, "table": table,
            "privileges": privileges, "grant": true
        });
        plugin.call_ok("apply_db_user_privileges", request.clone());
        // Applying an already-satisfied request exercises the server-side diff.
        plugin.call_ok("apply_db_user_privileges", request);
    }

    let parsed = plugin.call_ok(
        "get_db_user_privileges",
        json!({ "params": connection_params(), "user": USER, "host": LOGIN }),
    );
    let object_scope = parsed
        .as_array()
        .expect("grant sets")
        .iter()
        .find(|scope| scope["database"] == TEST_SCHEMA && scope["table"] == "ss014_permissions")
        .expect("direct object grant");
    assert!(object_scope["privileges"]
        .as_array()
        .expect("object privileges")
        .contains(&json!("SELECT")));
    assert!(object_scope["privileges"]
        .as_array()
        .expect("object privileges")
        .contains(&json!("INSERT")));
    assert!(
        !object_scope["privileges"]
            .as_array()
            .expect("object privileges")
            .contains(&json!("UPDATE")),
        "inherited rights must not look direct"
    );

    let raw = plugin.call_ok(
        "get_db_user_grants",
        json!({ "params": connection_params(), "user": USER, "host": LOGIN }),
    );
    let raw = raw.as_array().expect("raw grants");
    assert!(raw.iter().any(|line| line
        .as_str()
        .is_some_and(|line| { line.contains("ROLE MEMBERSHIP") && line.contains(ROLE) })));
    assert!(raw.iter().any(|line| line
        .as_str()
        .is_some_and(|line| { line.contains("INHERITED VIA ROLE") && line.contains("UPDATE") })));

    plugin.call_ok(
        "apply_db_user_privileges",
        json!({
            "params": connection_params(), "user": USER, "host": LOGIN,
            "database": TEST_SCHEMA, "table": "ss014_permissions",
            "privileges": ["SELECT", "INSERT"], "grant": false
        }),
    );
    plugin.execute(format!(
        "DENY DELETE ON OBJECT::[{TEST_SCHEMA}].[ss014_permissions] TO [{USER}]"
    ));
    let deny_error = plugin.call_error(
        "apply_db_user_privileges",
        json!({
            "params": connection_params(), "user": USER, "host": LOGIN,
            "database": TEST_SCHEMA, "table": "ss014_permissions",
            "privileges": ["DELETE"], "grant": true
        }),
    );
    assert!(deny_error.contains("DENY"), "{deny_error}");
    plugin.execute(format!(
        "REVOKE DELETE ON OBJECT::[{TEST_SCHEMA}].[ss014_permissions] FROM [{USER}]"
    ));
    plugin.execute(format!(
        "CREATE SCHEMA [{OWNED_SCHEMA}] AUTHORIZATION [{USER}]"
    ));

    let ownership_error = plugin.call_error(
        "drop_db_user",
        json!({ "params": connection_params(), "user": USER, "host": LOGIN }),
    );
    assert!(
        ownership_error.contains("schema or object"),
        "{ownership_error}"
    );
    assert!(ownership_error.contains("owns"), "{ownership_error}");

    plugin.execute(format!(
        "ALTER AUTHORIZATION ON SCHEMA::[{OWNED_SCHEMA}] TO [dbo]; \
         DROP SCHEMA [{OWNED_SCHEMA}]; \
         ALTER ROLE [{ROLE}] DROP MEMBER [{USER}]; \
         DROP ROLE [{ROLE}]"
    ));
    plugin.call_ok(
        "drop_db_user",
        json!({ "params": connection_params(), "user": USER, "host": LOGIN }),
    );
    let users = plugin.call_ok("get_db_users", json!({ "params": connection_params() }));
    assert!(!users
        .as_array()
        .expect("users array")
        .iter()
        .any(|account| { account["user"] == USER || account["host"] == LOGIN }));
    let login = plugin.execute(format!(
        "SELECT COUNT(*) AS login_count FROM sys.server_principals WHERE name = N'{LOGIN}'"
    ));
    assert_eq!(login["rows"], json!([[0]]));
}
