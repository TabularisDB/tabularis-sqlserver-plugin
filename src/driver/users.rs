//! SQL Server database-user and database-permission management.
//!
//! Tabularis models MySQL-style global/database/table scopes. For SQL Server
//! we map those three wire shapes to database/schema/object respectively:
//! `(None, None)`, `(Some(schema), None)`, and
//! `(Some(schema), Some(object))`.

use std::collections::BTreeMap;

use mssql_tds::message::transaction_management::TransactionIsolationLevel;
use mssql_tiberius_bridge::Row;

use crate::driver::helpers::bracket_quote;
use crate::driver::pool::BridgeConnection;
use crate::models::{DbPrivilegeCatalog, DbUserGrantSet, DbUserInfo};

const DATABASE_AND_SCHEMA_PRIVILEGES: &[&str] = &[
    "ALTER",
    "CONTROL",
    "DELETE",
    "EXECUTE",
    "INSERT",
    "REFERENCES",
    "SELECT",
    "TAKE OWNERSHIP",
    "UPDATE",
    "VIEW CHANGE TRACKING",
    "VIEW DEFINITION",
];

const DATABASE_ONLY_PRIVILEGES: &[&str] = &[
    "AUTHENTICATE",
    "BACKUP DATABASE",
    "BACKUP LOG",
    "CHECKPOINT",
    "CONNECT",
    "CREATE FUNCTION",
    "CREATE PROCEDURE",
    "CREATE ROLE",
    "CREATE SCHEMA",
    "CREATE SYNONYM",
    "CREATE TABLE",
    "CREATE TYPE",
    "CREATE VIEW",
    "SHOWPLAN",
    "SUBSCRIBE QUERY NOTIFICATIONS",
    "UNMASK",
    "VIEW DATABASE STATE",
];

const OBJECT_PRIVILEGES: &[&str] = &[
    "ALTER",
    "CONTROL",
    "DELETE",
    "EXECUTE",
    "INSERT",
    "RECEIVE",
    "REFERENCES",
    "SELECT",
    "TAKE OWNERSHIP",
    "UPDATE",
    "VIEW CHANGE TRACKING",
    "VIEW DEFINITION",
];

const LIST_USERS: &str = r#"
SELECT dp.name AS user_name,
       sp.name AS login_name,
       CAST(ISNULL(LOGINPROPERTY(sp.name, 'IsLocked'), 0) AS bit) AS is_locked
FROM sys.database_principals AS dp
JOIN sys.server_principals AS sp
  ON sp.sid = dp.sid AND sp.type = 'S'
WHERE dp.type = 'S'
  AND dp.authentication_type = 1
  AND dp.principal_id > 4
  AND dp.name NOT IN ('dbo', 'guest', 'INFORMATION_SCHEMA', 'sys')
ORDER BY dp.name, sp.name
"#;

const ACCOUNT_EXISTS: &str = r#"
SELECT CAST(CASE WHEN EXISTS (
    SELECT 1
    FROM sys.database_principals AS dp
    JOIN sys.server_principals AS sp ON sp.sid = dp.sid AND sp.type = 'S'
    WHERE dp.type = 'S' AND dp.authentication_type = 1
      AND dp.name = @P1 AND sp.name = @P2
) THEN 1 ELSE 0 END AS bit)
"#;

const LOGIN_EXISTS: &str = r#"
SELECT CAST(CASE WHEN EXISTS (
    SELECT 1 FROM sys.server_principals WHERE type = 'S' AND name = @P1
) THEN 1 ELSE 0 END AS bit)
"#;

const USER_EXISTS: &str = r#"
SELECT CAST(CASE WHEN EXISTS (
    SELECT 1 FROM sys.database_principals WHERE name = @P1
) THEN 1 ELSE 0 END AS bit)
"#;

const DIRECT_PERMISSIONS: &str = r#"
SELECT CAST('DIRECT' AS nvarchar(128)) AS source_name,
       p.state_desc,
       p.permission_name,
       p.class_desc,
       CASE WHEN p.class = 3 THEN SCHEMA_NAME(p.major_id)
            WHEN p.class = 1 THEN OBJECT_SCHEMA_NAME(p.major_id)
            ELSE DB_NAME() END AS scope_name,
       CASE WHEN p.class = 1 THEN OBJECT_NAME(p.major_id) ELSE NULL END AS object_name
