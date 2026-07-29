//! DDL generation and index / foreign-key drops.

use serde_json::Value;

use crate::driver::ops;
use crate::models::ColumnDefinition;
use crate::rpc::{conn_params, opt_str, req_field, req_str, respond};

pub async fn get_create_table_sql(id: Value, params: &Value) -> Value {
    let table_name = match req_str(params, "table_name") {
        Ok(t) => t,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let columns: Vec<ColumnDefinition> = match req_field(params, "columns") {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_create_table_sql(table_name, columns, opt_str(params, "schema")),
    )
}

pub async fn get_add_column_sql(id: Value, params: &Value) -> Value {
    let table = match req_str(params, "table") {
        Ok(t) => t,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let column: ColumnDefinition = match req_field(params, "column") {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_add_column_sql(table, column, opt_str(params, "schema")),
    )
}

pub async fn get_alter_column_sql(id: Value, params: &Value) -> Value {
    let table = match req_str(params, "table") {
        Ok(t) => t,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let (old_column, new_column): (ColumnDefinition, ColumnDefinition) = match (
        req_field(params, "old_column"),
        req_field(params, "new_column"),
    ) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_alter_column_sql(table, old_column, new_column, opt_str(params, "schema")),
    )
}

pub async fn get_create_index_sql(id: Value, params: &Value) -> Value {
    let (table, index_name) = match (req_str(params, "table"), req_str(params, "index_name")) {
        (Ok(t), Ok(i)) => (t, i),
        (Err(e), _) | (_, Err(e)) => return respond::<()>(id, Err(e)),
    };
    let columns: Vec<String> = match req_field(params, "columns") {
        Ok(c) => c,
        Err(e) => return respond::<()>(id, Err(e)),
    };
    let is_unique = params
        .get("is_unique")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    respond(
        id,
        ops::get_create_index_sql(
            table,
            index_name,
            columns,
            is_unique,
            opt_str(params, "schema"),
        ),
    )
}

pub async fn get_create_foreign_key_sql(id: Value, params: &Value) -> Value {
    let required = (
        req_str(params, "table"),
        req_str(params, "fk_name"),
        req_str(params, "column"),
        req_str(params, "ref_table"),
        req_str(params, "ref_column"),
    );
    let (table, fk_name, column, ref_table, ref_column) = match required {
        (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) => (a, b, c, d, e),
        (Err(e), ..)
        | (_, Err(e), ..)
        | (_, _, Err(e), ..)
        | (_, _, _, Err(e), _)
        | (_, _, _, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::get_create_foreign_key_sql(
            table,
            fk_name,
            column,
            ref_table,
            ref_column,
            opt_str(params, "on_delete"),
            opt_str(params, "on_update"),
            opt_str(params, "schema"),
        ),
    )
}

pub async fn drop_index(id: Value, params: &Value) -> Value {
    let (conn, table, index_name) = match (
        conn_params(params),
        req_str(params, "table"),
        req_str(params, "index_name"),
    ) {
        (Ok(c), Ok(t), Ok(i)) => (c, t, i),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::drop_index(&conn, table, index_name, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}

pub async fn drop_foreign_key(id: Value, params: &Value) -> Value {
    let (conn, table, fk_name) = match (
        conn_params(params),
        req_str(params, "table"),
        req_str(params, "fk_name"),
    ) {
        (Ok(c), Ok(t), Ok(f)) => (c, t, f),
        (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => return respond::<()>(id, Err(e)),
    };
    respond(
        id,
        ops::drop_foreign_key(&conn, table, fk_name, opt_str(params, "schema"))
            .await
            .map(|()| Value::Null),
    )
}
