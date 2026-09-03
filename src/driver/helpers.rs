//! Pure SQL Server identifier / literal helpers and parameter-binding adapters.
//!
//! The string utilities are deliberately kept free of any client or async
//! dependency so they can be unit-tested trivially and reused by multiple
//! modules (introspection, DDL, explain).

use crate::models::ColumnDefinition;

/// Wrap an identifier in square brackets — the SQL Server convention that is
/// safest for reserved words and for identifiers containing spaces, dots, or
/// hyphens. A closing bracket inside the identifier is escaped by doubling.
///
/// Reference: <https://learn.microsoft.com/en-us/sql/relational-databases/databases/database-identifiers>
///
/// ```text
/// bracket_quote("dbo")        -> "[dbo]"
/// bracket_quote("my table")   -> "[my table]"
/// bracket_quote("weird]name") -> "[weird]]name]"
/// ```
pub fn bracket_quote(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('[');
    for ch in name.chars() {
        if ch == ']' {
            out.push_str("]]");
        } else {
            out.push(ch);
        }
    }
    out.push(']');
    out
}

/// ANSI-style double-quoted identifier (requires `SET QUOTED_IDENTIFIER ON`,
/// which is the SQL Server default). A double-quote inside the identifier is
/// escaped by doubling. Prefer [`bracket_quote`] for DDL; this is for cases
/// where we echo back the driver-wide `identifier_quote` from the manifest.
#[allow(dead_code)]
pub fn quote_identifier(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

/// Produce a `[schema].[object]` reference. When `schema` is `None` or empty,
/// falls back to `[dbo]` (the SQL Server default schema).
pub fn qualify(schema: Option<&str>, object: &str) -> String {
    let schema = schema.unwrap_or("dbo");
    let schema = if schema.trim().is_empty() {
        "dbo"
    } else {
        schema
    };
    format!("{}.{}", bracket_quote(schema), bracket_quote(object))
}

/// Result-set column used to carry `@@ROWCOUNT` back to the driver for DML
/// statements. The TDS client reports rows *returned*, not rows *affected*,
/// so every DML batch captures the count itself and selects it as the final
/// result set under this name.
pub const AFFECTED_ROWS_COLUMN: &str = "__tabularis_affected_rows";

/// The trailing `SELECT` that surfaces `@@ROWCOUNT` (or a variable holding
/// it) as the batch's final single-cell result set.
fn select_affected_rows(expression: &str) -> String {
    format!("SELECT CAST({expression} AS BIGINT) AS [{AFFECTED_ROWS_COLUMN}];")
}

/// Append the `@@ROWCOUNT` capture to a single-statement DML so the affected
/// count comes back as a final result set (see [`AFFECTED_ROWS_COLUMN`]).
pub fn wrap_dml_with_rowcount(sql: &str) -> String {
    format!("{sql}\n; {}", select_affected_rows("@@ROWCOUNT"))
}

/// Build a parameterized SQL Server `INSERT` statement.
///
/// `schema`, `table`, and every entry in `columns` are identifiers and are
/// bracket-quoted here. Values are bound to `@P1, @P2, ...` by the caller in
/// the same order; this helper never accepts a pre-rendered table reference.
///
/// When `wrap_identity_insert` is true, the resulting batch toggles
/// `SET IDENTITY_INSERT` around the insert and is wrapped in `BEGIN TRY /
/// BEGIN CATCH` so the session-scoped setting is always cleared, even if the
/// insert fails.
///
/// The batch always ends by selecting the insert's `@@ROWCOUNT` as
/// [`AFFECTED_ROWS_COLUMN`]. In the identity-wrapped variant the count is
/// captured into a variable right after the `INSERT` — `SET IDENTITY_INSERT`
/// resets `@@ROWCOUNT`, so it cannot be read at the end of the batch.
///
/// Returns the SQL batch. The number of placeholders always matches
/// `columns.len()`.
pub fn build_insert_sql(
    schema: Option<&str>,
    table: &str,
    columns: &[String],
    wrap_identity_insert: bool,
) -> String {
    let expressions: Vec<String> = (1..=columns.len()).map(|i| format!("@P{i}")).collect();
    build_insert_sql_with_expressions(schema, table, columns, &expressions, wrap_identity_insert)
}

/// Build an INSERT whose value expressions have already been classified.
/// Most entries are positional parameters; an explicitly marked `is_raw`
/// edit may supply a SQL expression instead.
pub fn build_insert_sql_with_expressions(
    schema: Option<&str>,
    table: &str,
    columns: &[String],
    expressions: &[String],
    wrap_identity_insert: bool,
) -> String {
    debug_assert_eq!(columns.len(), expressions.len());
    let target = qualify(schema, table);
    let col_list = columns
        .iter()
        .map(|c| bracket_quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let insert = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        target,
        col_list,
        expressions.join(", ")
    );

    if wrap_identity_insert {
        // SET IDENTITY_INSERT is session-scoped and is *not* transactional,
        // so the CATCH block must explicitly turn it OFF before re-raising.
        // Setting OFF on a table that is already OFF is a no-op in SQL
        // Server, so this is safe even if the failure occurs before the ON
        // statement executes. The success and CATCH paths both turn it
        // OFF; SS-003 verifies a failed insert does not poison the reused
        // pooled session. No explicit transaction is needed — a single
        // INSERT is atomic on its own, and the TDS client rejects
        // BEGIN TRAN / COMMIT inside an `sp_executesql` RPC batch
        // (error 3981).
        format!(
            "DECLARE @tabularis_affected BIGINT = 0;\n\
             BEGIN TRY\n\
                 SET IDENTITY_INSERT {target} ON;\n\
                 {insert};\n\
                 SET @tabularis_affected = @@ROWCOUNT;\n\
                 SET IDENTITY_INSERT {target} OFF;\n\
             END TRY\n\
             BEGIN CATCH\n\
                 SET IDENTITY_INSERT {target} OFF;\n\
                 THROW;\n\
             END CATCH;\n\
             {select}",
            select = select_affected_rows("@tabularis_affected"),
        )
    } else {
        format!("{};\n{}", insert, select_affected_rows("@@ROWCOUNT"))
    }
}

/// Escape a single-quoted string literal by doubling embedded single quotes.
/// **Do not use this for parameterised values** — prefer positional parameter
/// binding (`@P1` / `conn.query(sql, &[&value])`). This helper is only for
/// metadata queries where the value is also the searchable key (e.g. when
/// embedding a schema name into a diagnostic comment).
pub fn escape_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

/// Map a [`serde_json::Value`] to a SQL parameter with the corresponding
/// SQL Server type instead of coercing every value through a string.
///
/// This helper dispatches on the JSON variant and hands the client a
/// natively-typed primitive, leaning on its existing
/// `ToSql for bool / i64 / f64 / String / Option<T>` implementations:
///
/// * `Null`            → `Option::<String>::None` → typed SQL NULL
/// * `Bool`            → `bool`  → `bit`
/// * `Number` (int)    → `i64`   → `bigint` (server widens as needed)
/// * `Number` (float)  → `f64`   → `float(53)`
/// * `String`          → `String` → `nvarchar(4000)`
/// * `Array` / `Object` → stringified JSON, bound as `nvarchar(4000)`
///
/// Returning a `Box<dyn ToSql>` keeps the lifetime story simple at the call
/// site: the caller collects owned boxes once, then borrows from them when
/// building the `&[&dyn ToSql]` slice required by `Client::execute` /
/// `Client::query`.
pub fn value_to_sql_param(
    value: &serde_json::Value,
) -> Result<Box<dyn mssql_tiberius_bridge::ToSql>, String> {
    match value {
        serde_json::Value::Null => Ok(Box::new(None::<String>)),
        serde_json::Value::Bool(value) => Ok(Box::new(*value)),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(Box::new(value))
            } else if let Some(value) = number.as_u64() {
                let value = i64::try_from(value)
                    .map_err(|_| format!("SQL Server integer exceeds BIGINT range: {number}"))?;
                Ok(Box::new(value))
            } else {
                number
                    .as_f64()
                    .map(|value| Box::new(value) as Box<dyn mssql_tiberius_bridge::ToSql>)
                    .ok_or_else(|| format!("Invalid SQL Server numeric value: {number}"))
            }
        }
        serde_json::Value::String(value) => match crate::driver::blob::decode_blob_wire(value)? {
            Some(bytes) => Ok(Box::new(bytes)),
            None => Ok(Box::new(value.clone())),
        },
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Ok(Box::new(value.to_string()))
        }
    }
}

