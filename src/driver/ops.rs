//! Driver operations: one free function per host RPC method. The JSON-RPC
//! handlers deserialize the request and call these directly.

use std::collections::HashMap;

use mssql_tiberius_bridge::ToSql;

use crate::driver::helpers::{
    bracket_quote, build_delete_composite_sql, build_update_composite_sql, qualify,
};
use crate::driver::{
    acquire, ddl, execute_on_connection, explain, helpers, introspection, routines, triggers,
};
use crate::models::{
    AiSchemaContext, BatchStatementResult, ColumnDefinition, ConnectionParams, ForeignKey, Index,
    PkMap, QueryResult, RoutineCallArg, RoutineInfo, RoutineParameter, TableColumn, TableInfo,
    TableSchema, TriggerInfo, ViewInfo,
};

pub async fn test_connection(params: &ConnectionParams) -> Result<(), String> {
    let mut conn = acquire(params).await?;
    conn.simple_query("SELECT 1")
        .await
        .map_err(|e| e.to_string())?
        .into_first_result();
    Ok(())
}

pub async fn get_databases(params: &ConnectionParams) -> Result<Vec<String>, String> {
    let mut conn = acquire(params).await?;
    // Skip system DBs (database_id <= 4: master, tempdb, model, msdb)
    let rows = conn
        .simple_query("SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name")
        .await
        .map_err(|e| e.to_string())?
        .into_first_result();

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(name) = row.get::<&str, _>(0) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

pub async fn get_schemas(params: &ConnectionParams) -> Result<Vec<String>, String> {
    let mut conn = acquire(params).await?;
    // User schemas: schema_id < 16384 excludes built-in (sys, INFORMATION_SCHEMA, guest, ...).
    // We also exclude the noise schemas explicitly; `dbo` is the default owner and must stay.
    let rows = conn
        .simple_query(
            "SELECT name FROM sys.schemas \
             WHERE schema_id < 16384 \
               AND name NOT IN ('sys','INFORMATION_SCHEMA','guest','db_owner','db_accessadmin','db_securityadmin','db_ddladmin','db_backupoperator','db_datareader','db_datawriter','db_denydatareader','db_denydatawriter') \
             ORDER BY name",
        )
        .await
        .map_err(|e| e.to_string())?
        .into_first_result();

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(name) = row.get::<&str, _>(0) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

// --- Schema inspection ------------------------------------------------------

pub async fn get_tables(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_tables(&mut conn, schema.unwrap_or("dbo")).await
}

pub async fn get_columns(
    params: &ConnectionParams,
    table: &str,
    schema: Option<&str>,
) -> Result<Vec<TableColumn>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_columns(&mut conn, table, schema).await
}

pub async fn get_foreign_keys(
    params: &ConnectionParams,
    table: &str,
    schema: Option<&str>,
) -> Result<Vec<ForeignKey>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_foreign_keys(&mut conn, table, schema).await
}

pub async fn get_indexes(
    params: &ConnectionParams,
    table: &str,
    schema: Option<&str>,
) -> Result<Vec<Index>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_indexes(&mut conn, table, schema).await
}

// --- Views --------------------------------------------------------------

pub async fn get_views(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<Vec<ViewInfo>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_views(&mut conn, schema.unwrap_or("dbo")).await
}

pub async fn get_view_definition(
    params: &ConnectionParams,
    view_name: &str,
    schema: Option<&str>,
) -> Result<String, String> {
    let mut conn = acquire(params).await?;
    introspection::get_module_definition(&mut conn, view_name, schema).await
}

pub async fn get_view_columns(
    params: &ConnectionParams,
    view_name: &str,
    schema: Option<&str>,
) -> Result<Vec<TableColumn>, String> {
    // `sys.columns` + `sys.types` work identically for views, so we reuse
    // the table introspection. The PK sub-query returns 0 for views
    // (no primary key on views), which is the correct behaviour.
    let mut conn = acquire(params).await?;
    introspection::get_columns(&mut conn, view_name, schema).await
}

pub async fn create_view(
    params: &ConnectionParams,
    view_name: &str,
    definition: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = format!(
        "CREATE VIEW {} AS {}",
        qualify(schema, view_name),
        definition
    );
    let mut conn = acquire(params).await?;
    conn.simple_query(sql)
        .await
        .map_err(|error| format!("Failed to create view: {error}"))?
        .into_first_result();
    Ok(())
}

