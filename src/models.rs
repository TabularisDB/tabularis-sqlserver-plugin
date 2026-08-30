//! Data models shared with the Tabularis host.
//!
//! These mirror the host's serde shapes so the JSON the plugin emits
//! deserializes into the host's structs unchanged. Fields the SQL Server
//! driver does not use are omitted; serde ignores unknown JSON fields on
//! deserialization and `#[serde(default)]` covers fields the host may leave
//! out.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The host sends either a single database name or a list of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DatabaseSelection {
    Single(String),
    Multiple(Vec<String>),
}

impl DatabaseSelection {
    pub fn primary(&self) -> &str {
        match self {
            DatabaseSelection::Single(s) => s.as_str(),
            DatabaseSelection::Multiple(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
        }
    }
}

impl std::fmt::Display for DatabaseSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.primary())
    }
}

impl Default for DatabaseSelection {
    fn default() -> Self {
        DatabaseSelection::Single(String::new())
    }
}

/// Connection parameters forwarded by the host on every RPC call.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct ConnectionParams {
    pub driver: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: DatabaseSelection,
    pub ssl_mode: Option<String>,
    pub ssl_ca: Option<String>,
    pub ssl_cert: Option<String>,
    pub ssl_key: Option<String>,
    /// URL or ADO.NET/ODBC keyword connection string. It is parsed and
    /// reconciled with the discrete fields before a pool is selected.
    pub connection_string: Option<String>,
    /// SQL run on every new physical connection in the pool. Statements are
    /// separated by `;`. Runs per pooled connection so the setting applies to
    /// every query regardless of which connection the pool hands out.
    pub startup_script: Option<String>,
    /// Connection ID for stable pooling (set at runtime by the host).
    pub connection_id: Option<String>,
}

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
    /// Extra result sets produced by a single statement beyond the first one.
    /// The first result set stays in `columns` / `rows` so consumers unaware
    /// of multi-result statements keep working unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_results: Option<Vec<QueryResult>>,
}

/// One statement's outcome within an `execute_query_batch` call. Exactly one
/// of `result` / `error` is `Some`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchStatementResult {
    pub result: Option<QueryResult>,
    pub error: Option<String>,
    pub execution_time_ms: Option<f64>,
}

impl BatchStatementResult {
    pub fn from_outcome(start: std::time::Instant, outcome: Result<QueryResult, String>) -> Self {
        let execution_time_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
        match outcome {
            Ok(r) => Self {
                result: Some(r),
                error: None,
                execution_time_ms,
            },
            Err(e) => Self {
                result: None,
                error: Some(e),
                execution_time_ms,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub foreign_keys: Vec<ForeignKey>,
}

/// Bounded schema metadata prepared for AI features (`get_ai_schema_context`).
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

/// One argument for invoking a stored routine, as collected by the
/// run-routine UI. `value: None` means SQL `NULL`; `is_raw` skips string
/// quoting so numbers and expressions pass through verbatim.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutineCallArg {
    pub name: String,
    pub mode: String, // "IN", "OUT", "INOUT"
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub is_raw: bool,
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

/// One database principal backed by a SQL Server login. The host's `host`
/// field carries the mapped login name for SQL Server.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbUserInfo {
    pub user: String,
    pub host: String,
    pub locked: bool,
}

/// SQL Server privilege names accepted by the three host scope lists.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DbPrivilegeCatalog {
    pub database: Vec<String>,
    pub global: Vec<String>,
    pub table: Vec<String>,
}

/// Direct grants at one database, schema, or object scope. SQL Server maps
/// those levels to `(None, None)`, `(Some(schema), None)`, and
/// `(Some(schema), Some(object))` on the host wire shape.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DbUserGrantSet {
    pub database: Option<String>,
    pub table: Option<String>,
    pub privileges: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ColumnDefinition {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_pk: bool,
    pub is_auto_increment: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataTypeInfo {
    pub name: String,
    pub category: String,
    pub requires_length: bool,
    pub requires_precision: bool,
    pub default_length: Option<String>,
    #[serde(default)]
    pub supports_auto_increment: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_extension: Option<String>,
}

/// `pk_map` as sent by the host for update/delete: primary-key column → value.
pub type PkMap = HashMap<String, serde_json::Value>;
