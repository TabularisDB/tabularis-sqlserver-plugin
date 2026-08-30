//! JSON-RPC adapters for SQL Server binary-column export and preview.

use serde_json::Value;

use crate::driver::{blob as driver_blob, ops};
use crate::models::PkMap;
use crate::rpc::{conn_params, opt_str, req_field, req_str, respond};

pub async fn save_blob_to_file(id: Value, params: &Value) -> Value {
    let (conn, table, col_name, file_path) = match (
        conn_params(params),
        req_str(params, "table"),
        req_str(params, "col_name"),
        req_str(params, "file_path"),
    ) {
        (Ok(conn), Ok(table), Ok(col_name), Ok(file_path)) => (conn, table, col_name, file_path),
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => return respond::<()>(id, Err(error)),
    };
    let pk_map: PkMap = match req_field(params, "pk_map") {
        Ok(pk_map) => pk_map,
        Err(error) => return respond::<()>(id, Err(error)),
    };

    respond(
        id,
        ops::save_blob_to_file(
            &conn,
            table,
            col_name,
            &pk_map,
            opt_str(params, "schema"),
            file_path,
        )
        .await,
    )
}

pub async fn fetch_blob_as_data_url(id: Value, params: &Value) -> Value {
    let (conn, table, col_name) = match (
        conn_params(params),
        req_str(params, "table"),
        req_str(params, "col_name"),
    ) {
        (Ok(conn), Ok(table), Ok(col_name)) => (conn, table, col_name),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return respond::<()>(id, Err(error))
        }
    };
    let pk_map: PkMap = match req_field(params, "pk_map") {
        Ok(pk_map) => pk_map,
        Err(error) => return respond::<()>(id, Err(error)),
    };
    let max_blob_size = match params.get("max_blob_size") {
        Some(_) => match req_field(params, "max_blob_size") {
            Ok(max_blob_size) => max_blob_size,
            Err(error) => return respond::<()>(id, Err(error)),
        },
        None => driver_blob::DEFAULT_MAX_BLOB_SIZE,
    };

    respond(
        id,
        ops::fetch_blob_as_data_url(
            &conn,
            table,
            col_name,
            &pk_map,
            opt_str(params, "schema"),
            max_blob_size,
        )
        .await,
    )
}
