//! Connection checks and query execution.

use serde_json::{json, Value};

use crate::driver::ops;
use crate::rpc::{conn_params, req_str, respond};

pub async fn test_connection(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::test_connection(&conn)
            .await
            .map(|()| json!({ "success": true })),
    )
}

/// Lightweight health check: reuses a pooled session, so it is cheaper than
/// `test_connection` for the host's periodic liveness probing.
pub async fn ping(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(id, ops::test_connection(&conn).await.map(|()| Value::Null))
}

fn limit_and_page(params: &Value) -> (Option<u32>, u32) {
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    let page = params
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(1);
    (limit, page)
}

pub async fn execute_query(id: Value, params: &Value) -> Value {
    let (conn, query) = match (conn_params(params), req_str(params, "query")) {
        (Ok(c), Ok(q)) => (c, q),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let (limit, page) = limit_and_page(params);
    respond(id, ops::execute_query(&conn, query, limit, page).await)
}

pub async fn execute_query_batch(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let queries: Vec<String> = match crate::rpc::req_field(params, "queries") {
        Ok(q) => q,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let (limit, page) = limit_and_page(params);
    respond(id, ops::execute_batch(&conn, &queries, limit, page).await)
}

pub async fn explain_query(id: Value, params: &Value) -> Value {
    let (conn, query) = match (conn_params(params), req_str(params, "query")) {
        (Ok(c), Ok(q)) => (c, q),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let analyze = params
        .get("analyze")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    respond(id, ops::explain_query(&conn, query, analyze).await)
}
