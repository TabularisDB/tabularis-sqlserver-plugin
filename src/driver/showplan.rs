//! SHOWPLAN_XML → visual-plan JSON.
//!
//! Parses SQL Server's SHOWPLAN XML document into the `ExplainPlan` shape the
//! Tabularis frontend renders (`@tabularis/explain`'s plan model). The host
//! passes a plugin's `explain_query` result through to the frontend
//! untouched, so the plan must arrive already parsed.

use roxmltree::{Document, Node};
use serde_json::{json, Value};

/// Parse a SHOWPLAN XML document into the shared visual-plan model.
pub fn parse_showplan_xml(raw: &str, original_query: &str) -> Result<Value, String> {
    let document = Document::parse(raw)
        .map_err(|error| format!("Failed to parse SQL Server SHOWPLAN_XML: {error}"))?;
    let operator = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "RelOp")
        .ok_or_else(|| "SQL Server SHOWPLAN_XML does not contain a RelOp".to_string())?;

    let root = parse_operator(operator, 0);
    let actual_time_ms = root.get("actual_time_ms").cloned().unwrap_or(Value::Null);
    let has_analyze_data = !root.get("actual_rows").map(Value::is_null).unwrap_or(true);
    Ok(json!({
        "root": root,
        "planning_time_ms": Value::Null,
        "execution_time_ms": actual_time_ms,
        "original_query": original_query,
        "driver": "sqlserver",
        "has_analyze_data": has_analyze_data,
        "raw_output": raw,
    }))
}

fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name)
        .map(|a| a.value())
}

fn attr_number(node: Node, name: &str) -> Value {
    attr(node, name)
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Descendants of `operator` that do not cross into a nested `RelOp`
/// subtree, i.e. the parts of the plan node that belong to this operator
/// rather than to one of its children.
fn owned_descendant<'a>(operator: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    fn visit<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
        for child in node.children().filter(Node::is_element) {
            if child.tag_name().name() == "RelOp" {
                continue;
            }
            if child.tag_name().name() == name {
                return Some(child);
            }
            if let Some(found) = visit(child, name) {
                return Some(found);
            }
        }
        None
    }
    visit(operator, name)
}