pub async fn alter_view(
    params: &ConnectionParams,
    view_name: &str,
    definition: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = format!(
        "ALTER VIEW {} AS {}",
        qualify(schema, view_name),
        definition
    );
    let mut conn = acquire(params).await?;
    conn.simple_query(sql)
        .await
        .map_err(|error| format!("Failed to alter view: {error}"))?
        .into_first_result();
    Ok(())
}

pub async fn drop_view(
    params: &ConnectionParams,
    view_name: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = format!("DROP VIEW IF EXISTS {}", qualify(schema, view_name));
    let mut conn = acquire(params).await?;
    conn.simple_query(sql)
        .await
        .map_err(|error| format!("Failed to drop view: {error}"))?
        .into_first_result();
    Ok(())
}

// --- Routines -----------------------------------------------------------

pub async fn get_routines(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<Vec<RoutineInfo>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_routines(&mut conn, schema.unwrap_or("dbo")).await
}

pub async fn get_routine_parameters(
    params: &ConnectionParams,
    routine_name: &str,
    schema: Option<&str>,
) -> Result<Vec<RoutineParameter>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_routine_parameters(&mut conn, routine_name, schema.unwrap_or("dbo")).await
}

pub async fn get_routine_definition(
    params: &ConnectionParams,
    routine_name: &str,
    schema: Option<&str>,
) -> Result<String, String> {
    let mut conn = acquire(params).await?;
    introspection::get_module_definition(&mut conn, routine_name, schema).await
}

pub async fn build_routine_call_sql(
    params: &ConnectionParams,
    routine_name: &str,
    routine_type: &str,
    args: &[RoutineCallArg],
    schema: Option<&str>,
) -> Result<String, String> {
    let parameters = if args
        .iter()
        .any(|arg| arg.mode.eq_ignore_ascii_case("OUT") || arg.mode.eq_ignore_ascii_case("INOUT"))
    {
        get_routine_parameters(params, routine_name, schema).await?
    } else {
        Vec::new()
    };
    let is_table_valued = if routine_type.eq_ignore_ascii_case("FUNCTION") {
        let mut conn = acquire(params).await?;
        introspection::is_table_valued_function(&mut conn, routine_name, schema).await?
    } else {
        false
    };
    routines::routine_call_sql(
        routine_name,
        routine_type,
        args,
        &parameters,
        is_table_valued,
        schema,
    )
}

pub async fn get_routine_edit_script(
    params: &ConnectionParams,
    routine_name: &str,
    schema: Option<&str>,
) -> Result<String, String> {
    let definition = get_routine_definition(params, routine_name, schema).await?;
    routines::routine_edit_script(&definition)
}

pub async fn drop_routine(
    params: &ConnectionParams,
    routine_name: &str,
    routine_type: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = routines::drop_routine_sql(routine_name, routine_type, schema);
    execute_query(params, &sql, None, 1).await.map(|_| ())
}

// --- Query execution ---------------------------------------------------

pub async fn execute_query(
    params: &ConnectionParams,
    query: &str,
    limit: Option<u32>,
    page: u32,
) -> Result<QueryResult, String> {
    let mut conn = acquire(params).await?;
    execute_on_connection(&mut conn, query, limit, page).await
}

pub async fn execute_batch(
    params: &ConnectionParams,
    queries: &[String],
    limit: Option<u32>,
    page: u32,
) -> Result<Vec<BatchStatementResult>, String> {
    let mut conn = acquire(params).await?;
    let mut results = Vec::with_capacity(queries.len());
    for query in queries {
        let start = std::time::Instant::now();
        let outcome = execute_on_connection(&mut conn, query, limit, page).await;
        results.push(BatchStatementResult::from_outcome(start, outcome));
    }
    Ok(results)
}

/// Run SHOWPLAN_XML / STATISTICS XML and parse the document into the visual
/// plan model the frontend renders. Unlike the built-in driver — whose raw
/// XML is parsed by `@tabularis/explain` in the frontend — a plugin's
/// `explain_query` result passes through to the frontend untouched, so the
/// parsing happens here.
pub async fn explain_query(
    params: &ConnectionParams,
    query: &str,
    analyze: bool,
) -> Result<serde_json::Value, String> {
    let mut conn = acquire(params).await?;
    let payload = explain::explain_showplan_xml(&mut conn, query, analyze).await?;
    crate::driver::showplan::parse_showplan_xml(&payload, query)
}

// --- CRUD ----------------------------------------------------------------