/// Return the SQL expression from the explicit row-edit raw-value shape.
/// Ordinary JSON objects remain bindable JSON values; only `is_raw: true`
/// opts into expression insertion.
pub fn raw_sql_expression(value: &serde_json::Value) -> Result<Option<&str>, String> {
    // An untyped SQL NULL is assignable to every nullable SQL Server type;
    // the bridge's fallback NVARCHAR NULL parameter is not (notably binary
    // and CLR UDT columns reject that implicit conversion).
    if value.is_null() {
        return Ok(Some("NULL"));
    }
    let serde_json::Value::Object(object) = value else {
        return Ok(None);
    };
    if object.get("is_raw") != Some(&serde_json::Value::Bool(true)) {
        return Ok(None);
    }
    let expression = object
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|expression| !expression.is_empty())
        .ok_or_else(|| {
            "SQL Server raw row-edit values require a non-empty string 'value'".to_string()
        })?;
    Ok(Some(expression))
}

/// Build a parameterised `WHERE` clause for a composite primary key.
///
/// `pk_cols` are bracket-quoted; each column is bound to an ordinal marker
/// starting at `@P{start_marker}`. The caller passes the matching values to
/// `.query()` in the same order, ensuring `@Pn` lines up positionally.
///
/// Returns `None` when `pk_cols` is empty — callers must treat this as a
/// programmer error (no PK to identify a row by).
///
/// ```text
/// build_pk_where_clause(&["id".into()], 1)
///   -> Some("[id] = @P1")
/// build_pk_where_clause(&["tenant_id".into(), "user_id".into()], 1)
///   -> Some("[tenant_id] = @P1 AND [user_id] = @P2")
/// build_pk_where_clause(&["a".into(), "b".into()], 2)
///   -> Some("[a] = @P2 AND [b] = @P3")
/// ```
pub fn build_pk_where_clause(pk_cols: &[String], start_marker: usize) -> Option<String> {
    if pk_cols.is_empty() {
        return None;
    }
    let parts: Vec<String> = pk_cols
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{} = @P{}", bracket_quote(col), start_marker + i))
        .collect();
    Some(parts.join(" AND "))
}

