//! Recorded JSON-RPC response conformance against Tabularis host models.
//!
//! The model definitions below are copied verbatim from
//! `tabularis/src-tauri/src/models.rs` at host commit
//! `ba0463d3b861ec8fad110126c67e3fc12bac9839`. Re-sync them and regenerate
//! `tests/fixtures/conformance/` with `python3 tests/capture_conformance.py`
//! whenever the host models or plugin RPC surface changes.

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

// BEGIN verbatim host model definitions.

#[derive(Debug, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub is_pk: bool,
    pub is_nullable: bool,
    pub is_auto_increment: bool,
    #[serde(default)]
    pub is_generated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_maximum_length: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    pub column_name: String,
    pub ref_table: String,
    pub ref_column: String,
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub column_name: String,
    pub is_unique: bool,
    pub is_primary: bool,
    pub seq_in_index: i32,
    #[serde(default)]
    pub is_expression: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Pagination {
    pub page: u32,
    pub page_size: u32,
    pub total_rows: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected_rows: u64,
    #[serde(default)]
    pub truncated: bool,
    pub pagination: Option<Pagination>,
    /// Extra result sets produced by a single statement beyond the first one,
    /// e.g. a MySQL `CALL` to a stored procedure containing multiple `SELECT`s.
    /// The first result set stays in `columns` / `rows` so consumers unaware
    /// of multi-result statements keep working unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_results: Option<Vec<QueryResult>>,
}

/// One statement's outcome within an `execute_batch` call. Exactly one of
/// `result` / `error` is `Some` — kept as separate optionals (not a tagged
/// enum) so the TypeScript side can do `if (item.error) ... else ... item.result`
/// without a discriminated-union helper. Use [`BatchStatementResult::from_outcome`]
/// to construct so the invariant is enforced.
///
/// `execution_time_ms` is measured server-side because a batch is one
/// Tauri round-trip but the history UI wants per-statement timings.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchStatementResult {
    pub result: Option<QueryResult>,
    pub error: Option<String>,
    pub execution_time_ms: Option<f64>,
}

/// Raw EXPLAIN output produced by a built-in driver.
///
/// Parsing lives in the `@tabularis/explain` TypeScript package
/// (`parseRawExplain`): a driver's job ends at handing over the payload it
/// obtained — text, a JSON document, or decoded rows re-serialised as a JSON
/// array — plus the format tag naming what it is.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RawExplainOutput {
    /// Driver id of the engine that produced the payload ("postgres", …).
    pub engine: String,
    /// Wire format tag understood by `@tabularis/explain`:
    /// `postgres-json`, `mysql-json`, `mysql-analyze-text`,
    /// `mysql-tabular-rows` or `sqlite-eqp-rows`.
    pub format: String,
    /// The untouched payload: text, a JSON document, or rows as a JSON array.
    pub payload: String,
    pub original_query: String,
}

/// What `explain_query` hands to the frontend: a raw payload for a registered
/// parser, or a plan a plugin driver already parsed. Both plugin result shapes
/// remain supported for backwards compatibility.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExplainQueryOutput {
    Raw { raw: RawExplainOutput },
    Plan { plan: serde_json::Value },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub foreign_keys: Vec<ForeignKey>,
}

/// Bounded schema metadata prepared by a database driver for AI features.
/// The host remains responsible for rendering this structured data into a
/// provider-agnostic prompt.
#[derive(Debug, Serialize, Deserialize)]
pub struct AiSchemaContext {
    pub tables: Vec<TableSchema>,
    pub total_table_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutineInfo {
    pub name: String,
    pub routine_type: String, // "PROCEDURE" | "FUNCTION"
    pub definition: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutineParameter {
    pub name: String,
    pub data_type: String,
    pub mode: String, // "IN", "OUT", "INOUT"
    pub ordinal_position: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ViewInfo {
    pub name: String,
    pub definition: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub name: String,
    pub table_name: String,
    pub event: String,  // e.g. "INSERT", "UPDATE", "DELETE", "INSERT OR UPDATE"
    pub timing: String, // "BEFORE", "AFTER", "INSTEAD OF"
    pub definition: Option<String>,
}

/// One database account as listed by the server (MySQL/MariaDB:
/// `mysql.user` rows, identified by the `user`@`host` pair).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUserInfo {
    pub user: String,
    pub host: String,
    /// Account is locked (`ALTER USER ... ACCOUNT LOCK`); `false` when the
    /// server does not expose the flag.
    pub locked: bool,
}

/// The privilege keywords a driver accepts in `apply_db_user_privileges`,
/// split by scope. Sent to the frontend so the privilege editor renders the
/// dialect's own catalog instead of hardcoding one.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DbPrivilegeCatalog {
    /// Privileges valid at the database scope (and also globally).
    pub database: Vec<String>,
    /// Privileges valid only at the global scope.
    pub global: Vec<String>,
    /// Privileges valid at the table scope.
    pub table: Vec<String>,
}

/// One account's privileges on one scope, parsed from the server's grant
/// metadata (MySQL: one `SHOW GRANTS` line). `database == None` is the
/// global scope; `table` is only ever `Some` when `database` is.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DbUserGrantSet {
    pub database: Option<String>,
    pub table: Option<String>,
    /// Canonical privilege keywords, `GRANT OPTION` included as an entry.
    pub privileges: Vec<String>,
}