FROM sys.database_permissions AS p
JOIN sys.database_principals AS grantee ON grantee.principal_id = p.grantee_principal_id
WHERE grantee.name = @P1
  AND p.class IN (0, 1, 3)
  AND (p.class <> 1 OR p.minor_id = 0)
ORDER BY p.class, scope_name, object_name, p.permission_name
"#;

const INHERITED_PERMISSIONS: &str = r#"
WITH role_tree AS (
    SELECT drm.role_principal_id
    FROM sys.database_role_members AS drm
    JOIN sys.database_principals AS member
      ON member.principal_id = drm.member_principal_id
    WHERE member.name = @P1
    UNION ALL
    SELECT drm.role_principal_id
    FROM sys.database_role_members AS drm
    JOIN role_tree AS child ON child.role_principal_id = drm.member_principal_id
)
SELECT role.name AS source_name,
       p.state_desc,
       p.permission_name,
       p.class_desc,
       CASE WHEN p.class = 3 THEN SCHEMA_NAME(p.major_id)
            WHEN p.class = 1 THEN OBJECT_SCHEMA_NAME(p.major_id)
            ELSE DB_NAME() END AS scope_name,
       CASE WHEN p.class = 1 THEN OBJECT_NAME(p.major_id) ELSE NULL END AS object_name
FROM role_tree AS tree
JOIN sys.database_principals AS role ON role.principal_id = tree.role_principal_id
JOIN sys.database_permissions AS p ON p.grantee_principal_id = role.principal_id
WHERE p.class IN (0, 1, 3)
  AND (p.class <> 1 OR p.minor_id = 0)
ORDER BY role.name, p.class, scope_name, object_name, p.permission_name
OPTION (MAXRECURSION 32)
"#;

const ROLE_MEMBERSHIPS: &str = r#"
WITH role_tree AS (
    SELECT drm.role_principal_id
    FROM sys.database_role_members AS drm
    JOIN sys.database_principals AS member
      ON member.principal_id = drm.member_principal_id
    WHERE member.name = @P1
    UNION ALL
    SELECT drm.role_principal_id
    FROM sys.database_role_members AS drm
    JOIN role_tree AS child ON child.role_principal_id = drm.member_principal_id
)
SELECT DISTINCT role.name
FROM role_tree AS tree
JOIN sys.database_principals AS role ON role.principal_id = tree.role_principal_id
ORDER BY role.name
OPTION (MAXRECURSION 32)
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionScope {
    Database(String),
    Schema(String),
    Object { schema: String, object: String },
}

#[derive(Debug, Clone)]
struct Permission {
    source: String,
    state: String,
    name: String,
    scope: PermissionScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequestedScope {
    Database,
    Schema(String),
    Object { schema: String, object: String },
}

impl RequestedScope {
    fn from_wire(database: Option<&str>, table: Option<&str>) -> Result<Self, String> {
        match (database, table) {
            (None, None) => Ok(Self::Database),
            (Some(schema), None) if !schema.trim().is_empty() => {
                Ok(Self::Schema(schema.to_string()))
            }
            (Some(schema), Some(object))
                if !schema.trim().is_empty() && !object.trim().is_empty() =>
            {
                Ok(Self::Object {
                    schema: schema.to_string(),
                    object: object.to_string(),
                })
            }
            (None, Some(_)) => Err("An object scope requires a schema".to_string()),
            _ => Err("Schema and object names cannot be empty".to_string()),
        }
    }

    fn target_sql(&self, database_name: &str) -> String {
        match self {
            Self::Database => format!("DATABASE::{}", bracket_quote(database_name)),
            Self::Schema(schema) => format!("SCHEMA::{}", bracket_quote(schema)),
            Self::Object { schema, object } => format!(
                "OBJECT::{}.{}",
                bracket_quote(schema),
                bracket_quote(object)
            ),
        }
    }

