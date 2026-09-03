//! Cross-module regression tests for the SQL construction audit.
//!
//! These tests deliberately use identifiers that are reserved words, start
//! with a digit, contain Unicode, quotes, and closing brackets. Each public
//! pure SQL builder is exercised here; live JSON-RPC coverage proves the same
//! quoting survives execution against SQL Server.

use crate::driver::{ddl, helpers, ops, routines, triggers, users};
use crate::models::{ColumnDefinition, RoutineCallArg, RoutineParameter};

const SCHEMA: &str = "9schéma]'";
const TABLE: &str = "[weird\"name]]";
const COLUMN: &str = "order";
const OTHER_COLUMN: &str = "Δ\"value]";

fn column(name: &str, data_type: &str) -> ColumnDefinition {
    ColumnDefinition {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: false,
        is_pk: false,
        is_auto_increment: false,
        default_value: None,
    }
}

#[test]
fn identifier_and_crud_builders_quote_every_hostile_identifier() {
    let qualified = "[9schéma]]'].[[weird\"name]]]]]";
    assert_eq!(helpers::bracket_quote(TABLE), "[[weird\"name]]]]]");
    assert_eq!(helpers::quote_identifier("Δ\"value]"), "\"Δ\"\"value]\"");
    assert_eq!(helpers::qualify(Some(SCHEMA), TABLE), qualified);
    assert_eq!(helpers::escape_single_quoted("a'b"), "a''b");

    let columns = vec![COLUMN.to_string(), OTHER_COLUMN.to_string()];
    let insert = helpers::build_insert_sql(Some(SCHEMA), TABLE, &columns, true);
    assert!(insert.contains(&format!(
        "INSERT INTO {qualified} ([order], [Δ\"value]]]) VALUES (@P1, @P2)"
    )));
    assert_eq!(
        insert
            .matches(&format!("SET IDENTITY_INSERT {qualified} OFF"))
            .count(),
        2
    );

    assert_eq!(
        helpers::build_pk_where_clause(&columns, 2).unwrap(),
        "[order] = @P2 AND [Δ\"value]]] = @P3"
    );
    assert_eq!(
        helpers::build_update_composite_sql(Some(SCHEMA), TABLE, OTHER_COLUMN, &[COLUMN.into()])
            .unwrap(),
        format!("UPDATE {qualified} SET [Δ\"value]]] = @P1 WHERE [order] = @P2")
    );
    assert_eq!(
        helpers::build_delete_composite_sql(Some(SCHEMA), TABLE, &[COLUMN.into()]).unwrap(),
        format!("DELETE FROM {qualified} WHERE [order] = @P1")
    );

    let wrapped = helpers::wrap_dml_with_rowcount("UPDATE [safe] SET [value] = @P1");
    assert!(wrapped.ends_with("SELECT CAST(@@ROWCOUNT AS BIGINT) AS [__tabularis_affected_rows];"));
    assert!(helpers::build_paginated_query("SELECT 1", 10, 1)
        .ends_with("OFFSET 0 ROWS FETCH NEXT 11 ROWS ONLY"));
}