// END verbatim host model definitions.

#[derive(Debug, Deserialize)]
struct RpcResponse {
    jsonrpc: String,
    result: Value,
    id: u64,
}

const DELIBERATELY_UNSUPPORTED: &[&str] = &[
    "get_materialized_views",
    "get_materialized_view_columns",
    "get_materialized_view_definition",
    "refresh_materialized_view",
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance")
}

fn fixture_methods() -> BTreeSet<String> {
    fs::read_dir(fixture_dir())
        .expect("read conformance fixture directory")
        .filter_map(|entry| {
            let path = entry.expect("read fixture entry").path();
            (path.extension().and_then(|extension| extension.to_str()) == Some("json")).then(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .expect("fixture names must be UTF-8")
                    .to_string()
            })
        })
        .collect()
}

fn implemented_methods() -> BTreeSet<String> {
    let unsupported: BTreeSet<_> = DELIBERATELY_UNSUPPORTED.iter().copied().collect();
    include_str!("../src/rpc.rs")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('"')?
                .split_once("\" =>")
                .map(|(method, _)| method)
        })
        .filter(|method| !unsupported.contains(method))
        .map(str::to_string)
        .collect()
}

fn fixture_result(method: &str) -> Value {
    let path = fixture_dir().join(format!("{method}.json"));
    let fixture: RpcResponse = serde_json::from_str(
        &fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("deserialize {}: {error}", path.display()));
    assert_eq!(fixture.jsonrpc, "2.0", "{method} JSON-RPC version");
    assert_eq!(fixture.id, 1, "{method} JSON-RPC id");
    fixture.result
}

fn assert_deserializes<T: DeserializeOwned>(methods: &[&str], checked: &mut BTreeSet<String>) {
    for method in methods {
        serde_json::from_value::<T>(fixture_result(method))
            .unwrap_or_else(|error| panic!("{method} does not match the host target: {error}"));
        assert!(checked.insert((*method).to_string()), "duplicate {method}");
    }
}

#[test]
fn fixtures_cover_every_implemented_rpc() {
    assert_eq!(fixture_methods(), implemented_methods());
}

#[test]
fn every_recorded_result_deserializes_into_the_host_target() {
    let mut checked = BTreeSet::new();

    assert_deserializes::<Value>(&["test_connection"], &mut checked);
    assert_deserializes::<()>(
        &[
            "initialize",
            "ping",
            "shutdown",
            "create_view",
            "alter_view",
            "drop_view",
            "drop_routine",
            "create_trigger",
            "drop_trigger",
            "create_db_user",
            "drop_db_user",
            "set_db_user_password",
            "apply_db_user_privileges",
            "save_blob_to_file",
            "drop_index",
            "drop_foreign_key",
        ],
        &mut checked,
    );
    assert_deserializes::<String>(
        &[
            "get_view_definition",
            "get_routine_definition",
            "build_routine_call_sql",
            "routine_create_template",
            "get_routine_edit_script",
            "get_trigger_definition",
            "fetch_blob_as_data_url",
        ],
        &mut checked,
    );
    assert_deserializes::<u64>(
        &["insert_record", "update_record", "delete_record"],
        &mut checked,
    );
    assert_deserializes::<Vec<String>>(
        &[
            "get_databases",
            "get_schemas",
            "get_db_user_grants",
            "get_create_table_sql",
            "get_add_column_sql",
            "get_alter_column_sql",
            "get_create_index_sql",
            "get_create_foreign_key_sql",
        ],
        &mut checked,
    );
    assert_deserializes::<Vec<TableInfo>>(&["get_tables"], &mut checked);
    assert_deserializes::<Vec<TableColumn>>(&["get_columns", "get_view_columns"], &mut checked);
    assert_deserializes::<Vec<ForeignKey>>(&["get_foreign_keys"], &mut checked);
    assert_deserializes::<Vec<Index>>(&["get_indexes"], &mut checked);
    assert_deserializes::<Vec<TableSchema>>(&["get_schema_snapshot"], &mut checked);
    assert_deserializes::<HashMap<String, Vec<TableColumn>>>(
        &["get_all_columns_batch"],
        &mut checked,
    );
    assert_deserializes::<HashMap<String, Vec<ForeignKey>>>(
        &["get_all_foreign_keys_batch"],
        &mut checked,
    );
    assert_deserializes::<AiSchemaContext>(&["get_ai_schema_context"], &mut checked);
    assert_deserializes::<Vec<ViewInfo>>(&["get_views"], &mut checked);
    assert_deserializes::<Vec<RoutineInfo>>(&["get_routines"], &mut checked);
    assert_deserializes::<Vec<RoutineParameter>>(&["get_routine_parameters"], &mut checked);
    assert_deserializes::<Vec<TriggerInfo>>(&["get_triggers"], &mut checked);
    assert_deserializes::<DbPrivilegeCatalog>(&["get_db_privilege_catalog"], &mut checked);
    assert_deserializes::<Vec<DbUserInfo>>(&["get_db_users"], &mut checked);
    assert_deserializes::<Vec<DbUserGrantSet>>(&["get_db_user_privileges"], &mut checked);
    assert_deserializes::<QueryResult>(&["execute_query"], &mut checked);
    assert_deserializes::<Vec<BatchStatementResult>>(&["execute_query_batch"], &mut checked);
    assert_deserializes::<RawExplainOutput>(&["explain_query"], &mut checked);

    assert_eq!(checked, implemented_methods());
}