pub async fn insert_record(
    params: &ConnectionParams,
    table: &str,
    data: HashMap<String, serde_json::Value>,
    schema: Option<&str>,
) -> Result<u64, String> {
    if data.is_empty() {
        return Err("SQL Server: INSERT requires at least one column/value pair".to_string());
    }

    // Acquire the connection up-front; both the identity probe and the
    // actual INSERT reuse it so the IDENTITY_INSERT batch and any error
    // recovery happen on the same session.
    let mut conn = acquire(params).await?;

    let identity_col = introspection::detect_identity_column(&mut conn, table, schema).await?;

    // Deterministic column order keeps the SQL stable for tests and for
    // SQL Server's plan cache (sp_executesql keys on the full text).
    let mut columns: Vec<String> = data.keys().cloned().collect();
    columns.sort();

    let needs_identity_insert = identity_col
        .as_ref()
        .map(|id| columns.iter().any(|c| c.eq_ignore_ascii_case(id)))
        .unwrap_or(false);

    let qualified = helpers::qualify(schema, table);
    let sql = helpers::build_insert_sql(
        &qualified,
        &columns,
        if needs_identity_insert {
            Some(qualified.as_str())
        } else {
            None
        },
    );

    // Map each JSON value to a typed SQL parameter. Owned boxes live
    // for the duration of the call so the borrowed `&dyn ToSql` slice is
    // valid.
    let owned_params: Vec<Box<dyn mssql_tiberius_bridge::ToSql>> = columns
        .iter()
        .map(|column| helpers::value_to_sql_param(&data[column]))
        .collect::<Result<_, _>>()?;
    let params_slice: Vec<&dyn mssql_tiberius_bridge::ToSql> =
        owned_params.iter().map(|b| b.as_ref()).collect();

    let result = conn
        .query(&sql, &params_slice)
        .await
        .map_err(|e| e.to_string())?;
    crate::driver::affected_rows_from_query(result)
}

pub async fn update_record(
    params: &ConnectionParams,
    table: &str,
    pk_map: &PkMap,
    col_name: &str,
    new_val: serde_json::Value,
    schema: Option<&str>,
) -> Result<u64, String> {
    let mut primary_keys: Vec<_> = pk_map.iter().collect();
    primary_keys.sort_by_key(|&(column, _)| column);
    let pk_columns: Vec<String> = primary_keys
        .iter()
        .map(|(column, _)| (*column).clone())
        .collect();
    let sql = build_update_composite_sql(schema, table, col_name, &pk_columns)
        .ok_or_else(|| "SQL Server: UPDATE requires at least one primary-key column".to_string())?;

    let mut owned_params = Vec::with_capacity(primary_keys.len() + 1);
    owned_params.push(helpers::value_to_sql_param(&new_val)?);
    for (_, value) in primary_keys {
        owned_params.push(helpers::value_to_sql_param(value)?);
    }
    let bound: Vec<&dyn ToSql> = owned_params.iter().map(|value| value.as_ref()).collect();

    let mut conn = acquire(params).await?;
    let result = conn
        .query(helpers::wrap_dml_with_rowcount(&sql), &bound)
        .await
        .map_err(|error| error.to_string())?;
    crate::driver::affected_rows_from_query(result)
}

pub async fn delete_record(
    params: &ConnectionParams,
    table: &str,
    pk_map: &PkMap,
    schema: Option<&str>,
) -> Result<u64, String> {
    let mut primary_keys: Vec<_> = pk_map.iter().collect();
    primary_keys.sort_by_key(|&(column, _)| column);
    let pk_columns: Vec<String> = primary_keys
        .iter()
        .map(|(column, _)| (*column).clone())
        .collect();
    let sql = build_delete_composite_sql(schema, table, &pk_columns)
        .ok_or_else(|| "SQL Server: DELETE requires at least one primary-key column".to_string())?;

    let owned_params: Vec<Box<dyn ToSql>> = primary_keys
        .into_iter()
        .map(|(_, value)| helpers::value_to_sql_param(value))
        .collect::<Result<_, _>>()?;
    let bound: Vec<&dyn ToSql> = owned_params.iter().map(|value| value.as_ref()).collect();

    let mut conn = acquire(params).await?;
    let result = conn
        .query(helpers::wrap_dml_with_rowcount(&sql), &bound)
        .await
        .map_err(|error| error.to_string())?;
    crate::driver::affected_rows_from_query(result)
}

// --- DDL generation -----------------------------------------------------