/// Build a parameterised `DELETE` statement targeting a composite primary key.
///
/// Returns `None` when `pk_cols` is empty.
pub fn build_delete_composite_sql(
    schema: Option<&str>,
    table: &str,
    pk_cols: &[String],
) -> Option<String> {
    let where_clause = build_pk_where_clause(pk_cols, 1)?;
    Some(format!(
        "DELETE FROM {} WHERE {}",
        qualify(schema, table),
        where_clause
    ))
}

/// Build a parameterised `UPDATE` statement that sets `col_name` to `@P1`
/// and matches rows by a composite primary key bound to `@P2`..`@P{n+1}`.
///
/// Returns `None` when `pk_cols` is empty.
pub fn build_update_composite_sql(
    schema: Option<&str>,
    table: &str,
    col_name: &str,
    pk_cols: &[String],
) -> Option<String> {
    build_update_composite_sql_with_expression(schema, table, col_name, "@P1", pk_cols, 2)
}

pub fn build_update_composite_sql_with_expression(
    schema: Option<&str>,
    table: &str,
    col_name: &str,
    value_expression: &str,
    pk_cols: &[String],
    first_pk_marker: usize,
) -> Option<String> {
    let where_clause = build_pk_where_clause(pk_cols, first_pk_marker)?;
    Some(format!(
        "UPDATE {} SET {} = {} WHERE {}",
        qualify(schema, table),
        bracket_quote(col_name),
        value_expression,
        where_clause
    ))
}

pub fn render_column_definition(column: &ColumnDefinition, inline_primary_key: bool) -> String {
    let mut definition = format!("{} {}", bracket_quote(&column.name), column.data_type);
    if column.is_auto_increment {
        definition.push_str(" IDENTITY(1,1)");
    }
    definition.push_str(if column.is_nullable {
        " NULL"
    } else {
        " NOT NULL"
    });
    if let Some(default) = &column.default_value {
        definition.push_str(&format!(" DEFAULT {default}"));
    }
    if inline_primary_key && column.is_pk {
        definition.push_str(" PRIMARY KEY");
    }
    definition
}

pub fn query_returns_result_set(query: &str) -> bool {
    top_level_statements(&code_mask(query))
        .iter()
        .any(|statement| statement_returns_result_set(statement))
}

pub fn query_can_be_paginated(query: &str) -> bool {
    let statements = top_level_statements(&code_mask(query));
    statements.len() == 1 && statement_can_be_paginated(&statements[0])
}

/// Whether `@@ROWCOUNT` is meaningful for the final statement in a batch.
/// DDL must not receive an appended sentinel: SQL Server requires module
/// creation statements to own their batch.
pub fn query_reports_affected_rows(query: &str) -> bool {
    top_level_statements(&code_mask(query))
        .last()
        .map(|statement| {
            let words = top_level_words(statement);
            statement_operation(&words).is_some_and(|(operation_index, operation)| {
                matches!(operation, "INSERT" | "UPDATE" | "DELETE" | "MERGE")
                    || (operation == "SELECT" && select_has_top_level_into(&words, operation_index))
            })
        })
        .unwrap_or(false)
}

/// Apply SQL Server `OFFSET … FETCH` pagination, requesting one extra row so
/// callers can determine whether another page exists. SQL Server requires an
/// `ORDER BY`; for an unordered query the synthetic order keeps the host's
/// pagination contract available but cannot make page boundaries stable.
pub fn build_paginated_query(query: &str, page_size: u32, page: u32) -> String {
    let normalized = query.trim().trim_end_matches(';').trim_end();
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let fetch = page_size.saturating_add(1);
    let has_order_by = contains_top_level_order_by(normalized);
    let order_by = if has_order_by {
        ""
    } else {
        " ORDER BY (SELECT NULL)"
    };

    format!("{normalized}{order_by} OFFSET {offset} ROWS FETCH NEXT {fetch} ROWS ONLY")
}