    fn allows(&self, privilege: &str) -> bool {
        match self {
            Self::Database => {
                DATABASE_AND_SCHEMA_PRIVILEGES.contains(&privilege)
                    || DATABASE_ONLY_PRIVILEGES.contains(&privilege)
            }
            Self::Schema(_) => DATABASE_AND_SCHEMA_PRIVILEGES.contains(&privilege),
            Self::Object { .. } => OBJECT_PRIVILEGES.contains(&privilege),
        }
    }

    fn matches(&self, permission: &PermissionScope) -> bool {
        match (self, permission) {
            (Self::Database, PermissionScope::Database(_)) => true,
            (Self::Schema(requested), PermissionScope::Schema(actual)) => {
                requested.eq_ignore_ascii_case(actual)
            }
            (
                Self::Object {
                    schema: requested_schema,
                    object: requested_object,
                },
                PermissionScope::Object {
                    schema: actual_schema,
                    object: actual_object,
                },
            ) => {
                requested_schema.eq_ignore_ascii_case(actual_schema)
                    && requested_object.eq_ignore_ascii_case(actual_object)
            }
            _ => false,
        }
    }
}

pub fn privilege_catalog() -> DbPrivilegeCatalog {
    DbPrivilegeCatalog {
        // The frontend shows `database + global` for its top-level card and
        // `database` for its middle card. We use those as database and schema
        // respectively, so `global` contains database-only permissions.
        database: strings(DATABASE_AND_SCHEMA_PRIVILEGES),
        global: strings(DATABASE_ONLY_PRIVILEGES),
        table: strings(OBJECT_PRIVILEGES),
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn validate_account(user: &str, login: &str) -> Result<(), String> {
    if user.trim().is_empty() {
        return Err("Database user name cannot be empty".to_string());
    }
    if login.trim().is_empty() {
        return Err("SQL Server login name cannot be empty".to_string());
    }
    Ok(())
}

fn password_literal(password: &str) -> String {
    format!("N'{}'", password.replace('\'', "''"))
}

pub(crate) fn build_create_login_sql(login: &str, password: &str) -> String {
    format!(
        "CREATE LOGIN {} WITH PASSWORD = {}, CHECK_POLICY = ON, CHECK_EXPIRATION = OFF",
        bracket_quote(login),
        password_literal(password)
    )
}

pub(crate) fn build_create_user_sql(user: &str, login: &str) -> String {
    format!(
        "CREATE USER {} FOR LOGIN {}",
        bracket_quote(user),
        bracket_quote(login)
    )
}

pub(crate) fn build_drop_user_sql(user: &str) -> String {
    format!("DROP USER {}", bracket_quote(user))
}

pub(crate) fn build_drop_login_sql(login: &str) -> String {
    format!("DROP LOGIN {}", bracket_quote(login))
}

pub(crate) fn build_set_password_sql(login: &str, password: &str) -> String {
    format!(
        "ALTER LOGIN {} WITH PASSWORD = {}",
        bracket_quote(login),
        password_literal(password)
    )
}

fn redact_password(mut message: String, password: &str) -> String {
    if !password.is_empty() {
        message = message.replace(password, "[REDACTED]");
        let escaped = password.replace('\'', "''");
        if escaped != password {
            message = message.replace(&escaped, "[REDACTED]");
        }
    }
    message
}

async fn execute_transaction_batch(
    conn: &mut BridgeConnection,
    statements: &[String],
) -> Result<(), String> {
    // SQL BEGIN/COMMIT sent as a regular batch triggers SQL Server error 3981
    // with this preview client. Its TDS transaction-management API carries the
    // transaction descriptor correctly, so use that API around raw language
    // batches and explicitly roll back the first failed statement.
    let query_timeout_seconds = conn.query_timeout_seconds();
    let client = conn.inner_mut();
    client
        .close_query()
        .await
        .map_err(|error| error.to_string())?;
    client
        .begin_transaction(TransactionIsolationLevel::ReadCommitted, None)
        .await
        .map_err(|error| error.to_string())?;

    for statement in statements {
        let outcome = match client
            .execute(statement.clone(), query_timeout_seconds, None)
            .await
        {
            Ok(()) => client.close_query().await,
            Err(error) => Err(error),
        };
        if let Err(error) = outcome {
            let _ = client.close_query().await;
            let rollback = client.rollback_transaction(None, None).await;
            return Err(match rollback {
                Ok(()) => error.to_string(),
                Err(rollback_error) => {
                    format!("{error}; transaction rollback also failed: {rollback_error}")
                }
            });
        }
    }

    client
        .commit_transaction(None, None)
        .await
        .map_err(|error| error.to_string())
}

async fn query_bool(
    conn: &mut BridgeConnection,
    query: &str,
    values: &[&dyn mssql_tiberius_bridge::ToSql],
) -> Result<bool, String> {
    Ok(conn
        .query(query, values)
        .await
        .map_err(|error| error.to_string())?
        .into_first_result()
        .first()
        .and_then(|row| row.get::<bool, _>(0))
        .unwrap_or(false))
}

async fn ensure_account(
    conn: &mut BridgeConnection,
    user: &str,
    login: &str,
) -> Result<(), String> {
    validate_account(user, login)?;
    if query_bool(conn, ACCOUNT_EXISTS, &[&user, &login]).await? {
        Ok(())
    } else {
        Err(format!(
            "Database user {} is not mapped to SQL Server login {} in the current database",
            bracket_quote(user),
            bracket_quote(login)
        ))
    }
}

pub async fn get_users(conn: &mut BridgeConnection) -> Result<Vec<DbUserInfo>, String> {
    let rows = conn
        .simple_query(LIST_USERS)
        .await
        .map_err(|error| format!("Failed to list SQL Server database users: {error}"))?
        .into_first_result();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(DbUserInfo {
                user: row.get::<&str, _>("user_name")?.to_string(),
                host: row.get::<&str, _>("login_name")?.to_string(),
                locked: row.get::<bool, _>("is_locked").unwrap_or(false),
            })
        })
        .collect())
}

