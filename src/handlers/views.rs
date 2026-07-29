//! View listing, definition, and management.

use serde_json::Value;

use crate::driver::ops;
use crate::rpc::{conn_params, opt_str, req_str, respond};

pub async fn get_views(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(id, ops::get_views(&conn, opt_str(params, "schema")).await)
}

pub async fn get_view_definition(id: Value, params: &Value) -> Value {
    let (conn, view_name) = match (conn_params(params), req_str(params, "view_name")) {
        (Ok(c), Ok(v)) => (c, v),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_view_definition(&conn, view_name, opt_str(params, "schema")).await,
    )
}

pub async fn get_view_columns(id: Value, params: &Value) -> Value {
    let (conn, view_name) = match (conn_params(params), req_str(params, "view_name")) {
        (Ok(c), Ok(v)) => (c, v),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_view_columns(&conn, view_name, opt_str(params, "schema")).await,
    )
}

pub async fn create_view(id: Value, params: &Value) -> Value {
    let (conn, view_name, definition) = match (
        conn_params(params),
        req_str(params, "view_name"),
        req_str(params, "definition"),
    ) {
        (Ok(c), Ok(v), Ok(d)) => (c, v, d),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::create_view(&conn, view_name, definition, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}

pub async fn alter_view(id: Value, params: &Value) -> Value {
    let (conn, view_name, definition) = match (
        conn_params(params),
        req_str(params, "view_name"),
        req_str(params, "definition"),
    ) {
        (Ok(c), Ok(v), Ok(d)) => (c, v, d),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::alter_view(&conn, view_name, definition, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}

pub async fn drop_view(id: Value, params: &Value) -> Value {
    let (conn, view_name) = match (conn_params(params), req_str(params, "view_name")) {
        (Ok(c), Ok(v)) => (c, v),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::drop_view(&conn, view_name, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}