fn statement_can_be_paginated(statement: &str) -> bool {
    let words = top_level_words(statement);
    statement_operation(&words).is_some_and(|(operation_index, operation)| match operation {
        "SELECT" => !select_has_top_level_into(&words, operation_index),
        "VALUES" => true,
        _ => false,
    })
}

fn statement_returns_result_set(statement: &str) -> bool {
    let words = top_level_words(statement);
    let Some((operation_index, operation)) = statement_operation(&words) else {
        return false;
    };
    let dml_returns_rows = words[operation_index + 1..]
        .iter()
        .any(|word| word == "OUTPUT");

    match operation {
        "SELECT" => !select_has_top_level_into(&words, operation_index),
        "VALUES" | "EXEC" | "EXECUTE" => true,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" => dml_returns_rows,
        _ => crate::common::returns_result_set(statement),
    }
}

fn statement_operation(words: &[String]) -> Option<(usize, &str)> {
    let first = words.first()?;
    if first != "WITH" {
        return Some((0, first.as_str()));
    }

    words.iter().enumerate().skip(1).find_map(|(index, word)| {
        matches!(
            word.as_str(),
            "SELECT" | "VALUES" | "EXEC" | "EXECUTE" | "INSERT" | "UPDATE" | "DELETE" | "MERGE"
        )
        .then_some((index, word.as_str()))
    })
}

fn select_has_top_level_into(words: &[String], operation_index: usize) -> bool {
    words[operation_index + 1..]
        .iter()
        .any(|word| word == "INTO")
}

fn top_level_words(statement: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;
    for character in statement.chars().chain(std::iter::once(' ')) {
        match character {
            '(' => {
                if depth == 0 && !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                depth = depth.saturating_add(1);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.clear();
            }
            _ if depth == 0 && (character.is_alphanumeric() || character == '_') => {
                current.push(character.to_ascii_uppercase());
            }
            _ if depth == 0 && !current.is_empty() => {
                words.push(std::mem::take(&mut current));
            }
            _ => {}
        }
    }
    words
}

fn top_level_statements(masked: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;
    for character in masked.chars() {
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                if !current.trim().is_empty() {
                    statements.push(current.trim().to_string());
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        statements.push(current.trim().to_string());
    }
    statements
}

fn code_mask(query: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Normal,
        SingleQuote,
        DoubleQuote,
        Bracket,
        LineComment,
        BlockComment,
    }

    let characters: Vec<char> = query.chars().collect();
    let mut masked = String::with_capacity(query.len());
    let mut state = State::Normal;
    let mut position = 0;
    while position < characters.len() {
        let character = characters[position];
        let next = characters.get(position + 1).copied();
        match state {
            State::Normal => match (character, next) {
                ('\'', _) => {
                    state = State::SingleQuote;
                    masked.push(' ');
                }
                ('"', _) => {
                    state = State::DoubleQuote;
                    masked.push(' ');
                }
                ('[', _) => {
                    state = State::Bracket;
                    masked.push(' ');
                }
                ('-', Some('-')) => {
                    state = State::LineComment;
                    masked.push_str("  ");
                    position += 1;
                }
                ('/', Some('*')) => {
                    state = State::BlockComment;
                    masked.push_str("  ");
                    position += 1;
                }
                _ => masked.push(character),
            },
            State::SingleQuote if character == '\'' => {
                masked.push(' ');
                if next == Some('\'') {
                    masked.push(' ');
                    position += 1;
                } else {
                    state = State::Normal;
                }
            }
            State::DoubleQuote if character == '"' => {
                masked.push(' ');
                if next == Some('"') {
                    masked.push(' ');
                    position += 1;
                } else {
                    state = State::Normal;
                }
            }
            State::Bracket if character == ']' => {
                masked.push(' ');
                if next == Some(']') {
                    masked.push(' ');
                    position += 1;
                } else {
                    state = State::Normal;
                }
            }
            State::LineComment if matches!(character, '\n' | '\r') => {
                masked.push(character);
                state = State::Normal;
            }
            State::BlockComment if character == '*' && next == Some('/') => {
                masked.push_str("  ");
                state = State::Normal;
                position += 1;
            }
            _ => masked.push(' '),
        }
        position += 1;
    }
    masked
}

fn contains_top_level_order_by(query: &str) -> bool {
    let words = top_level_words(&code_mask(query));
    words
        .windows(2)
        .any(|pair| pair[0] == "ORDER" && pair[1] == "BY")
}

#[cfg(test)]
mod tests;
