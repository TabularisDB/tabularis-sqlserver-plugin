use super::*;

#[test]
fn bracket_quote_wraps_plain_identifier() {
    assert_eq!(bracket_quote("dbo"), "[dbo]");
    assert_eq!(bracket_quote("MyTable"), "[MyTable]");
}

#[test]
fn bracket_quote_preserves_dots_and_spaces() {
    assert_eq!(bracket_quote("my.table"), "[my.table]");
    assert_eq!(bracket_quote("name with space"), "[name with space]");
}

#[test]
fn bracket_quote_escapes_closing_bracket() {
    assert_eq!(bracket_quote("weird]name"), "[weird]]name]");
    assert_eq!(bracket_quote("]"), "[]]]");
    assert_eq!(bracket_quote("a]]b"), "[a]]]]b]");
}

#[test]
fn bracket_quote_handles_empty_string() {
    assert_eq!(bracket_quote(""), "[]");
}

#[test]
fn bracket_quote_leaves_other_specials_intact() {
    // Brackets and ] are the only metacharacters inside [..] — square
    // brackets are *not* regex there, and single quotes are irrelevant.
    assert_eq!(bracket_quote("a'b\"c"), "[a'b\"c]");
}

#[test]
fn quote_identifier_wraps_and_escapes_double_quote() {
    assert_eq!(quote_identifier("col"), "\"col\"");
    assert_eq!(quote_identifier("weird\"name"), "\"weird\"\"name\"");
    assert_eq!(quote_identifier(""), "\"\"");
}

#[test]
fn qualify_uses_dbo_when_schema_missing() {
    assert_eq!(qualify(None, "Users"), "[dbo].[Users]");
    assert_eq!(qualify(Some(""), "Users"), "[dbo].[Users]");
    assert_eq!(qualify(Some("   "), "Users"), "[dbo].[Users]");
}

#[test]
fn qualify_keeps_explicit_schema() {
    assert_eq!(qualify(Some("sales"), "Orders"), "[sales].[Orders]");
}

#[test]
fn qualify_escapes_brackets_in_both_parts() {
    assert_eq!(qualify(Some("we]ird"), "ta]ble"), "[we]]ird].[ta]]ble]");
}

#[test]
fn escape_single_quoted_doubles_apostrophes() {
    assert_eq!(escape_single_quoted("o'brien"), "o''brien");
    assert_eq!(escape_single_quoted("'''"), "''''''");
    assert_eq!(escape_single_quoted("plain"), "plain");
}

#[test]
fn bracket_quote_is_round_trip_safe_through_itself() {
    // Quoting an already-quoted identifier is a useful invariant for
    // nested composition: bracket_quote(bracket_quote(x)) must still be
    // parseable — it just adds another layer of brackets.
    let once = bracket_quote("weird]name");
    let twice = bracket_quote(&once);
    assert!(twice.starts_with('['));
    assert!(twice.ends_with(']'));
    // Inner brackets ']' are each doubled again.
    assert!(twice.contains("]]]]"));
}

#[test]
fn build_insert_sql_plain_emits_positional_placeholders() {
    let sql = build_insert_sql(
        None,
        "Users",
        &["id".to_string(), "name".to_string(), "email".to_string()],
        false,
    );
    assert_eq!(
        sql,
        "INSERT INTO [dbo].[Users] ([id], [name], [email]) VALUES (@P1, @P2, @P3);\n\
         SELECT CAST(@@ROWCOUNT AS BIGINT) AS [__tabularis_affected_rows];"
    );
}

#[test]
fn wrap_dml_with_rowcount_keeps_multi_statement_batch_and_single_sentinel() {
    let batch = "SET NOCOUNT ON; SELECT 1; UPDATE [dbo].[Users] SET [active] = 1";
    let sql = wrap_dml_with_rowcount(batch);

    assert!(sql.starts_with(batch));
    assert_eq!(sql.matches(AFFECTED_ROWS_COLUMN).count(), 1);
    assert!(sql.ends_with("SELECT CAST(@@ROWCOUNT AS BIGINT) AS [__tabularis_affected_rows];"));
}

#[test]
fn build_insert_sql_plain_quotes_column_identifiers() {
    let sql = build_insert_sql(
        Some("sales"),
        "Orders",
        &["order id".to_string(), "weird]col".to_string()],
        false,
    );
    assert!(sql.contains("([order id], [weird]]col])"));
    assert!(sql.contains("VALUES (@P1, @P2)"));
}

