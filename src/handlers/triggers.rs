//! Trigger listing and management.

use serde_json::Value;

use crate::driver::ops;
use crate::rpc::{conn_params, opt_str, req_str, respond};

pub async fn get_triggers(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_triggers(&conn, opt_str(params, "schema")).await,
    )
}

pub async fn get_trigger_definition(id: Value, params: &Value) -> Value {
    let (conn, trigger_name) = match (conn_params(params), req_str(params, "trigger_name")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_trigger_definition(&conn, trigger_name, opt_str(params, "schema")).await,
    )
}

pub async fn create_trigger(id: Value, params: &Value) -> Value {
    let (conn, trigger_sql) = match (conn_params(params), req_str(params, "trigger_sql")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::create_trigger(&conn, trigger_sql)
            .await
            .map(|()| Value::Null),
    )
}

pub async fn drop_trigger(id: Value, params: &Value) -> Value {
    let (conn, trigger_name) = match (conn_params(params), req_str(params, "trigger_name")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::drop_trigger(&conn, trigger_name, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}
