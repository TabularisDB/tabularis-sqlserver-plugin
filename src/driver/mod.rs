//! Microsoft SQL Server driver core.
//!
//! Editing is enabled for single/composite primary keys and IDENTITY tables.
//! The driver supports schema introspection, table/view DDL, foreign keys,
//! triggers, and stored-routine management.

pub mod blob;
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
/// UI expects for empty SELECTs. This cannot be simulated faithfully without
/// a TDS stream; SS-003 exercises zero-row headers here through simple,
/// multi-result, batch-RPC, and paginated JSON-RPC calls.
async fn run_query_collecting(
    conn: &mut pool::BridgeConnection,
    query: &str,
) -> Result<Vec<QueryResult>, String> {
    let query_timeout_seconds = conn.query_timeout_seconds();
    let client = conn.inner_mut();
    // Drain any leftover state from a prior query / dropped stream so we
    // don't hit "open batch" errors when re-using the client.
    client
        .close_query()
        .await
        .map_err(|error| error.to_string())?;
    client
        .execute(query.to_string(), query_timeout_seconds, None)
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
                metadata
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let column_type = extract::normalized_column_type(
                            column.data_type,
                            column.type_info.length,
                        );
                        extract::extract_value_as(&row, index, column_type)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
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

fn affected_rows_value(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn finish_result_bearing_dml(mut results: Vec<QueryResult>) -> Result<QueryResult, String> {
    let affected = results
        .last()
        .filter(|result| result.columns == [helpers::AFFECTED_ROWS_COLUMN])
        .and_then(|result| result.rows.first())
        .and_then(|row| row.first())
        .and_then(affected_rows_value)
        .ok_or_else(|| {
            "SQL Server did not return affected rows for result-bearing DML".to_string()
        })?;
    // The sentinel is an implementation detail. Removing it here guarantees
    // OUTPUT and mixed batches never expose a stray one-cell grid to the host.
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

async fn execute_result_bearing_dml(
    conn: &mut pool::BridgeConnection,
    query: &str,
) -> Result<QueryResult, String> {
    let wrapped = helpers::wrap_dml_with_rowcount(query);
    finish_result_bearing_dml(run_query_collecting(conn, &wrapped).await?)
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
        // final DML statement goes through the @@ROWCOUNT-capturing batch —
        // result-bearing (OUTPUT clauses) or not. SS-003 verifies plain,
        // multi-statement, and OUTPUT cases against SQL Server.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(columns: &[&str], rows: Vec<Vec<serde_json::Value>>) -> QueryResult {
        QueryResult {
            columns: columns.iter().map(|column| (*column).to_string()).collect(),
            rows,
            ..empty_query_result(Vec::new())
        }
    }

    #[test]
    fn dml_sentinel_is_removed_from_output_result() {
        let output = result(&["id"], vec![vec![json!(7)]]);
        let sentinel = result(&[helpers::AFFECTED_ROWS_COLUMN], vec![vec![json!(1)]]);

        let actual = finish_result_bearing_dml(vec![output, sentinel]).unwrap();

        assert_eq!(actual.columns, ["id"]);
        assert_eq!(actual.rows, vec![vec![json!(7)]]);
        assert_eq!(actual.affected_rows, 1);
        assert!(actual.additional_results.is_none());
    }

    #[test]
    fn dml_sentinel_preserves_additional_result_sets() {
        let first = result(&["before"], vec![vec![json!("first")]]);
        let output = result(&["id"], vec![vec![json!(9)]]);
        let sentinel = result(&[helpers::AFFECTED_ROWS_COLUMN], vec![vec![json!(3)]]);

        let actual = finish_result_bearing_dml(vec![first, output, sentinel]).unwrap();

        assert_eq!(actual.affected_rows, 3);
        let additional = actual.additional_results.unwrap();
        assert_eq!(additional.len(), 1);
        assert_eq!(additional[0].columns, ["id"]);
        assert_eq!(additional[0].rows, vec![vec![json!(9)]]);
    }

    #[test]
    fn dml_without_output_returns_only_affected_count() {
        let sentinel = result(&[helpers::AFFECTED_ROWS_COLUMN], vec![vec![json!(2)]]);

        let actual = finish_result_bearing_dml(vec![sentinel]).unwrap();

        assert!(actual.columns.is_empty());
        assert!(actual.rows.is_empty());
        assert_eq!(actual.affected_rows, 2);
        assert!(actual.additional_results.is_none());
    }

    #[test]
    fn dml_affected_count_accepts_js_unsafe_integer_string() {
        let sentinel = result(
            &[helpers::AFFECTED_ROWS_COLUMN],
            vec![vec![json!("9007199254740992")]],
        );

        let actual = finish_result_bearing_dml(vec![sentinel]).unwrap();

        assert_eq!(actual.affected_rows, 9_007_199_254_740_992);
    }

    #[test]
    fn dml_requires_a_well_formed_trailing_sentinel() {
        let output = result(&["id"], vec![vec![json!(7)]]);
        let error = finish_result_bearing_dml(vec![output]).unwrap_err();

        assert!(error.contains("did not return affected rows"));
    }
}
