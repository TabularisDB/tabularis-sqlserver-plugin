//! JSON-RPC dispatch and response helpers.

use serde::Serialize;
use serde_json::{json, Value};

use crate::connection::resolve_connection_params;
use crate::driver::error::redact_connection_secrets;
use crate::handlers::{blob, crud, ddl, metadata, query, routines, triggers, users, views};
use crate::models::ConnectionParams;
use crate::{pool_manager, settings};

const PLUGIN_NAME: &str = "SQL Server plugin";

/// Host RPCs that SQL Server deliberately does not implement.
///
/// Keep this list limited to methods present in the host protocol. The
/// coverage tests below require every host method to be dispatched or listed
/// here with a non-empty reason.
const NOT_IMPLEMENTED: &[(&str, &str)] = &[
    (
        "get_materialized_views",
        "SQL Server has indexed views, not materialized views; indexed views are maintained synchronously",
    ),
    (
        "get_materialized_view_columns",
        "SQL Server has indexed views, not materialized views; indexed views are maintained synchronously",
    ),
    (
        "get_materialized_view_definition",
        "SQL Server has indexed views, not materialized views; indexed views are maintained synchronously",
    ),
    (
        "refresh_materialized_view",
        "SQL Server indexed views are maintained synchronously and cannot be refreshed as materialized views",
    ),
];

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
        "initialize" => {
            // Initialization is intentionally infallible: malformed known
            // values warn and fall back, while unknown keys are ignored.
            settings::initialize(&params);
            ok_response(id, Value::Null)
        }
        "ping" => query::ping(id, &params).await,
        "test_connection" => query::test_connection(id, &params).await,
        "shutdown" => {
            pool_manager::shutdown().await;
            ok_response(id, Value::Null)
        }

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

        // Database users and privileges.
        "get_db_privilege_catalog" => users::get_db_privilege_catalog(id).await,
        "get_db_users" => users::get_db_users(id, &params).await,
        "create_db_user" => users::create_db_user(id, &params).await,
        "drop_db_user" => users::drop_db_user(id, &params).await,
        "set_db_user_password" => users::set_db_user_password(id, &params).await,
        "get_db_user_grants" => users::get_db_user_grants(id, &params).await,
        "get_db_user_privileges" => users::get_db_user_privileges(id, &params).await,
        "apply_db_user_privileges" => users::apply_db_user_privileges(id, &params).await,

        // Query execution.
        "execute_query" => query::execute_query(id, &params).await,
        "execute_query_batch" => query::execute_query_batch(id, &params).await,
        "explain_query" => query::explain_query(id, &params).await,

        // CRUD.
        "insert_record" => crud::insert_record(id, &params).await,
        "update_record" => crud::update_record(id, &params).await,
        "delete_record" => crud::delete_record(id, &params).await,

        // BLOB export and preview.
        "save_blob_to_file" => blob::save_blob_to_file(id, &params).await,
        "fetch_blob_as_data_url" => blob::fetch_blob_as_data_url(id, &params).await,

        // DDL.
        "get_create_table_sql" => ddl::get_create_table_sql(id, &params).await,
        "get_add_column_sql" => ddl::get_add_column_sql(id, &params).await,
        "get_alter_column_sql" => ddl::get_alter_column_sql(id, &params).await,
        "get_create_index_sql" => ddl::get_create_index_sql(id, &params).await,
        "get_create_foreign_key_sql" => ddl::get_create_foreign_key_sql(id, &params).await,
        "drop_index" => ddl::drop_index(id, &params).await,
        "drop_foreign_key" => ddl::drop_foreign_key(id, &params).await,

        other => match not_implemented_reason(other) {
            Some(reason) => not_implemented(id, other, reason),
            None => method_not_found(id, other),
        },
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

fn not_implemented_reason(method: &str) -> Option<&'static str> {
    NOT_IMPLEMENTED
        .iter()
        .find_map(|(candidate, reason)| (*candidate == method).then_some(*reason))
}

fn not_implemented(id: Value, method: &str, reason: &str) -> Value {
    error_response(
        id,
        -32601,
        &format!(
            "Method not found (-32601): '{method}' is not implemented by {PLUGIN_NAME}: {reason}"
        ),
    )
}