pub async fn create_user(
    conn: &mut BridgeConnection,
    user: &str,
    login: &str,
    password: &str,
) -> Result<(), String> {
    validate_account(user, login)?;
    if query_bool(conn, LOGIN_EXISTS, &[&login]).await? {
        return Err(format!(
            "SQL Server login {} already exists",
            bracket_quote(login)
        ));
    }
    if query_bool(conn, USER_EXISTS, &[&user]).await? {
        return Err(format!(
            "Database principal {} already exists in the current database",
            bracket_quote(user)
        ));
    }

    let create_login = build_create_login_sql(login, password);
    conn.simple_query(create_login)
        .await
        .map_err(|error| {
            redact_password(
                format!(
                    "Failed to create SQL Server login {}: {error}",
                    bracket_quote(login)
                ),
                password,
            )
        })?
        .into_results();

    let create_database_user = build_create_user_sql(user, login);
    if let Err(error) = conn.simple_query(create_database_user).await {
        let cleanup = conn.simple_query(build_drop_login_sql(login)).await;
        let cleanup_note = cleanup
            .err()
            .map(|cleanup_error| format!("; login cleanup also failed: {cleanup_error}"))
            .unwrap_or_default();
        return Err(redact_password(
            format!(
                "Failed to create database user {} for login {}: {error}{cleanup_note}",
                bracket_quote(user),
                bracket_quote(login)
            ),
            password,
        ));
    }
    Ok(())
}

pub async fn drop_user(conn: &mut BridgeConnection, user: &str, login: &str) -> Result<(), String> {
    ensure_account(conn, user, login).await?;
    conn.simple_query(build_drop_user_sql(user)).await
        .map_err(|error| {
            format!(
                "Failed to drop database user {}. SQL Server may be protecting a schema or object owned by this user: {error}",
                bracket_quote(user)
            )
        })?
        .into_results();
    conn.simple_query(build_drop_login_sql(login)).await
        .map_err(|error| {
            format!(
                "Database user {} was dropped, but its SQL Server login {} could not be dropped: {error}",
                bracket_quote(user),
                bracket_quote(login)
            )
        })?
        .into_results();
    Ok(())
}

