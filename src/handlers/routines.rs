//! Stored procedure / function listing, invocation SQL, and management.

use serde_json::Value;

use crate::driver::ops;
use crate::driver::routines as routine_sql;
use crate::models::RoutineCallArg;
use crate::rpc::{conn_params, opt_str, req_field, req_str, respond};

pub async fn get_routines(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_routines(&conn, opt_str(params, "schema")).await,
    )
}

pub async fn get_routine_parameters(id: Value, params: &Value) -> Value {
    let (conn, routine_name) = match (conn_params(params), req_str(params, "routine_name")) {
        (Ok(c), Ok(r)) => (c, r),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_routine_parameters(&conn, routine_name, opt_str(params, "schema")).await,
    )
}

pub async fn get_routine_definition(id: Value, params: &Value) -> Value {
    let (conn, routine_name) = match (conn_params(params), req_str(params, "routine_name")) {
        (Ok(c), Ok(r)) => (c, r),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_routine_definition(&conn, routine_name, opt_str(params, "schema")).await,
    )
}

pub async fn build_routine_call_sql(id: Value, params: &Value) -> Value {
    let (conn, routine_name, routine_type) = match (
        conn_params(params),
        req_str(params, "routine_name"),
        req_str(params, "routine_type"),
    ) {
        (Ok(c), Ok(r), Ok(t)) => (c, r, t),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let args: Vec<RoutineCallArg> = match req_field(params, "args") {
        Ok(a) => a,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::build_routine_call_sql(
            &conn,
            routine_name,
            routine_type,
            &args,
            opt_str(params, "schema"),
        )
        .await,
    )
}

/// Purely syntactic — no connection involved, so the host does not send one.
pub async fn routine_create_template(id: Value, params: &Value) -> Value {
    let routine_type = match req_str(params, "routine_type") {
        Ok(t) => t,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond::<String>(
        id,
        Ok(routine_sql::routine_create_template(
            routine_type,
            opt_str(params, "schema"),
        )),
    )
}

pub async fn get_routine_edit_script(id: Value, params: &Value) -> Value {
    let (conn, routine_name) = match (conn_params(params), req_str(params, "routine_name")) {
        (Ok(c), Ok(r)) => (c, r),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_routine_edit_script(&conn, routine_name, opt_str(params, "schema")).await,
    )
}

pub async fn drop_routine(id: Value, params: &Value) -> Value {
    let (conn, routine_name, routine_type) = match (
        conn_params(params),
        req_str(params, "routine_name"),
        req_str(params, "routine_type"),
    ) {
        (Ok(c), Ok(r), Ok(t)) => (c, r, t),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::drop_routine(&conn, routine_name, routine_type, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}