pub fn get_create_table_sql(
    table_name: &str,
    columns: Vec<ColumnDefinition>,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut col_defs = Vec::new();
    let mut pk_cols = Vec::new();

    for column in &columns {
        col_defs.push(helpers::render_column_definition(column, false));
        if column.is_pk {
            pk_cols.push(bracket_quote(&column.name));
        }
    }

    if !pk_cols.is_empty() {
        col_defs.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    let table_ref = qualify(schema, table_name);
    Ok(vec![format!(
        "CREATE TABLE {} (\n  {}\n)",
        table_ref,
        col_defs.join(",\n  ")
    )])
}

pub fn get_add_column_sql(
    table: &str,
    column: ColumnDefinition,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    Ok(vec![format!(
        "ALTER TABLE {} ADD {}",
        qualify(schema, table),
        helpers::render_column_definition(&column, true),
    )])
}

pub fn get_alter_column_sql(
    table: &str,
    old_column: ColumnDefinition,
    new_column: ColumnDefinition,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    ddl::alter_column_sql(table, &old_column, &new_column, schema)
}

pub fn get_create_index_sql(
    table: &str,
    index_name: &str,
    columns: Vec<String>,
    is_unique: bool,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    if columns.is_empty() {
        return Err("SQL Server: CREATE INDEX requires at least one column".into());
    }
    let columns = columns
        .iter()
        .map(|column| bracket_quote(column))
        .collect::<Vec<_>>()
        .join(", ");
    let unique = if is_unique { "UNIQUE " } else { "" };
    Ok(vec![format!(
        "CREATE {unique}INDEX {} ON {} ({columns})",
        bracket_quote(index_name),
        qualify(schema, table),
    )])
}

#[allow(clippy::too_many_arguments)]
pub fn get_create_foreign_key_sql(
    table: &str,
    fk_name: &str,
    column: &str,
    ref_table: &str,
    ref_column: &str,
    on_delete: Option<&str>,
    on_update: Option<&str>,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    ddl::create_foreign_key_sql(
        table, fk_name, column, ref_table, ref_column, on_delete, on_update, schema,
    )
}

pub async fn drop_index(
    params: &ConnectionParams,
    table: &str,
    index_name: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = format!(
        "DROP INDEX {} ON {}",
        bracket_quote(index_name),
        qualify(schema, table),
    );
    let mut conn = acquire(params).await?;
    conn.execute(sql, &[])
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn drop_foreign_key(
    params: &ConnectionParams,
    table: &str,
    fk_name: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = format!(
        "ALTER TABLE {} DROP CONSTRAINT {}",
        qualify(schema, table),
        bracket_quote(fk_name),
    );
    let mut conn = acquire(params).await?;
    conn.execute(sql, &[])
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// --- Triggers -----------------------------------------------------------

pub async fn get_triggers(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<Vec<TriggerInfo>, String> {
    let mut conn = acquire(params).await?;
    triggers::get_triggers(&mut conn, schema).await
}

pub async fn get_trigger_definition(
    params: &ConnectionParams,
    trigger_name: &str,
    schema: Option<&str>,
) -> Result<String, String> {
    let mut conn = acquire(params).await?;
    introspection::get_module_definition(&mut conn, trigger_name, schema).await
}

pub async fn create_trigger(params: &ConnectionParams, trigger_sql: &str) -> Result<(), String> {
    execute_query(params, trigger_sql, None, 1)
        .await
        .map(|_| ())
}

pub async fn drop_trigger(
    params: &ConnectionParams,
    trigger_name: &str,
    schema: Option<&str>,
) -> Result<(), String> {
    let sql = triggers::drop_trigger_sql(trigger_name, schema);
    execute_query(params, &sql, None, 1).await.map(|_| ())
}

// --- ER diagram batch -----------------------------------------------------

pub async fn get_schema_snapshot(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<Vec<TableSchema>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_schema_snapshot(&mut conn, schema.unwrap_or("dbo")).await
}

pub async fn get_all_columns_batch(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<HashMap<String, Vec<TableColumn>>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_all_columns_batch(&mut conn, schema.unwrap_or("dbo")).await
}

pub async fn get_all_foreign_keys_batch(
    params: &ConnectionParams,
    schema: Option<&str>,
) -> Result<HashMap<String, Vec<ForeignKey>>, String> {
    let mut conn = acquire(params).await?;
    introspection::get_all_foreign_keys_batch(&mut conn, schema.unwrap_or("dbo")).await
}

/// Batch metadata for AI Query Assist: the full snapshot query, truncated to
/// `max_tables` while reporting the pre-truncation table count.
pub async fn get_ai_schema_context(
    params: &ConnectionParams,
    schema: Option<&str>,
    max_tables: usize,
) -> Result<AiSchemaContext, String> {
    let mut tables = get_schema_snapshot(params, schema).await?;
    let total_table_count = tables.len();
    tables.truncate(max_tables);
    Ok(AiSchemaContext {
        tables,
        total_table_count,
    })
}