fn method_not_found(id: Value, method: &str) -> Value {
    error_response(
        id,
        -32601,
        &format!(
            "Method not found (-32601): '{method}' is not implemented by {PLUGIN_NAME}: unknown JSON-RPC method"
        ),
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
    let params: ConnectionParams =
        serde_json::from_value(params.get("params").cloned().unwrap_or(Value::Null))
            .map_err(|err| format!("invalid connection params: {err}"))?;
    resolve_connection_params(&params).map_err(|error| {
        redact_connection_secrets(format!("invalid connection params: {error}"), &params)
    })
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Snapshot extracted from every literal `PluginProcess::call` and
    /// `call_with_timeout` in Tabularis
    /// `src-tauri/src/plugins/driver.rs` at core commit 9e6975aa.
    const HOST_METHODS: &[&str] = &[
        "initialize",
        "ping",
        "test_connection",
        "get_databases",
        "get_schemas",
        "get_tables",
        "get_columns",
        "get_foreign_keys",
        "get_indexes",
        "get_views",
        "get_view_definition",
        "get_view_columns",
        "create_view",
        "alter_view",
        "drop_view",
        "get_materialized_views",
        "get_materialized_view_columns",
        "get_materialized_view_definition",
        "refresh_materialized_view",
        "get_routines",
        "get_routine_parameters",
        "get_routine_definition",
        "build_routine_call_sql",
        "routine_create_template",
        "get_routine_edit_script",
        "drop_routine",
        "execute_query",
        "execute_query_batch",
        "explain_query",
        "insert_record",
        "update_record",
        "delete_record",
        "save_blob_to_file",
        "fetch_blob_as_data_url",
        "get_create_table_sql",
        "get_add_column_sql",
        "get_alter_column_sql",
        "get_create_index_sql",
        "get_create_foreign_key_sql",
        "drop_index",
        "drop_foreign_key",
        "get_triggers",
        "get_db_privilege_catalog",
        "get_db_users",
        "get_db_user_grants",
        "create_db_user",
        "drop_db_user",
        "set_db_user_password",
        "get_db_user_privileges",
        "apply_db_user_privileges",
        "get_trigger_definition",
        "create_trigger",
        "drop_trigger",
        "get_schema_snapshot",
        "get_ai_schema_context",
        "get_all_columns_batch",
        "get_all_foreign_keys_batch",
    ];

    fn dispatched_match_arms() -> BTreeSet<&'static str> {
        include_str!("rpc.rs")
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix('"')?
                    .split_once("\" =>")
                    .map(|(method, _)| method)
            })
            .collect()
    }

    fn handle_line_on_worker_stack(line: String) -> Value {
        std::thread::Builder::new()
            .stack_size(crate::WORKER_STACK_SIZE)
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(Box::pin(handle_line(&line)))
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn every_host_method_is_dispatched_or_deliberately_not_implemented() {
        let dispatched = dispatched_match_arms();
        let host_methods: BTreeSet<_> = HOST_METHODS.iter().copied().collect();
        let not_implemented: BTreeSet<_> = NOT_IMPLEMENTED
            .iter()
            .map(|(method, reason)| {
                assert!(!reason.trim().is_empty(), "{method} needs a reason");
                *method
            })
            .collect();

        assert_eq!(
            host_methods.len(),
            HOST_METHODS.len(),
            "duplicate host method"
        );
        assert_eq!(not_implemented.len(), NOT_IMPLEMENTED.len());

        let uncovered: Vec<_> = host_methods
            .difference(&dispatched)
            .filter(|method| !not_implemented.contains(*method))
            .copied()
            .collect();
        assert!(
            uncovered.is_empty(),
            "host methods are neither dispatched nor deliberately unsupported: {uncovered:?}"
        );

        let stale_exclusions: Vec<_> = not_implemented.difference(&host_methods).copied().collect();
        assert!(
            stale_exclusions.is_empty(),
            "NOT_IMPLEMENTED contains methods outside the host contract: {stale_exclusions:?}"
        );

        let plugin_only: Vec<_> = dispatched
            .difference(&host_methods)
            .filter(|method| **method != "shutdown")
            .copied()
            .collect();
        assert!(
            plugin_only.is_empty(),
            "dispatch contains methods outside the host contract: {plugin_only:?}"
        );
        assert!(dispatched.contains("shutdown"));
    }

    #[test]
    fn deliberate_exclusions_return_named_reasoned_errors() {
        for (method, reason) in NOT_IMPLEMENTED {
            let request = json!({ "jsonrpc": "2.0", "method": method, "id": 7 });
            let response = handle_line_on_worker_stack(request.to_string());

            assert_eq!(response["error"]["code"], -32601, "{method}");
            let message = response["error"]["message"].as_str().unwrap();
            assert!(message.contains(method), "{message}");
            assert!(message.contains("-32601"), "{message}");
            assert!(message.contains(PLUGIN_NAME), "{message}");
            assert!(message.contains(reason), "{message}");
        }
    }

    #[test]
    fn unknown_method_error_names_the_method_and_plugin() {
        let request = json!({ "jsonrpc": "2.0", "method": "future_host_rpc", "id": 9 });
        let response = handle_line_on_worker_stack(request.to_string());

        assert_eq!(response["error"]["code"], -32601);
        let message = response["error"]["message"].as_str().unwrap();
        assert!(message.contains("future_host_rpc"));
        assert!(message.contains("-32601"));
        assert!(message.contains(PLUGIN_NAME));
        assert!(message.contains("unknown JSON-RPC method"));
    }

    #[test]
    fn shutdown_closes_cached_pools_and_returns_null() {
        std::thread::Builder::new()
            .stack_size(crate::WORKER_STACK_SIZE)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let pool = pool_manager::get_sqlserver_pool(&ConnectionParams {
                            driver: "sqlserver".into(),
                            host: Some("localhost".into()),
                            port: Some(1433),
                            username: Some("sa".into()),
                            password: Some("test-password".into()),
                            database: crate::models::DatabaseSelection::Single("master".into()),
                            connection_id: Some("shutdown-rpc-test".into()),
                            ..Default::default()
                        })
                        .await
                        .unwrap();
                        assert_eq!(pool_manager::pool_count().await, 1);

                        let response = Box::pin(handle_line(
                            r#"{"jsonrpc":"2.0","method":"shutdown","id":11}"#,
                        ))
                        .await;

                        assert_eq!(
                            response,
                            json!({ "jsonrpc": "2.0", "result": null, "id": 11 })
                        );
                        assert_eq!(pool_manager::pool_count().await, 0);
                        assert!(pool.is_closed());
                    });
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
