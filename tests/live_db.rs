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
fn pagination_returns_ordered_pages_has_more_and_explicit_unknown_total() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table("pagination", "id INT PRIMARY KEY");
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[pagination] VALUES (1), (2), (3), (4), (5)"
    ));
    let query = format!("SELECT id FROM [{TEST_SCHEMA}].[pagination] ORDER BY id");

    let page_one = plugin.call_ok(
        "execute_query",
        json!({ "params": connection_params(), "query": query, "limit": 2, "page": 1 }),
    );
    assert_eq!(page_one["rows"], json!([[1], [2]]));
    assert_eq!(page_one["pagination"]["page"], 1);
    assert_eq!(page_one["pagination"]["page_size"], 2);
    assert_eq!(page_one["pagination"]["has_more"], true);
    assert_eq!(page_one["pagination"]["total_rows"], Value::Null);

    let page_two = plugin.call_ok(
        "execute_query",
        json!({ "params": connection_params(), "query": query, "limit": 2, "page": 2 }),
    );
    assert_eq!(page_two["rows"], json!([[3], [4]]));
    assert_eq!(page_two["pagination"]["page"], 2);
    assert_eq!(page_two["pagination"]["has_more"], true);
    assert_eq!(page_two["pagination"]["total_rows"], Value::Null);
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
fn explain_query_returns_showplan_xml_for_estimate_and_analyze() {
    let mut plugin = Plugin::with_scratch_database();
    plugin.reset_table("explain", "id INT PRIMARY KEY, value INT NOT NULL");
    plugin.execute(format!(
        "INSERT INTO [{TEST_SCHEMA}].[explain] VALUES (1, 10), (2, 20)"
    ));
    let query = format!("SELECT value FROM [{TEST_SCHEMA}].[explain] WHERE id = 1");

    for analyze in [false, true] {
        let plan = plugin.call_ok(
            "explain_query",
            json!({
                "params": connection_params(),
                "query": query,
                "analyze": analyze
            }),
        );
        let raw = plan["raw_output"]
            .as_str()
            .expect("parsed plan must retain its raw SHOWPLAN XML");
        assert!(raw.contains("ShowPlanXML"), "analyze={analyze}: {raw}");
        assert_eq!(plan["driver"], "sqlserver");
    }
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
fn connection_string_only_is_rejected_until_ss_011() {
    let mut plugin = Plugin::with_scratch_database();
    let params = connection_params();
    let connection_string = format!(
        "sqlserver://{}:{}@{}:{}/{}",
        params["username"].as_str().expect("username"),
        params["password"].as_str().expect("password"),
        params["host"].as_str().expect("host"),
        params["port"].as_u64().expect("port"),
        params["database"].as_str().expect("database"),
    );

    // TODO(SS-011): change this to call_ok once ConnectionParams accepts and
    // parses connection_string. Today serde ignores the field and the plugin
    // attempts its empty/default discrete connection, which must fail.
    let error = plugin.call_error(
        "test_connection",
        json!({ "params": { "connection_string": connection_string } }),
    );
    assert!(!error.is_empty());
}
