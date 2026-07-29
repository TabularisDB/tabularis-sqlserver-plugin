//! Execution-plan capture via `SET SHOWPLAN_XML` / `SET STATISTICS XML`.

use crate::driver::extract::extract_value;
use crate::driver::pool::BridgeConnection;

/// Fetch the SHOWPLAN XML document for `query`.
///
/// `analyze: false` uses `SHOWPLAN_XML` (estimated plan, statement not run);
/// `analyze: true` uses `STATISTICS XML`, which intentionally executes the
/// statement, matching the explicit Analyze action of the other drivers.
pub async fn explain_showplan_xml(
    conn: &mut BridgeConnection,
    query: &str,
    analyze: bool,
) -> Result<String, String> {
    let option = if analyze {
        "STATISTICS XML"
    } else {
        "SHOWPLAN_XML"
    };
    conn.simple_query(format!("SET {option} ON"))
        .await
        .map_err(|error| error.to_string())?
        .into_results();

    let query_result = conn
        .simple_query(query)
        .await
        .map(|result| result.into_results())
        .map_err(|error| error.to_string());
    let disable_result = conn
        .simple_query(format!("SET {option} OFF"))
        .await
        .map_err(|error| error.to_string())
        .map(|result| {
            result.into_results();
        });

    let result_sets = query_result?;
    disable_result?;
    result_sets
        .iter()
        .flat_map(|rows| rows.iter())
        .flat_map(|row| (0..row.columns().len()).map(move |index| extract_value(row, index)))
        .find_map(|value| {
            value
                .as_str()
                .filter(|text| text.contains("ShowPlanXML"))
                .map(str::to_string)
        })
        .ok_or_else(|| "SQL Server did not return a SHOWPLAN_XML document".to_string())
}
