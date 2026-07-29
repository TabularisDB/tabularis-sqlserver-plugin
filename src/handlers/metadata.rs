//! Schema metadata: databases, schemas, tables, columns, indexes, FKs, and
//! the ER-diagram / AI batch variants.

use serde_json::Value;

use crate::driver::ops;
use crate::rpc::{conn_params, opt_str, req_str, respond};

pub async fn get_databases(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(id, ops::get_databases(&conn).await)
}

pub async fn get_schemas(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(id, ops::get_schemas(&conn).await)
}

pub async fn get_tables(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(id, ops::get_tables(&conn, opt_str(params, "schema")).await)
}

pub async fn get_columns(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_columns(&conn, table, opt_str(params, "schema")).await,
    )
}

pub async fn get_foreign_keys(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_foreign_keys(&conn, table, opt_str(params, "schema")).await,
    )
}

pub async fn get_indexes(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_indexes(&conn, table, opt_str(params, "schema")).await,
    )
}

pub async fn get_schema_snapshot(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_schema_snapshot(&conn, opt_str(params, "schema")).await,
    )
}

pub async fn get_all_columns_batch(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_all_columns_batch(&conn, opt_str(params, "schema")).await,
    )
}

pub async fn get_all_foreign_keys_batch(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_all_foreign_keys_batch(&conn, opt_str(params, "schema")).await,
    )
}

pub async fn get_ai_schema_context(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let max_tables = params
        .get("max_tables")
        .and_then(Value::as_u64)
        .unwrap_or(20) as usize;
    respond(
        id,
        ops::get_ai_schema_context(&conn, opt_str(params, "schema"), max_tables).await,
    )
}