pub async fn set_password(
    conn: &mut BridgeConnection,
    user: &str,
    login: &str,
    password: &str,
) -> Result<(), String> {
    ensure_account(conn, user, login).await?;
    let sql = build_set_password_sql(login, password);
    conn.simple_query(sql)
        .await
        .map_err(|error| {
            redact_password(
                format!(
                    "Failed to change password for SQL Server login {}: {error}",
                    bracket_quote(login)
                ),
                password,
            )
        })?
        .into_results();
    Ok(())
}

fn permission_from_row(row: &Row) -> Option<Permission> {
    let class = row.get::<&str, _>("class_desc")?;
    let scope_name = row.get::<&str, _>("scope_name").unwrap_or("");
    let scope = match class {
        "DATABASE" => PermissionScope::Database(scope_name.to_string()),
        "SCHEMA" => PermissionScope::Schema(scope_name.to_string()),
        "OBJECT_OR_COLUMN" => PermissionScope::Object {
            schema: scope_name.to_string(),
            object: row.get::<&str, _>("object_name")?.to_string(),
        },
        _ => return None,
    };
    Some(Permission {
        source: row.get::<&str, _>("source_name")?.to_string(),
        state: row.get::<&str, _>("state_desc")?.to_string(),
        name: row.get::<&str, _>("permission_name")?.to_string(),
        scope,
    })
}

async fn permissions(
    conn: &mut BridgeConnection,
    user: &str,
    inherited: bool,
) -> Result<Vec<Permission>, String> {
    let sql = if inherited {
        INHERITED_PERMISSIONS
    } else {
        DIRECT_PERMISSIONS
    };
    Ok(conn
        .query(sql, &[&user])
        .await
        .map_err(|error| format!("Failed to inspect SQL Server permissions: {error}"))?
        .into_first_result()
        .iter()
        .filter_map(permission_from_row)
        .collect())
}

fn scope_wire(scope: &PermissionScope) -> (Option<String>, Option<String>) {
    match scope {
        PermissionScope::Database(_) => (None, None),
        PermissionScope::Schema(schema) => (Some(schema.clone()), None),
        PermissionScope::Object { schema, object } => (Some(schema.clone()), Some(object.clone())),
    }
}

fn permission_sql(permission: &Permission, user: &str) -> String {
    let target = match &permission.scope {
        PermissionScope::Database(database) => {
            format!("DATABASE::{}", bracket_quote(database))
        }
        PermissionScope::Schema(schema) => {
            format!("SCHEMA::{}", bracket_quote(schema))
        }
        PermissionScope::Object { schema, object } => format!(
            "OBJECT::{}.{}",
            bracket_quote(schema),
            bracket_quote(object)
        ),
    };
    let verb = if permission.state == "DENY" {
        "DENY"
    } else {
        "GRANT"
    };
    let suffix = if permission.state == "GRANT_WITH_GRANT_OPTION" {
        " WITH GRANT OPTION"
    } else {
        ""
    };
    format!(
        "{verb} {} ON {target} TO {}{suffix}",
        permission.name,
        bracket_quote(user)
    )
}

pub async fn get_grants(
    conn: &mut BridgeConnection,
    user: &str,
    login: &str,
) -> Result<Vec<String>, String> {
    ensure_account(conn, user, login).await?;
    let direct = permissions(conn, user, false).await?;
    let inherited = permissions(conn, user, true).await?;
    let role_rows = conn
        .query(ROLE_MEMBERSHIPS, &[&user])
        .await
        .map_err(|error| format!("Failed to inspect SQL Server role memberships: {error}"))?
        .into_first_result();

    let mut lines = direct
        .iter()
        .map(|permission| permission_sql(permission, user))
        .collect::<Vec<_>>();
    lines.extend(role_rows.iter().filter_map(|row| {
        row.get::<&str, _>(0).map(|role| {
            format!(
                "ROLE MEMBERSHIP: ALTER ROLE {} ADD MEMBER {}",
                bracket_quote(role),
                bracket_quote(user)
            )
        })
    }));
    lines.extend(inherited.iter().map(|permission| {
        format!(
            "INHERITED VIA ROLE {}: {}",
            bracket_quote(&permission.source),
            permission_sql(permission, &permission.source)
        )
    }));
    Ok(lines)
}