#[test]
fn build_insert_sql_with_identity_wraps_in_try_catch() {
    let sql = build_insert_sql(None, "Users", &["id".to_string(), "name".to_string()], true);
    assert!(sql.contains("BEGIN TRY"));
    assert!(sql.contains("SET IDENTITY_INSERT [dbo].[Users] ON;"));
    assert!(sql.contains("INSERT INTO [dbo].[Users] ([id], [name]) VALUES (@P1, @P2);"));
    assert!(sql.contains("SET IDENTITY_INSERT [dbo].[Users] OFF;"));
    assert!(sql.contains("BEGIN CATCH"));
    assert!(sql.contains("THROW;"));
    // No BEGIN TRAN/COMMIT: the TDS client rejects transaction statements
    // inside an sp_executesql RPC batch (error 3981), and a single INSERT
    // is atomic without one.
    assert!(!sql.contains("TRAN"));
    // The OFF guard must appear both on success and in CATCH so the
    // session-scoped setting cannot leak when an insert fails.
    let off_count = sql
        .matches("SET IDENTITY_INSERT [dbo].[Users] OFF;")
        .count();
    assert_eq!(off_count, 2);
    // @@ROWCOUNT must be captured immediately after the INSERT (the later
    // SET IDENTITY_INSERT resets it) and selected at the end of the batch.
    assert!(sql.contains(
        "INSERT INTO [dbo].[Users] ([id], [name]) VALUES (@P1, @P2);\n\
         SET @tabularis_affected = @@ROWCOUNT;\n\
         SET IDENTITY_INSERT [dbo].[Users] OFF;"
    ));
    assert!(
        sql.contains("SELECT CAST(@tabularis_affected AS BIGINT) AS [__tabularis_affected_rows];")
    );
}

#[test]
fn build_insert_sql_with_identity_quotes_its_target() {
    let sql = build_insert_sql(Some("9schéma]"), "weird\"name]", &["k".to_string()], true);
    assert!(sql.contains("SET IDENTITY_INSERT [9schéma]]].[weird\"name]]] ON;"));
    assert!(sql.contains("SET IDENTITY_INSERT [9schéma]]].[weird\"name]]] OFF;"));
}

#[test]
fn value_to_sql_param_accepts_supported_json_variants() {
    for value in [
        serde_json::Value::Null,
        serde_json::json!(true),
        serde_json::json!(42),
        serde_json::json!(-7_i64),
        serde_json::json!(3.5),
        serde_json::json!("hello"),
        serde_json::json!([1, 2, 3]),
        serde_json::json!({"k": 1}),
    ] {
        assert!(value_to_sql_param(&value).is_ok(), "rejected {value}");
    }
}

// --- composite PK SQL builders (issue #145) ----------------------------

#[test]
fn value_to_sql_param_rejects_unsigned_bigint_overflow() {
    let value = serde_json::json!(u64::MAX);
    let error = value_to_sql_param(&value).expect_err("overflow must be rejected");
    assert!(error.contains("BIGINT range"));
}

#[test]
fn pk_where_clause_returns_none_for_empty_cols() {
    assert_eq!(build_pk_where_clause(&[], 1), None);
}

#[test]
fn pk_where_clause_single_column_starts_at_p1() {
    assert_eq!(
        build_pk_where_clause(&["id".to_string()], 1),
        Some("[id] = @P1".to_string())
    );
}

#[test]
fn pk_where_clause_composite_chains_with_and() {
    assert_eq!(
        build_pk_where_clause(&["tenant_id".to_string(), "user_id".to_string()], 1),
        Some("[tenant_id] = @P1 AND [user_id] = @P2".to_string())
    );
}

#[test]
fn pk_where_clause_offset_marker_for_update_path() {
    // UPDATE binds the new value at @P1, so the PK markers start at @P2.
    assert_eq!(
        build_pk_where_clause(&["a".to_string(), "b".to_string()], 2),
        Some("[a] = @P2 AND [b] = @P3".to_string())
    );
}

#[test]
fn pk_where_clause_escapes_brackets_in_column_names() {
    assert_eq!(
        build_pk_where_clause(&["we]ird".to_string()], 1),
        Some("[we]]ird] = @P1".to_string())
    );
}

