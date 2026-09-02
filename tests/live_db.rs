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

    fn call(&mut self, method: &str, params: Value) -> Value {
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

        let mut response_line = String::new();
        self.stdout
            .read_line(&mut response_line)
            .expect("read response from plugin stdout");
        assert!(
            !response_line.is_empty(),
            "plugin exited without a response"
        );
        let response: Value =
            serde_json::from_str(response_line.trim()).expect("parse JSON-RPC response");
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

    fn execute(&mut self, query: impl Into<String>) -> Value {
        self.call_ok(
            "execute_query",
            json!({ "params": connection_params(), "query": query.into() }),
        )
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
fn syntax_and_constraint_errors_surface_and_pooled_connection_recovers() {
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
        json!({ "params": connection_params(), "query": "SELEC definitely_invalid" }),
    );
    assert!(!syntax_error.is_empty());
    let after_syntax = plugin.execute("SELECT CAST(1 AS INT) AS connection_ok");
    assert_eq!(after_syntax["rows"], json!([[1]]));

    let constraint_error = plugin.call_error(
        "execute_query",
        json!({
            "params": connection_params(),
            "query": format!("INSERT INTO [{TEST_SCHEMA}].[errors] VALUES (2, 10)")
        }),
    );
    assert!(!constraint_error.is_empty());
    let after_constraint = plugin.execute(format!(
        "SELECT COUNT(*) AS row_count FROM [{TEST_SCHEMA}].[errors]"
    ));
    assert_eq!(after_constraint["rows"], json!([[1]]));
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
