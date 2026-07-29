//! Microsoft SQL Server driver core.
//!
//! Editing is enabled for single/composite primary keys and IDENTITY tables.
//! The driver supports schema introspection, table/view DDL, foreign keys,
//! triggers, and stored-routine management.

pub mod ddl;
pub mod explain;
pub mod extract;
pub mod helpers;
pub mod introspection;
pub mod ops;
pub mod pool;
pub mod routines;
pub mod showplan;
pub mod triggers;
pub mod types;
pub mod version;

use mssql_tds::connection::tds_client::{ResultSet, ResultSetClient};
use mssql_tiberius_bridge::row::RowSchema;
use mssql_tiberius_bridge::Row;

use crate::models::{ConnectionParams, Pagination, QueryResult};
use crate::pool_manager::get_sqlserver_pool;

/// Acquire a pooled client from the pool manager.
pub async fn acquire(
    params: &ConnectionParams,
) -> Result<deadpool::managed::Object<pool::BridgeManager>, String> {
    let pool = get_sqlserver_pool(params).await?;
    pool.get().await.map_err(|e| e.to_string())
}

fn empty_query_result(columns: Vec<String>) -> QueryResult {
    QueryResult {
        columns,
        rows: Vec::new(),
        affected_rows: 0,
        truncated: false,
        pagination: None,
        additional_results: None,
    }
}

/// Run `query` as a simple batch and collect every result set.
///
/// Goes through the bridge's `inner_mut()` escape hatch instead of
/// `simple_query().into_results()`: the bridge derives columns from rows, so
/// a result set with zero rows would lose its column headers. Reading the
/// result-set metadata directly preserves them, matching the behaviour the
/// UI expects for empty SELECTs.
async fn run_query_collecting(
    conn: &mut pool::BridgeConnection,
    query: &str,
) -> Result<Vec<QueryResult>, String> {
    let client = conn.inner_mut();
    // Drain any leftover state from a prior query / dropped stream so we
    // don't hit "open batch" errors when re-using the client.
    client
        .close_query()
        .await
        .map_err(|error| error.to_string())?;
    client
        .execute(query.to_string(), None, None)
        .await
        .map_err(|error| error.to_string())?;

    let mut results = Vec::new();
    while let Some(result_set) = client.get_current_resultset() {
        let metadata = result_set.get_metadata().clone();
        let schema = RowSchema::from_metadata(&metadata);
        let mut current = empty_query_result(
            metadata
                .iter()
                .map(|column| column.column_name.clone())
                .collect(),
        );
        while let Some(values) = result_set
            .next_row()
            .await
            .map_err(|error| error.to_string())?
        {
            let row = Row::from_schema(schema.clone(), values);
            current.rows.push(
                (0..row.columns().len())
                    .map(|index| extract::extract_value(&row, index))
                    .collect(),
            );
        }
        results.push(current);
        if !client
            .move_to_next()
            .await
            .map_err(|error| error.to_string())?
        {
            break;
        }
    }
    Ok(results)
}

/// Pull the trailing [`helpers::AFFECTED_ROWS_COLUMN`] result set produced by
/// a parameterized DML batch and return its count.
pub fn affected_rows_from_query(result: mssql_tiberius_bridge::QueryResult) -> Result<u64, String> {
    result
        .into_results()
        .last()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get::<i64, _>(0))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "SQL Server did not return affected rows for DML".to_string())
}

async fn execute_result_bearing_dml(
    conn: &mut pool::BridgeConnection,
    query: &str,
) -> Result<QueryResult, String> {
    let wrapped = helpers::wrap_dml_with_rowcount(query);
    let mut results = run_query_collecting(conn, &wrapped).await?;
    let affected = results
        .last()
        .filter(|result| result.columns == [helpers::AFFECTED_ROWS_COLUMN])
        .and_then(|result| result.rows.first())
        .and_then(|row| row.first())
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            "SQL Server did not return affected rows for result-bearing DML".to_string()
        })?;
    results.pop();

    let mut first = if results.is_empty() {
        empty_query_result(Vec::new())
    } else {
        results.remove(0)
    };
    first.affected_rows = affected;
    if !results.is_empty() {
        first.additional_results = Some(results);
    }
    Ok(first)
}

pub async fn execute_on_connection(
    conn: &mut pool::BridgeConnection,
    query: &str,
    limit: Option<u32>,
    page: u32,
) -> Result<QueryResult, String> {
    let returns_result_set = helpers::query_returns_result_set(query);
    if helpers::query_reports_affected_rows(query) {
        // The TDS client reports rows returned, not rows affected, so every
        // DML goes through the @@ROWCOUNT-capturing batch — result-bearing
        // (OUTPUT clauses) or not.
        let mut result = execute_result_bearing_dml(conn, query).await?;
        if !returns_result_set {
            result.columns = Vec::new();
            result.rows = Vec::new();
        }
        return Ok(result);
    }
    if !returns_result_set {
        conn.simple_query(query)
            .await
            .map_err(|error| error.to_string())?
            .into_results();
        return Ok(empty_query_result(Vec::new()));
    }

    let pagination_limit = limit.filter(|_| helpers::query_can_be_paginated(query));
    let mut pagination = pagination_limit.map(|page_size| Pagination {
        page,
        page_size,
        total_rows: None,
        has_more: false,
    });
    let final_query = match pagination_limit {
        Some(page_size) => helpers::build_paginated_query(query, page_size, page),
        None => query.to_string(),
    };
    let mut results = run_query_collecting(conn, &final_query).await?;
    let mut first = if results.is_empty() {
        empty_query_result(Vec::new())
    } else {
        results.remove(0)
    };

    if let Some(ref mut pagination) = pagination {
        pagination.has_more = first.rows.len() > pagination.page_size as usize;
        if pagination.has_more {
            first.rows.truncate(pagination.page_size as usize);
            first.truncated = true;
        }
    }
    first.pagination = pagination;
    if !results.is_empty() {
        first.additional_results = Some(results);
    }
    Ok(first)
}
