//! JSON-RPC adapters for SQL Server database users and privileges.

use serde_json::Value;

use crate::driver::ops;
use crate::rpc::{conn_params, opt_str, req_field, req_str, respond};

pub async fn get_db_privilege_catalog(id: Value) -> Value {
    respond(id, Ok(ops::get_db_privilege_catalog()))
}

pub async fn get_db_users(id: Value, params: &Value) -> Value {
    let conn = match conn_params(params) {
        Ok(conn) => conn,
        Err(error) => return respond::<()>(id, Err(error)),
    };
    respond(id, ops::get_db_users(&conn).await)
}

pub async fn create_db_user(id: Value, params: &Value) -> Value {
    let (conn, user, login, password) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
        req_str(params, "password"),
    ) {
        (Ok(conn), Ok(user), Ok(login), Ok(password)) => (conn, user, login, password),
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => return respond::<()>(id, Err(error)),
    };
    respond(id, ops::create_db_user(&conn, user, login, password).await)
}

pub async fn drop_db_user(id: Value, params: &Value) -> Value {
    let (conn, user, login) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
    ) {
        (Ok(conn), Ok(user), Ok(login)) => (conn, user, login),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return respond::<()>(id, Err(error))
        }
    };
    respond(id, ops::drop_db_user(&conn, user, login).await)
}

pub async fn set_db_user_password(id: Value, params: &Value) -> Value {
    let (conn, user, login, password) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
        req_str(params, "password"),
    ) {
        (Ok(conn), Ok(user), Ok(login), Ok(password)) => (conn, user, login, password),
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => return respond::<()>(id, Err(error)),
    };
    respond(
        id,
        ops::set_db_user_password(&conn, user, login, password).await,
    )
}

pub async fn get_db_user_grants(id: Value, params: &Value) -> Value {
    let (conn, user, login) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
    ) {
        (Ok(conn), Ok(user), Ok(login)) => (conn, user, login),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return respond::<()>(id, Err(error))
        }
    };
    respond(id, ops::get_db_user_grants(&conn, user, login).await)
}

pub async fn get_db_user_privileges(id: Value, params: &Value) -> Value {
    let (conn, user, login) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
    ) {
        (Ok(conn), Ok(user), Ok(login)) => (conn, user, login),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return respond::<()>(id, Err(error))
        }
    };
    respond(id, ops::get_db_user_privileges(&conn, user, login).await)
}

pub async fn apply_db_user_privileges(id: Value, params: &Value) -> Value {
    let (conn, user, login) = match (
        conn_params(params),
        req_str(params, "user"),
        req_str(params, "host"),
    ) {
        (Ok(conn), Ok(user), Ok(login)) => (conn, user, login),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return respond::<()>(id, Err(error))
        }
    };
    let privileges: Vec<String> = match req_field(params, "privileges") {
        Ok(privileges) => privileges,
        Err(error) => return respond::<()>(id, Err(error)),
    };
    let grant: bool = match req_field(params, "grant") {
        Ok(grant) => grant,
        Err(error) => return respond::<()>(id, Err(error)),
    };
    respond(
        id,
        ops::apply_db_user_privileges(
            &conn,
            user,
            login,
            opt_str(params, "database"),
            opt_str(params, "table"),
            &privileges,
            grant,
        )
        .await,
    )
}