/// Direct child operators: descendant `RelOp` elements whose path from this
/// operator contains no other `RelOp`.
fn child_operators<'a>(operator: Node<'a, 'a>) -> Vec<Node<'a, 'a>> {
    fn visit<'a>(node: Node<'a, 'a>, out: &mut Vec<Node<'a, 'a>>) {
        for child in node.children().filter(Node::is_element) {
            if child.tag_name().name() == "RelOp" {
                out.push(child);
            } else {
                visit(child, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(operator, &mut out);
    out
}

fn relation_name(operator: Node) -> Value {
    match owned_descendant(operator, "Object").and_then(|target| attr(target, "Table")) {
        Some(table) => Value::String(table.replace(['[', ']'], "")),
        None => Value::Null,
    }
}

fn predicate(operator: Node) -> Value {
    match owned_descendant(operator, "ScalarOperator")
        .and_then(|scalar| attr(scalar, "ScalarString"))
    {
        Some(text) => Value::String(text.to_string()),
        None => Value::Null,
    }
}

/// Actual rows / elapsed time / executions summed and maxed across the
/// per-thread runtime counters, when the plan carries analyze data.
fn runtime_metrics(operator: Node) -> (Value, Value, Value) {
    let Some(runtime) = owned_descendant(operator, "RunTimeInformation") else {
        return (Value::Null, Value::Null, Value::Null);
    };
    let counters: Vec<Node> = runtime
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "RunTimeCountersPerThread")
        .collect();
    if counters.is_empty() {
        return (Value::Null, Value::Null, Value::Null);
    }
    let number = |node: Node, name: &str| -> f64 {
        attr(node, name)
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
    };
    let rows: f64 = counters.iter().map(|c| number(*c, "ActualRows")).sum();
    let time = counters
        .iter()
        .map(|c| number(*c, "ActualElapsedms"))
        .fold(f64::NEG_INFINITY, f64::max);
    let loops: f64 = counters
        .iter()
        .map(|c| number(*c, "ActualExecutions"))
        .sum();
    let to_value = |value: f64| {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    };
    (to_value(rows), to_value(time), to_value(loops))
}

fn parse_operator(operator: Node, fallback_id: u64) -> Value {
    let physical = attr(operator, "PhysicalOp")
        .or_else(|| attr(operator, "LogicalOp"))
        .unwrap_or("Unknown")
        .to_string();
    let logical = attr(operator, "LogicalOp")
        .unwrap_or(physical.as_str())
        .to_string();
    let (actual_rows, actual_time_ms, actual_loops) = runtime_metrics(operator);
    let id = attr(operator, "NodeId")
        .map(str::to_string)
        .unwrap_or_else(|| fallback_id.to_string());
    let children: Vec<Value> = child_operators(operator)
        .into_iter()
        .enumerate()
        .map(|(index, child)| parse_operator(child, fallback_id * 10 + index as u64 + 1))
        .collect();
    let join_type = if logical.to_lowercase().contains("join") {
        Value::String(logical.clone())
    } else {
        Value::Null
    };

    json!({
        "id": format!("sqlserver-{id}"),
        "node_type": physical,
        "relation": relation_name(operator),
        "startup_cost": Value::Null,
        "total_cost": attr_number(operator, "EstimatedTotalSubtreeCost"),
        "plan_rows": attr_number(operator, "EstimateRows"),
        "actual_rows": actual_rows,
        "actual_time_ms": actual_time_ms,
        "actual_loops": actual_loops,
        "buffers_hit": Value::Null,
        "buffers_read": Value::Null,
        "filter": predicate(operator),
        "index_condition": Value::Null,
        "join_type": join_type,
        "hash_condition": Value::Null,
        "extra": { "logical_operation": logical },
        "children": children,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_showplan_xml;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-16"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan" Version="1.564">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT * FROM t">
      <QueryPlan>
        <RelOp NodeId="0" PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="10" EstimatedTotalSubtreeCost="0.05">
          <NestedLoops>
            <RelOp NodeId="1" PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="10" EstimatedTotalSubtreeCost="0.02">
              <IndexScan>
                <Object Database="[db]" Schema="[dbo]" Table="[t]" Index="[PK_t]" />
                <Predicate><ScalarOperator ScalarString="[db].[dbo].[t].[id]&gt;(5)" /></Predicate>
              </IndexScan>
              <RunTimeInformation>
                <RunTimeCountersPerThread Thread="0" ActualRows="7" ActualElapsedms="3" ActualExecutions="1" />
                <RunTimeCountersPerThread Thread="1" ActualRows="2" ActualElapsedms="5" ActualExecutions="1" />
              </RunTimeInformation>
            </RelOp>
          </NestedLoops>
        </RelOp>
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    #[test]
    fn parses_root_operator_and_children() {
        let plan = parse_showplan_xml(SAMPLE, "SELECT * FROM t").unwrap();
        assert_eq!(plan["driver"], "sqlserver");
        assert_eq!(plan["original_query"], "SELECT * FROM t");
        let root = &plan["root"];
        assert_eq!(root["id"], "sqlserver-0");
        assert_eq!(root["node_type"], "Nested Loops");
        assert_eq!(root["join_type"], "Inner Join");
        assert_eq!(root["children"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn extracts_relation_predicate_and_runtime_metrics() {
        let plan = parse_showplan_xml(SAMPLE, "").unwrap();
        let child = &plan["root"]["children"][0];
        assert_eq!(child["relation"], "t");
        assert_eq!(child["filter"], "[db].[dbo].[t].[id]>(5)");
        assert_eq!(child["actual_rows"], 9.0);
        assert_eq!(child["actual_time_ms"], 5.0);
        assert_eq!(child["actual_loops"], 2.0);
        assert_eq!(plan["has_analyze_data"], false); // root has no runtime info
    }

    #[test]
    fn rejects_documents_without_relop() {
        let err = parse_showplan_xml("<ShowPlanXML/>", "").unwrap_err();
        assert!(err.contains("RelOp"));
    }

    /// Regenerate the TypeScript port's committed golden files with the Rust
    /// parser that they must match until SS-035 removes this implementation.
    #[test]
    #[ignore = "writes committed explain fixture expectations"]
    fn write_showplan_fixture_goldens() {
        let fixture_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("explain/tests/fixtures");
        let expected_dir = fixture_dir.join("expected");
        std::fs::create_dir_all(&expected_dir).unwrap();

        for entry in std::fs::read_dir(&fixture_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("xml") {
                continue;
            }

            let xml = std::fs::read_to_string(&path).unwrap();
            let plan = parse_showplan_xml(&xml, "").unwrap();
            let mut json = serde_json::to_string_pretty(&plan).unwrap();
            json.push('\n');
            let name = path.file_stem().unwrap();
            std::fs::write(expected_dir.join(name).with_extension("json"), json).unwrap();
        }
    }
}