#[test]
fn drift_prone_wire_fields_are_exercised() {
    let query: QueryResult = serde_json::from_value(fixture_result("execute_query")).unwrap();
    assert!(query.additional_results.is_some());
    let batch: Vec<BatchStatementResult> =
        serde_json::from_value(fixture_result("execute_query_batch")).unwrap();
    assert!(batch[0]
        .result
        .as_ref()
        .and_then(|result| result.pagination.as_ref())
        .is_some());
    assert!(batch[1].error.is_some());

    let columns: Vec<TableColumn> = serde_json::from_value(fixture_result("get_columns")).unwrap();
    let label = columns
        .iter()
        .find(|column| column.name == "label")
        .unwrap();
    assert_eq!(label.character_maximum_length, Some(42));
    assert_eq!(label.default_value.as_deref(), Some("(N'pending')"));
    let parent = columns
        .iter()
        .find(|column| column.name == "parent_id")
        .unwrap();
    assert_eq!(parent.character_maximum_length, None);
    assert_eq!(parent.default_value, None);
    assert!(columns.iter().any(|column| column.is_generated));

    let foreign_keys: Vec<ForeignKey> =
        serde_json::from_value(fixture_result("get_foreign_keys")).unwrap();
    assert_eq!(foreign_keys[0].on_delete.as_deref(), Some("SET NULL"));
    assert_eq!(foreign_keys[0].on_update.as_deref(), Some("NO ACTION"));
    let mut nullable_foreign_keys = fixture_result("get_foreign_keys");
    nullable_foreign_keys[0]["on_delete"] = Value::Null;
    nullable_foreign_keys[0]["on_update"] = Value::Null;
    let nullable: Vec<ForeignKey> = serde_json::from_value(nullable_foreign_keys).unwrap();
    assert!(nullable[0].on_delete.is_none() && nullable[0].on_update.is_none());

    let triggers: Vec<TriggerInfo> =
        serde_json::from_value(fixture_result("get_triggers")).unwrap();
    let vocabulary: BTreeSet<_> = triggers
        .iter()
        .map(|trigger| (trigger.timing.as_str(), trigger.event.as_str()))
        .collect();
    assert!(vocabulary.contains(&("AFTER", "INSERT OR UPDATE")));
    assert!(vocabulary.contains(&("INSTEAD OF", "DELETE")));

    let parameters: Vec<RoutineParameter> =
        serde_json::from_value(fixture_result("get_routine_parameters")).unwrap();
    assert_eq!(parameters[0].mode, "IN");
    assert_eq!(parameters[1].mode, "INOUT");

    let context: AiSchemaContext =
        serde_json::from_value(fixture_result("get_ai_schema_context")).unwrap();
    assert_eq!(context.tables.len(), 3);
    assert_eq!(context.total_table_count, 5);

    let raw: RawExplainOutput = serde_json::from_value(fixture_result("explain_query")).unwrap();
    assert_eq!(raw.engine, "sqlserver");
    assert_eq!(raw.format, "sqlserver-showplan-xml");
    assert!(raw.payload.contains("<ShowPlanXML"));
    let host_output = ExplainQueryOutput::Raw { raw };
    assert!(matches!(host_output, ExplainQueryOutput::Raw { .. }));
}
