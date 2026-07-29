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

use futures::TryStreamExt;

use crate::models::{ConnectionParams, Pagination, QueryResult};
use crate::pool_manager::get_sqlserver_pool;

/// Acquire a Tiberius client from the pool.
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

async fn collect_query_results(
    mut stream: tiberius::QueryStream<'_>,
) -> Result<Vec<QueryResult>, String> {
    let mut results = Vec::new();
    let mut current: Option<QueryResult> = None;

    while let Some(item) = stream.try_next().await.map_err(|error| error.to_string())? {
        match item {
            tiberius::QueryItem::Metadata(metadata) => {
                if let Some(previous) = current.take() {
                    results.push(previous);
                }
                current = Some(empty_query_result(
                    metadata
                        .columns()
                        .iter()
                        .map(|column| column.name().to_string())
                        .collect(),
                ));
            }
            tiberius::QueryItem::Row(row) => {
                let result = current.get_or_insert_with(|| {
                    empty_query_result(
                        row.columns()
                            .iter()
                            .map(|column| column.name().to_string())
                            .collect(),
                    )
                });
                result.rows.push(
                    (0..row.columns().len())
                        .map(|index| extract::extract_value(&row, index))
                        .collect(),
                );
            }
        }
    }
    if let Some(result) = current {
        results.push(result);
    }
    Ok(results)
}

async fn execute_result_bearing_dml(
    conn: &mut pool::BridgeConnection,
    query: &str,
) -> Result<QueryResult, String> {
    const AFFECTED_COLUMN: &str = "__tabularis_affected_rows";
    let wrapped = format!("{query}\n; SELECT CAST(@@ROWCOUNT AS BIGINT) AS [{AFFECTED_COLUMN}]");
    let stream = conn
        .simple_query(wrapped)
        .await
        .map_err(|error| error.to_string())?;
    let mut results = collect_query_results(stream).await?;
    let affected = results
        .last()
        .filter(|result| result.columns == [AFFECTED_COLUMN])
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
    if returns_result_set && helpers::query_reports_affected_rows(query) {
        return execute_result_bearing_dml(conn, query).await;
    }
    if !returns_result_set {
        if helpers::query_reports_affected_rows(query) {
            let affected_rows = conn
                .execute(query, &[])
                .await
                .map_err(|error| error.to_string())?
                .total();
            return Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                affected_rows,
                truncated: false,
                pagination: None,
                additional_results: None,
            });
        }

        conn.simple_query(query)
            .await
            .map_err(|error| error.to_string())?
            .into_results()
            .await
            .map_err(|error| error.to_string())?;
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
    let stream = conn
        .simple_query(final_query)
        .await
        .map_err(|error| error.to_string())?;
    let mut results = collect_query_results(stream).await?;
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