#[test]
fn column_and_ddl_builders_quote_every_identifier() {
    let qualified = helpers::qualify(Some(SCHEMA), TABLE);
    let mut key = column(COLUMN, "INT");
    key.is_pk = true;
    key.is_auto_increment = true;
    key.default_value = Some("(7)".to_string());
    assert_eq!(
        helpers::render_column_definition(&key, true),
        "[order] INT IDENTITY(1,1) NOT NULL DEFAULT (7) PRIMARY KEY"
    );

    let create_table = ops::get_create_table_sql(TABLE, vec![key.clone()], Some(SCHEMA)).unwrap();
    assert!(create_table[0].starts_with(&format!("CREATE TABLE {qualified}")));
    assert!(create_table[0].contains("PRIMARY KEY ([order])"));

    let add =
        ops::get_add_column_sql(TABLE, column(OTHER_COLUMN, "NVARCHAR(20)"), Some(SCHEMA)).unwrap();
    assert_eq!(
        add[0],
        format!("ALTER TABLE {qualified} ADD [Δ\"value]]] NVARCHAR(20) NOT NULL")
    );

    let index_name = "9índex\"]";
    let index = ops::get_create_index_sql(
        TABLE,
        index_name,
        vec![COLUMN.into(), OTHER_COLUMN.into()],
        true,
        Some(SCHEMA),
    )
    .unwrap();
    assert_eq!(
        index[0],
        format!("CREATE UNIQUE INDEX [9índex\"]]] ON {qualified} ([order], [Δ\"value]]])")
    );

    let fk_name = "9fk\"]";
    let referenced = "9référence\"]";
    let direct_fk = ddl::create_foreign_key_sql(
        TABLE,
        fk_name,
        COLUMN,
        referenced,
        OTHER_COLUMN,
        Some("cascade"),
        Some("set null"),
        Some(SCHEMA),
    )
    .unwrap();
    let rpc_fk = ops::get_create_foreign_key_sql(
        TABLE,
        fk_name,
        COLUMN,
        referenced,
        OTHER_COLUMN,
        Some("cascade"),
        Some("set null"),
        Some(SCHEMA),
    )
    .unwrap();
    assert_eq!(direct_fk, rpc_fk);
    assert!(direct_fk[0].contains("CONSTRAINT [9fk\"]]] FOREIGN KEY ([order])"));
    assert!(direct_fk[0].contains("REFERENCES [9schéma]]'].[9référence\"]]] ([Δ\"value]]])"));
}

#[test]
fn alter_column_quotes_multipart_names_and_escapes_metadata_literals() {
    let old = column("old'name]", "INT");
    let mut new = column(COLUMN, "BIGINT");
    new.is_nullable = true;
    new.default_value = Some("(42)".to_string());

    let direct = ddl::alter_column_sql(TABLE, &old, &new, Some(SCHEMA)).unwrap();
    let rpc = ops::get_alter_column_sql(TABLE, old, new, Some(SCHEMA)).unwrap();
    assert_eq!(direct, rpc);
    assert!(direct[0].contains("[9schéma]]''].[[weird\"name]]]]].[old''name]]]"));
    assert!(direct[0].contains(", N'order', N'COLUMN'"));
    assert_eq!(
        direct[1],
        "ALTER TABLE [9schéma]]'].[[weird\"name]]]]] ALTER COLUMN [order] BIGINT NULL"
    );
    assert!(direct[2].contains("OBJECT_ID(N'[9schéma]]''].[[weird\"name]]]]]')"));
    assert!(direct[2].contains("c.[name] = N'order'"));
    assert!(direct[3].contains("ADD CONSTRAINT [DF_[weird\"name]]]]_order]"));
}

#[test]
fn directly_executed_drop_and_view_builders_quote_identifiers() {
    let qualified = helpers::qualify(Some(SCHEMA), TABLE);
    let definition = "SELECT CAST(1 AS INT) AS [order]";
    assert_eq!(
        ops::build_create_view_sql(TABLE, definition, Some(SCHEMA)),
        format!("CREATE VIEW {qualified} AS {definition}")
    );
    assert_eq!(
        ops::build_alter_view_sql(TABLE, definition, Some(SCHEMA)),
        format!("ALTER VIEW {qualified} AS {definition}")
    );
    assert_eq!(
        ops::build_drop_view_sql(TABLE, Some(SCHEMA)),
        format!("DROP VIEW IF EXISTS {qualified}")
    );
    assert_eq!(
        ops::build_drop_index_sql(TABLE, "9índex\"]", Some(SCHEMA)),
        format!("DROP INDEX [9índex\"]]] ON {qualified}")
    );
    assert_eq!(
        ops::build_drop_foreign_key_sql(TABLE, "9fk\"]", Some(SCHEMA)),
        format!("ALTER TABLE {qualified} DROP CONSTRAINT [9fk\"]]]")
    );
    assert_eq!(
        triggers::drop_trigger_sql(TABLE, Some(SCHEMA)),
        format!("DROP TRIGGER {qualified}")
    );
}

