//! Row-level INSERT / UPDATE / DELETE.

use serde_json::Value;

use crate::driver::ops;
use crate::models::PkMap;
use crate::rpc::{conn_params, opt_str, req_field, req_str, respond};

pub async fn insert_record(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let data: PkMap = match req_field(params, "data") {
        Ok(d) => d,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::insert_record(&conn, table, data, opt_str(params, "schema")).await,
    )
}

pub async fn update_record(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let (pk_map, col_name): (PkMap, &str) =
        match (req_field(params, "pk_map"), req_str(params, "col_name")) {
            (Ok(m), Ok(c)) => (m, c),
            (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
        };
    let new_val = params.get("new_val").cloned().unwrap_or(Value::Null);
    respond(
        id,
        ops::update_record(
            &conn,
            table,
            &pk_map,
            col_name,
            new_val,
            opt_str(params, "schema"),
        )
        .await,
    )
}

pub async fn delete_record(id: Value, params: &Value) -> Value {
    let (conn, table) = match (conn_params(params), req_str(params, "table")) {
        (Ok(c), Ok(t)) => (c, t),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let pk_map: PkMap = match req_field(params, "pk_map") {
        Ok(m) => m,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::delete_record(&conn, table, &pk_map, opt_str(params, "schema")).await,
    )
}