#[test]
fn delete_composite_sql_uses_dbo_when_schema_missing() {
    let sql = build_delete_composite_sql(None, "Users", &["id".to_string()]).unwrap();
    assert_eq!(sql, "DELETE FROM [dbo].[Users] WHERE [id] = @P1");
}

#[test]
fn delete_composite_sql_chains_composite_keys() {
    let sql = build_delete_composite_sql(
        Some("sales"),
        "OrderItems",
        &["order_id".to_string(), "line_no".to_string()],
    )
    .unwrap();
    assert_eq!(
        sql,
        "DELETE FROM [sales].[OrderItems] WHERE [order_id] = @P1 AND [line_no] = @P2"
    );
}

#[test]
fn delete_composite_sql_returns_none_without_pk() {
    assert_eq!(build_delete_composite_sql(None, "Users", &[]), None);
}

#[test]
fn update_composite_sql_binds_new_value_at_p1_and_pk_at_p2() {
    let sql = build_update_composite_sql(None, "Users", "email", &["id".to_string()]).unwrap();
    assert_eq!(
        sql,
        "UPDATE [dbo].[Users] SET [email] = @P1 WHERE [id] = @P2"
    );
}

#[test]
fn update_composite_sql_chains_composite_keys_starting_at_p2() {
    let sql = build_update_composite_sql(
        Some("sales"),
        "OrderItems",
        "qty",
        &["order_id".to_string(), "line_no".to_string()],
    )
    .unwrap();
    assert_eq!(
        sql,
        "UPDATE [sales].[OrderItems] SET [qty] = @P1 WHERE [order_id] = @P2 AND [line_no] = @P3"
    );
}

#[test]
fn update_composite_sql_returns_none_without_pk() {
    assert_eq!(
        build_update_composite_sql(None, "Users", "email", &[]),
        None
    );
}

#[test]
fn update_composite_sql_escapes_brackets_in_column_and_pk_names() {
    let sql =
        build_update_composite_sql(Some("we]ird"), "ta]ble", "co]l", &["p]k".to_string()]).unwrap();
    assert_eq!(
        sql,
        "UPDATE [we]]ird].[ta]]ble] SET [co]]l] = @P1 WHERE [p]]k] = @P2"
    );
}

#[test]
fn render_column_definition_handles_identity_default_and_pk() {
    let column = ColumnDefinition {
        name: "order]id".into(),
        data_type: "INT".into(),
        is_nullable: false,
        is_pk: true,
        is_auto_increment: true,
        default_value: Some("(1)".into()),
    };
    assert_eq!(
        render_column_definition(&column, true),
        "[order]]id] INT IDENTITY(1,1) NOT NULL DEFAULT (1) PRIMARY KEY"
    );
}

#[test]
fn classifier_handles_cte_followed_by_select() {
    let query = "WITH cte AS (SELECT 1 AS id) SELECT id FROM cte";
    assert!(query_returns_result_set(query));
    assert!(query_can_be_paginated(query));
    assert!(!query_reports_affected_rows(query));
}

#[test]
fn classifier_handles_cte_followed_by_insert() {
    let query = "WITH source AS (SELECT 1 AS id) INSERT INTO audit(id) SELECT id FROM source";
    assert!(!query_returns_result_set(query));
    assert!(!query_can_be_paginated(query));
    assert!(query_reports_affected_rows(query));
}

#[test]
fn classifier_treats_select_into_as_affected_rows_not_a_result_set() {
    for query in [
        "SELECT id INTO #selected FROM users",
        "WITH source AS (SELECT id FROM users) SELECT id INTO #selected FROM source",
    ] {
        assert!(
            !query_returns_result_set(query),
            "got result set for {query}"
        );
        assert!(!query_can_be_paginated(query), "paginated {query}");
        assert!(
            query_reports_affected_rows(query),
            "lost row count for {query}"
        );
    }
}

#[test]
fn classifier_handles_merge_and_exec() {
    let merge = "MERGE users AS target USING source ON target.id = source.id WHEN MATCHED THEN UPDATE SET target.active = 1";
    assert!(!query_returns_result_set(merge));
    assert!(!query_can_be_paginated(merge));
    assert!(query_reports_affected_rows(merge));
    assert!(query_returns_result_set(&format!(
        "{merge} OUTPUT INSERTED.id"
    )));

    let exec = "EXEC sp_executesql N'SELECT 1 AS value'";
    assert!(query_returns_result_set(exec));
    assert!(!query_can_be_paginated(exec));
    assert!(!query_reports_affected_rows(exec));
}