pub async fn get_privileges(
    conn: &mut BridgeConnection,
    user: &str,
    login: &str,
) -> Result<Vec<DbUserGrantSet>, String> {
    ensure_account(conn, user, login).await?;
    let mut grouped: BTreeMap<(Option<String>, Option<String>), Vec<String>> = BTreeMap::new();
    for permission in permissions(conn, user, false).await? {
        // DENY and inherited role rights stay in the raw grants view. Showing
        // either as a checked direct grant would make the editor lie about
        // what a REVOKE can remove.
        if permission.state != "GRANT" && permission.state != "GRANT_WITH_GRANT_OPTION" {
            continue;
        }
        let names = grouped.entry(scope_wire(&permission.scope)).or_default();
        if !names.contains(&permission.name) {
            names.push(permission.name);
            names.sort();
        }
    }
    Ok(grouped
        .into_iter()
        .map(|((database, table), privileges)| DbUserGrantSet {
            database,
            table,
            privileges,
        })
        .collect())
}

fn canonical_privileges(
    scope: &RequestedScope,
    privileges: &[String],
) -> Result<Vec<String>, String> {
    if privileges.is_empty() {
        return Err("No privileges selected".to_string());
    }
    let mut canonical = Vec::with_capacity(privileges.len());
    for privilege in privileges {
        let name = privilege.trim().to_uppercase();
        if !scope.allows(name.as_str()) {
            return Err(format!(
                "Unsupported SQL Server privilege '{privilege}' for this scope"
            ));
        }
        if !canonical.contains(&name) {
            canonical.push(name);
        }
    }
    Ok(canonical)
}

pub(crate) fn build_permission_change_sql(
    database_name: &str,
    user: &str,
    database: Option<&str>,
    table: Option<&str>,
    privilege: &str,
    grant: bool,
) -> Result<String, String> {
    let scope = RequestedScope::from_wire(database, table)?;
    let privilege = canonical_privileges(&scope, &[privilege.to_string()])?
        .into_iter()
        .next()
        .expect("one validated privilege");
    let verb = if grant { "GRANT" } else { "REVOKE" };
    let preposition = if grant { "TO" } else { "FROM" };
    Ok(format!(
        "{verb} {privilege} ON {} {preposition} {}",
        scope.target_sql(database_name),
        bracket_quote(user)
    ))
}

#[allow(clippy::too_many_arguments)]
pub async fn apply_privileges(
    conn: &mut BridgeConnection,
    database_name: &str,
    user: &str,
    login: &str,
    database: Option<&str>,
    table: Option<&str>,
    privileges: &[String],
    grant: bool,
) -> Result<(), String> {
    ensure_account(conn, user, login).await?;
    let scope = RequestedScope::from_wire(database, table)?;
    let requested = canonical_privileges(&scope, privileges)?;
    let current = permissions(conn, user, false).await?;

    let mut statements = Vec::new();
    for privilege in requested {
        let states = current
            .iter()
            .filter(|permission| {
                scope.matches(&permission.scope) && permission.name.eq_ignore_ascii_case(&privilege)
            })
            .map(|permission| permission.state.as_str())
            .collect::<Vec<_>>();
        if states.contains(&"DENY") {
            return Err(format!(
                "Cannot manage denied permission '{privilege}': remove the SQL Server DENY explicitly before using Tabularis"
            ));
        }
        let already_granted = states
            .iter()
            .any(|state| matches!(*state, "GRANT" | "GRANT_WITH_GRANT_OPTION"));
        if already_granted == grant {
            continue;
        }
        statements.push(build_permission_change_sql(
            database_name,
            user,
            database,
            table,
            &privilege,
            grant,
        )?);
    }

    if statements.is_empty() {
        return Ok(());
    }
    execute_transaction_batch(conn, &statements)
        .await
        .map_err(|error| format!("Failed to apply SQL Server privilege diff: {error}"))
}

#[cfg(test)]
mod tests;
