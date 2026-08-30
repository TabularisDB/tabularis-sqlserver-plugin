//! JSON-RPC dispatch and response helpers.

use serde::Serialize;
use serde_json::{json, Value};

use crate::connection::resolve_connection_params;
use crate::handlers::{crud, ddl, metadata, query, routines, triggers, views};
use crate::models::ConnectionParams;

/// Parse one JSON-RPC line and return the response value (serialised
/// downstream by `main.rs`). Never panics — parse errors and method
/// failures are surfaced as JSON-RPC error responses.
pub async fn handle_line(line: &str) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => return error_response(Value::Null, -32700, &format!("parse error: {err}")),
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method.as_str() {
        "initialize" => ok_response(id, Value::Null),
        "ping" => query::ping(id, &params).await,
        "test_connection" => query::test_connection(id, &params).await,

        // Metadata.
        "get_databases" => metadata::get_databases(id, &params).await,
        "get_schemas" => metadata::get_schemas(id, &params).await,
        "get_tables" => metadata::get_tables(id, &params).await,
        "get_columns" => metadata::get_columns(id, &params).await,
        "get_foreign_keys" => metadata::get_foreign_keys(id, &params).await,
        "get_indexes" => metadata::get_indexes(id, &params).await,
        "get_schema_snapshot" => metadata::get_schema_snapshot(id, &params).await,
        "get_all_columns_batch" => metadata::get_all_columns_batch(id, &params).await,
        "get_all_foreign_keys_batch" => metadata::get_all_foreign_keys_batch(id, &params).await,
        "get_ai_schema_context" => metadata::get_ai_schema_context(id, &params).await,

        // Views.
        "get_views" => views::get_views(id, &params).await,
        "get_view_definition" => views::get_view_definition(id, &params).await,
        "get_view_columns" => views::get_view_columns(id, &params).await,
        "create_view" => views::create_view(id, &params).await,
        "alter_view" => views::alter_view(id, &params).await,
        "drop_view" => views::drop_view(id, &params).await,

        // Routines.
        "get_routines" => routines::get_routines(id, &params).await,
        "get_routine_parameters" => routines::get_routine_parameters(id, &params).await,
        "get_routine_definition" => routines::get_routine_definition(id, &params).await,
        "build_routine_call_sql" => routines::build_routine_call_sql(id, &params).await,
        "routine_create_template" => routines::routine_create_template(id, &params).await,
        "get_routine_edit_script" => routines::get_routine_edit_script(id, &params).await,
        "drop_routine" => routines::drop_routine(id, &params).await,

        // Triggers.
        "get_triggers" => triggers::get_triggers(id, &params).await,
        "get_trigger_definition" => triggers::get_trigger_definition(id, &params).await,
        "create_trigger" => triggers::create_trigger(id, &params).await,
        "drop_trigger" => triggers::drop_trigger(id, &params).await,

        // Query execution.
        "execute_query" => query::execute_query(id, &params).await,
        "execute_query_batch" => query::execute_query_batch(id, &params).await,
        "explain_query" => query::explain_query(id, &params).await,

        // CRUD.
        "insert_record" => crud::insert_record(id, &params).await,
        "update_record" => crud::update_record(id, &params).await,
        "delete_record" => crud::delete_record(id, &params).await,

        // DDL.
        "get_create_table_sql" => ddl::get_create_table_sql(id, &params).await,
        "get_add_column_sql" => ddl::get_add_column_sql(id, &params).await,
        "get_alter_column_sql" => ddl::get_alter_column_sql(id, &params).await,
        "get_create_index_sql" => ddl::get_create_index_sql(id, &params).await,
        "get_create_foreign_key_sql" => ddl::get_create_foreign_key_sql(id, &params).await,
        "drop_index" => ddl::drop_index(id, &params).await,
        "drop_foreign_key" => ddl::drop_foreign_key(id, &params).await,

        other => not_implemented(id, other),
    }
}

pub fn ok_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "result": result,
        "id": id,
    })
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": message },
        "id": id,
    })
}

pub fn not_implemented(id: Value, method: &str) -> Value {
    error_response(
        id,
        -32601,
        &format!("method '{method}' is not implemented by this plugin"),
    )
}

/// Turn a driver outcome into a JSON-RPC response.
pub fn respond<T: Serialize>(id: Value, outcome: Result<T, String>) -> Value {
    match outcome {
        Ok(result) => match serde_json::to_value(result) {
            Ok(value) => ok_response(id, value),
            Err(err) => error_response(id, -32603, &format!("serialization failed: {err}")),
        },
        Err(message) => error_response(id, -32000, &message),
    }
}

/// Deserialize the nested `params.params` connection object every RPC method
/// receives.
pub fn conn_params(params: &Value) -> Result<ConnectionParams, String> {
    let params = serde_json::from_value(params.get("params").cloned().unwrap_or(Value::Null))
        .map_err(|err| format!("invalid connection params: {err}"))?;
    resolve_connection_params(&params)
        .map_err(|error| format!("invalid connection params: {error}"))
}

pub fn opt_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

pub fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(params, key).ok_or_else(|| format!("missing required string parameter '{key}'"))
}

/// Deserialize a required parameter into a concrete type.
pub fn req_field<T: serde::de::DeserializeOwned>(params: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(params.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|err| format!("invalid parameter '{key}': {err}"))
}