#[test]
fn classifier_handles_select_from_temp_table() {
    let query = "SELECT id FROM #selected ORDER BY id";
    assert!(query_returns_result_set(query));
    assert!(query_can_be_paginated(query));
    assert!(!query_reports_affected_rows(query));
}

#[test]
fn classifier_handles_mixed_batches() {
    let ending_in_select = "UPDATE users SET active = 1; SELECT id FROM users";
    assert!(query_returns_result_set(ending_in_select));
    assert!(!query_can_be_paginated(ending_in_select));
    assert!(!query_reports_affected_rows(ending_in_select));

    let ending_in_dml = "SELECT id FROM users; DELETE FROM users WHERE active = 0";
    assert!(query_returns_result_set(ending_in_dml));
    assert!(!query_can_be_paginated(ending_in_dml));
    assert!(query_reports_affected_rows(ending_in_dml));
}

#[test]
fn result_set_classification_ignores_literals_comments_and_identifiers() {
    assert!(!query_returns_result_set(
        "UPDATE [SELECT] SET [value] = '; SELECT 1' -- ; SELECT 2"
    ));
    assert!(!query_returns_result_set(
        "UPDATE users SET note = 'order by id' /* order by id; SELECT 1 */"
    ));
}

#[test]
fn affected_rows_are_only_reported_for_final_dml_statement() {
    assert!(query_reports_affected_rows("UPDATE users SET active = 1"));
    assert!(query_reports_affected_rows(
        "UPDATE users SET active = 1 OUTPUT INSERTED.id"
    ));
    assert!(query_reports_affected_rows(
        "SELECT 1; UPDATE users SET active = 1"
    ));
    assert!(query_reports_affected_rows(
        "SET NOCOUNT ON; WITH target AS (SELECT id FROM users) DELETE FROM target"
    ));
    assert!(!query_reports_affected_rows(
        "UPDATE users SET active = 1; SELECT 1"
    ));
    assert!(!query_reports_affected_rows(
        "CREATE PROCEDURE dbo.p AS SELECT 1"
    ));
    assert!(!query_reports_affected_rows("DROP TABLE dbo.items"));
}

#[test]
fn paginated_query_adds_order_when_missing() {
    assert_eq!(
        build_paginated_query("SELECT * FROM [users];", 25, 2),
        "SELECT * FROM [users] ORDER BY (SELECT NULL) OFFSET 25 ROWS FETCH NEXT 26 ROWS ONLY"
    );
}

#[test]
fn paginated_query_preserves_top_level_order_with_sql_whitespace() {
    for query in [
        "SELECT * FROM [users] ORDER BY [id]",
        "SELECT * FROM [users] ORDER\nBY [id]",
        "SELECT * FROM [users] ORDER /* stable key */ BY [id]",
    ] {
        let paginated = build_paginated_query(query, 10, 1);
        assert!(
            !paginated.contains("ORDER BY (SELECT NULL)"),
            "got {paginated}"
        );
        assert!(
            paginated.ends_with("OFFSET 0 ROWS FETCH NEXT 11 ROWS ONLY"),
            "got {paginated}"
        );
    }
}

#[test]
fn paginated_query_ignores_nested_order_in_subquery_and_window() {
    for query in [
        "SELECT * FROM (SELECT TOP 5 * FROM [users] ORDER BY [id]) AS [recent]",
        "SELECT ROW_NUMBER() OVER (ORDER BY [id]) AS [position] FROM [users]",
    ] {
        let paginated = build_paginated_query(query, 10, 1);
        assert!(
            paginated.contains("ORDER BY (SELECT NULL) OFFSET"),
            "got {paginated}"
        );
    }
}

#[test]
fn paginated_query_ignores_order_words_in_literals_and_comments() {
    for query in [
        "SELECT 'order' AS [label]",
        "SELECT 'ORDER BY' AS [label]",
        "SELECT 1 -- order by [id]",
        "SELECT 1 /* the word order and ORDER BY [id] */",
        "SELECT N'città ORDER BY nome'",
    ] {
        let paginated = build_paginated_query(query, 10, 1);
        assert!(
            paginated.contains("ORDER BY (SELECT NULL) OFFSET"),
            "got {paginated}"
        );
    }
}
