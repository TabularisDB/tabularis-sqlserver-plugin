use super::*;

#[test]
fn catalog_separates_database_schema_and_object_permissions() {
    let catalog = privilege_catalog();
    assert!(catalog.database.contains(&"SELECT".to_string()));
    assert!(catalog.global.contains(&"CREATE TABLE".to_string()));
    assert!(!catalog.database.contains(&"CREATE TABLE".to_string()));
    assert!(catalog.table.contains(&"RECEIVE".to_string()));
}

#[test]
fn wire_scopes_map_to_database_schema_and_object() {
    assert_eq!(
        RequestedScope::from_wire(None, None).unwrap(),
        RequestedScope::Database
    );
    assert_eq!(
        RequestedScope::from_wire(Some("sales"), None).unwrap(),
        RequestedScope::Schema("sales".to_string())
    );
    assert_eq!(
        RequestedScope::from_wire(Some("sales"), Some("orders")).unwrap(),
        RequestedScope::Object {
            schema: "sales".to_string(),
            object: "orders".to_string()
        }
    );
    assert!(RequestedScope::from_wire(None, Some("orders")).is_err());
}

#[test]
fn targets_bracket_quote_every_identifier() {
    assert_eq!(
        RequestedScope::Database.target_sql("db]name"),
        "DATABASE::[db]]name]"
    );
    assert_eq!(
        RequestedScope::Schema("schema]name".to_string()).target_sql("ignored"),
        "SCHEMA::[schema]]name]"
    );
    assert_eq!(
        RequestedScope::Object {
            schema: "odd]schema".to_string(),
            object: "odd]object".to_string(),
        }
        .target_sql("ignored"),
        "OBJECT::[odd]]schema].[odd]]object]"
    );
}

#[test]
fn privileges_are_scope_validated_and_deduplicated() {
    let database = canonical_privileges(
        &RequestedScope::Database,
        &[
            "select".to_string(),
            " SELECT ".to_string(),
            "SHOWPLAN".to_string(),
        ],
    )
    .unwrap();
    assert_eq!(database, ["SELECT", "SHOWPLAN"]);

    let schema = canonical_privileges(
        &RequestedScope::Schema("dbo".to_string()),
        &["CREATE TABLE".to_string()],
    );
    assert!(schema.unwrap_err().contains("Unsupported"));
}

#[test]
fn password_errors_are_redacted() {
    let password = "Secret'Value";
    let error = redact_password(
        "server rejected Secret'Value represented as Secret''Value".to_string(),
        password,
    );
    assert!(!error.contains("Secret"));
    assert!(error.contains("[REDACTED]"));
}

#[test]
fn permission_rendering_marks_scope_deny_and_grant_option() {
    let database = Permission {
        source: "DIRECT".to_string(),
        state: "GRANT".to_string(),
        name: "CONNECT".to_string(),
        scope: PermissionScope::Database("db]name".to_string()),
    };
    assert_eq!(
        permission_sql(&database, "reader"),
        "GRANT CONNECT ON DATABASE::[db]]name] TO [reader]"
    );

    let denied = Permission {
        source: "DIRECT".to_string(),
        state: "DENY".to_string(),
        name: "DELETE".to_string(),
        scope: PermissionScope::Schema("sales".to_string()),
    };
    assert_eq!(
        permission_sql(&denied, "reader]name"),
        "DENY DELETE ON SCHEMA::[sales] TO [reader]]name]"
    );

    let grantable = Permission {
        source: "DIRECT".to_string(),
        state: "GRANT_WITH_GRANT_OPTION".to_string(),
        name: "SELECT".to_string(),
        scope: PermissionScope::Object {
            schema: "sales".to_string(),
            object: "orders".to_string(),
        },
    };
    assert_eq!(
        permission_sql(&grantable, "reader"),
        "GRANT SELECT ON OBJECT::[sales].[orders] TO [reader] WITH GRANT OPTION"
    );
}
