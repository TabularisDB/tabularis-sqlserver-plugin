use super::*;
use serde_json::json;

fn request(kind: &str, columns: &[&str], limit: Option<u32>) -> TemplateRequest {
    serde_json::from_value(json!({
        "table": "order]details", "schema": "sales]archive", "kind": kind,
        "columns": columns, "limit": limit,
    }))
    .unwrap()
}

#[test]
fn select_templates_use_top_and_quote_every_identifier() {
    assert_eq!(
        build(&request("select", &["id", "order]name"], Some(100))).unwrap(),
        "SELECT TOP (100)\n  [id],\n  [order]]name]\nFROM [sales]]archive].[order]]details];"
    );
    assert_eq!(
        build(&request("select", &[], Some(100))).unwrap(),
        "SELECT TOP (100) *\nFROM [sales]]archive].[order]]details];"
    );
    assert_eq!(
        build(&request("select", &[], None)).unwrap(),
        "SELECT *\nFROM [sales]]archive].[order]]details];"
    );
    assert!(build(&request("select", &[], Some(0)))
        .unwrap()
        .contains("TOP (0)"));
}

#[test]
fn modification_templates_are_guarded_and_have_unique_host_placeholders() {
    assert_eq!(build(&request("update", &["a b", "a-b", "1"], None)).unwrap(),
        "UPDATE [sales]]archive].[order]]details]\nSET\n  [a b] = :value_1,\n  [a-b] = :value_2,\n  [1] = :value_3\nWHERE 1 = 0;");
    assert!(build(&request("update", &[], None))
        .unwrap()
        .contains("[column] = :value_1"));
    assert_eq!(
        build(&request("delete", &[], None)).unwrap(),
        "DELETE\nFROM [sales]]archive].[order]]details]\nWHERE 1 = 0;"
    );
    assert!(build(&request("update", &[], Some(100))).is_err());
    assert!(build(&request("delete", &[], Some(100))).is_err());
}

#[test]
fn omitted_optional_fields_keep_default_schema_and_unbounded_select() {
    let request = serde_json::from_value(json!({ "kind": "select", "table": "users" })).unwrap();
    assert_eq!(build(&request).unwrap(), "SELECT *\nFROM [dbo].[users];");
    let mut invalid = request;
    invalid.table = " ".into();
    assert!(build(&invalid).is_err());
    assert!(build(&self::request("select", &[""], None)).is_err());
}

#[test]
fn rpc_generates_without_a_connection_and_rejects_malformed_inputs() {
    // Match the production worker stack: the shared async dispatcher also
    // contains large driver futures unrelated to this pure generation RPC.
    let response = std::thread::Builder::new()
        .stack_size(crate::WORKER_STACK_SIZE)
        .spawn(|| {
            let line = json!({
                "jsonrpc": "2.0", "id": 17, "method": "get_table_query_template",
                "params": { "request": { "table": "users", "kind": "select", "limit": 100 } },
            })
            .to_string();
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(Box::pin(crate::rpc::handle_line(&line)))
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(response["id"], 17);
    assert_eq!(
        response["result"],
        "SELECT TOP (100) *\nFROM [dbo].[users];"
    );
    for request in [
        json!({ "table": "users", "kind": "drop" }),
        json!({ "table": "users", "kind": "select", "limit": -1 }),
        json!({ "table": "users", "kind": "select", "limit": "100; DROP TABLE users" }),
    ] {
        let response = crate::handlers::query_templates::get_table_query_template(
            json!(1),
            &json!({ "request": request }),
        );
        assert!(response.get("error").is_some());
    }
    assert!(
        serde_json::from_str::<serde_json::Value>(include_str!("../../../.tabularium")).unwrap()
            ["capabilities"]["table_query_templates"]
            .as_bool()
            .unwrap()
    );
}