#[test]
fn routine_builders_escape_literals_and_preserve_only_explicit_raw_expressions() {
    let args = [
        RoutineCallArg {
            name: "order".to_string(),
            mode: "IN".to_string(),
            value: Some("x'); DROP TABLE victims;--".to_string()),
            is_raw: false,
        },
        RoutineCallArg {
            name: "raw_value".to_string(),
            mode: "IN".to_string(),
            value: Some("DATEADD(day, 1, SYSDATETIME())".to_string()),
            is_raw: true,
        },
        RoutineCallArg {
            name: "out_value".to_string(),
            mode: "OUT".to_string(),
            value: None,
            is_raw: false,
        },
    ];
    let parameters = [RoutineParameter {
        name: "@out_value".to_string(),
        data_type: "NVARCHAR(20)".to_string(),
        mode: "OUT".to_string(),
        ordinal_position: 3,
    }];
    let sql =
        routines::routine_call_sql(TABLE, "PROCEDURE", &args, &parameters, false, Some(SCHEMA))
            .unwrap();
    assert!(sql.contains("@order = N'x''); DROP TABLE victims;--'"));
    assert!(sql.contains("@raw_value = DATEADD(day, 1, SYSDATETIME())"));
    assert!(sql.contains("EXEC [9schéma]]'].[[weird\"name]]]]]"));
    assert!(sql.contains("@tabularis_output_2 AS [out_value]"));

    assert!(routines::routine_create_template("FUNCTION", Some(SCHEMA))
        .starts_with("CREATE FUNCTION [9schéma]]'].[my_function]"));
    assert_eq!(
        routines::routine_edit_script("CREATE PROCEDURE [safe].[p] AS SELECT 1").unwrap(),
        "ALTER PROCEDURE [safe].[p] AS SELECT 1"
    );
    assert_eq!(
        routines::drop_routine_sql(TABLE, "FUNCTION", Some(SCHEMA)),
        "DROP FUNCTION [9schéma]]'].[[weird\"name]]]]]"
    );
}

#[test]
fn user_management_builders_quote_names_and_escape_unbindable_password_literals() {
    let user = "9usér\"]";
    let login = "9lógin\"]";
    let password = "S3cret'); DROP LOGIN victim;--";

    assert_eq!(
        users::build_create_user_sql(user, login),
        "CREATE USER [9usér\"]]] FOR LOGIN [9lógin\"]]]"
    );
    assert_eq!(users::build_drop_user_sql(user), "DROP USER [9usér\"]]]");
    assert_eq!(
        users::build_drop_login_sql(login),
        "DROP LOGIN [9lógin\"]]]"
    );

    let create = users::build_create_login_sql(login, password);
    let alter = users::build_set_password_sql(login, password);
    for sql in [create, alter] {
        assert!(sql.contains("[9lógin\"]]]"));
        assert!(sql.contains("PASSWORD = N'S3cret''); DROP LOGIN victim;--'"));
    }

    assert_eq!(
        users::build_permission_change_sql(
            "9database\"]",
            user,
            Some(SCHEMA),
            Some(TABLE),
            "select",
            true,
        )
        .unwrap(),
        "GRANT SELECT ON OBJECT::[9schéma]]'].[[weird\"name]]]]] TO [9usér\"]]]"
    );
    assert!(users::build_permission_change_sql(
        "database",
        user,
        Some(SCHEMA),
        Some(TABLE),
        "SELECT; DROP TABLE victims",
        true,
    )
    .is_err());
}
