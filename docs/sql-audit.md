# SQL construction and identifier audit

This audit covers the release-candidate `src/driver/` tree and was refreshed
during `SS-046`. It classifies every production `format!` call that emits SQL
or an SQL fragment. The original audit found one unsafe API boundary:
`build_insert_sql` accepted an already-rendered table target. It now accepts
`schema` and `table` separately and applies `qualify` itself. The directly
executed view, index, foreign-key, user, and login statements also have pure
builders so their quoting can be regression-tested without a database
connection. Later raw row-edit support is included below as an explicit SQL
expression boundary rather than being mistaken for an ordinary value.

## Classification rules

| Code | Class | Required handling |
| --- | --- | --- |
| I | Identifier | Always pass the original identifier through `bracket_quote` or `qualify`. Identifiers cannot be bound as TDS values. |
| L | Literal value | Bind with `@Pn` whenever the RPC and SQL grammar allow it. Otherwise use a dedicated escaping helper. |
| K | Fixed syntax | Hard-coded text, numeric values, generated parameter markers, or a keyword selected from a closed allowlist. |
| S | SQL source | An explicit SQL expression, definition, or query supplied through an SQL-editing API. It is not reclassified as an identifier or literal. |

`S` is intentionally separate from scalar values. Escaping an SQL definition
as a literal would change the operation rather than make it safer. The raw SQL
boundaries are listed below and are never used for identifier-shaped fields.

## `format!` call-site inventory

Line anchors identify the audited call, not an API stability promise.
Non-SQL formatting (errors, version labels, temporal/JSON rendering, BLOB wire
encoding, and tests) is excluded.

### `helpers.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `helpers.rs:62` | Multipart object reference | schema I via `bracket_quote`; object I via `bracket_quote` | Safe |
| `helpers.rs:74` | Affected-row `SELECT` | expression K from two private constants; result alias I/K constant | Safe |
| `helpers.rs:80` | DML plus row-count sentinel | sql S from query execution; sentinel K | Intentional SQL boundary |
| `helpers.rs:107` | `@Pn` marker | ordinal K integer | Safe |
| `helpers.rs:128` | `INSERT` | target I from `qualify`; columns I from `bracket_quote`; expressions are bound-marker K or explicitly marked S | Safe identifiers; intentional raw row-edit boundary |
| `helpers.rs:146` | Identity-insert batch | target I; insert may contain explicit S; row-count select K | Safe within documented raw boundary |
| `helpers.rs:162` | Plain insert plus row-count select | insert may contain explicit S; select K | Safe within documented raw boundary |
| `helpers.rs:274` | Primary-key predicate | column I via `bracket_quote`; ordinal K integer | Safe; values are bound |
| `helpers.rs:288` | Composite-key `DELETE` | table I via `qualify`; predicate internally built | Safe; values are bound |
| `helpers.rs:317` | Composite-key `UPDATE` | table and column I; value expression is bound-marker K or explicitly marked S | Safe within documented raw boundary |
| `helpers.rs:327` | Column definition head | column I via `bracket_quote`; data type S from the DDL editor/model | Intentional reviewed DDL source |
| `helpers.rs:337` | Column default | default S from the DDL editor/model | Intentional reviewed DDL source |
| `helpers.rs:387` | Paginated query | query S; optional order clause K; offset/fetch K integers | Intentional query boundary |

### `ddl/mod.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `ddl/mod.rs:31` | Multipart name passed to `sp_rename` | schema, table, old column I via `bracket_quote` | Safe intermediate |
| `ddl/mod.rs:37` | `sp_rename` invocation | old multipart name L and new name L via `escape_single_quoted`; object kind K | Safe; returned script has no parameter channel |
| `ddl/mod.rs:45` | `ALTER COLUMN` | table I via `qualify`; column I via `bracket_quote`; type S; nullability K | Intentional reviewed DDL source |
| `ddl/mod.rs:60` | Add default constraint | table I; generated constraint I; default S; column I | Intentional reviewed DDL source |
| `ddl/mod.rs:72` | Generated default-constraint name | prefix K; table and column I inputs | Safe intermediate; quoted at use |
| `ddl/mod.rs:80` | Truncated constraint name | head I input; hash K hexadecimal | Safe intermediate; quoted at use |
| `ddl/mod.rs:84` | Find and drop default constraint | object name L and column name L escaped; table I; discovered constraint I via server `QUOTENAME` | Safe; returned script has no parameter channel |
| `ddl/mod.rs:94` | Object name for `OBJECT_ID` | schema and table I via `bracket_quote`, then L via `escape_single_quoted` | Safe nested literal |
| `ddl/mod.rs:130` | Add foreign key | table, constraint, columns, referenced table I via quoting helpers | Safe |
| `ddl/mod.rs:139` | `ON DELETE` action | action K from four-value allowlist | Safe |
| `ddl/mod.rs:142` | `ON UPDATE` action | action K from four-value allowlist | Safe |

### `blob.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `blob.rs:56` | Size-limited BLOB lookup | column I via `bracket_quote`; table I via `qualify`; predicate internally built | Safe; size and keys are bound |
| `blob.rs:65` | Full BLOB lookup | column I via `bracket_quote`; table I via `qualify`; predicate internally built | Safe; keys are bound |

### `ops.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `ops.rs:117` | `CREATE VIEW` | view I via `qualify`; definition S from view editor | Intentional SQL-definition boundary |
| `ops.rs:125` | `ALTER VIEW` | view I via `qualify`; definition S from view editor | Intentional SQL-definition boundary |
| `ops.rs:129` | `DROP VIEW` | view I via `qualify` | Safe; executed directly |
| `ops.rs:451` | Insert expression marker | ordinal K from the bound-parameter count | Safe |
| `ops.rs:606` | Table primary-key clause | columns I, each already bracket-quoted | Safe |
| `ops.rs:610` | `CREATE TABLE` | table I via `qualify`; definitions internally built | Safe identifiers; reviewed type/default source |
| `ops.rs:622` | `ADD COLUMN` | table I via `qualify`; definition internally built | Safe identifiers; reviewed type/default source |
| `ops.rs:654` | `CREATE INDEX` | uniqueness K boolean; index, table, columns I via quoting helpers | Safe; returned for review |
| `ops.rs:678` | `DROP INDEX` | index I via `bracket_quote`; table I via `qualify` | Safe; executed directly |
| `ops.rs:704` | Drop foreign-key constraint | table I via `qualify`; constraint I via `bracket_quote` | Safe; executed directly |

### `routines/mod.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `routines/mod.rs:8` | Routine argument literal | value L via `escape_single_quoted` | Safe; builder returns SQL and cannot return bindings |
| `routines/mod.rs:28` | Table-valued function call | target I via `qualify`; values L or explicit S from arguments | Safe within documented raw boundary |
| `routines/mod.rs:30` | Scalar function call | target I via `qualify`; values L or explicit S from arguments | Safe within documented raw boundary |
| `routines/mod.rs:46` | Named argument marker | validated ASCII parameter name K | Safe |
| `routines/mod.rs:58` | Output variable | index K integer | Safe |
| `routines/mod.rs:59` | Output declaration | variable K; type S from server metadata; initial value L or explicit S | Safe within metadata/raw boundary |
| `routines/mod.rs:63` | Output assignment | binding and variable K built internally | Safe |
| `routines/mod.rs:64` | Output projection | variable K; alias I via `bracket_quote` | Safe |
| `routines/mod.rs:66` | Input assignment | binding K; value L or explicit S | Safe within documented raw boundary |
| `routines/mod.rs:71` | Procedure call without arguments | target I via `qualify` | Safe |
| `routines/mod.rs:73` | Procedure call with arguments | target I; assignments internally built | Safe within documented raw boundary |
| `routines/mod.rs:76` | Output `SELECT` | projections internally built | Safe |
| `routines/mod.rs:84` | Function template | schema I via `bracket_quote`; remainder K | Safe |
| `routines/mod.rs:88` | Procedure template | schema I via `bracket_quote`; remainder K | Safe |
| `routines/mod.rs:104` | Convert create definition to alter | definition S accepted only with the `CREATE` prefix | Intentional routine-editor boundary |
| `routines/mod.rs:115` | Drop routine | object kind K from function/procedure branch; routine I via `qualify` | Safe; executed directly |

`RoutineCallArg.is_raw` is the routine argument-value bypass. `false` renders a
Unicode string literal with embedded quotes doubled; `None` renders fixed
`NULL`. `true` preserves the value as an SQL expression so callers can enter
values such as `SYSDATETIME()` or `DEFAULT`. This method returns editable SQL
to the host and does not execute it. The hostile-value test proves that the
bypass occurs only when the wire flag is explicitly true.

### `triggers/mod.rs` and `explain.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `triggers/mod.rs:50` | `DROP TRIGGER` | trigger I via `qualify` | Safe; executed directly |
| `explain.rs:21` | Enable plan capture | option K from `SHOWPLAN_XML` or `STATISTICS XML` branch | Safe |
| `explain.rs:32` | Disable plan capture | option K from the same closed branch | Safe |

`create_trigger` accepts a complete trigger definition from the trigger SQL
editor and sends it unchanged. It has no `format!` site and is an intentional
SQL-definition boundary.

### `users.rs`

| Source | SQL produced | Interpolations | Result |
| --- | --- | --- | --- |
| `users.rs:211` | Database permission target | database I via `bracket_quote` | Safe |
| `users.rs:212` | Schema permission target | schema I via `bracket_quote` | Safe |
| `users.rs:213` | Object permission target | schema and object I via `bracket_quote` | Safe |
| `users.rs:282` | Login password literal | password L with apostrophes doubled | Safe grammar-required literal |
| `users.rs:286` | `CREATE LOGIN` | login I via `bracket_quote`; password escaped L; options K | Safe |
| `users.rs:294` | `CREATE USER` | user and login I via `bracket_quote` | Safe |
| `users.rs:302` | `DROP USER` | user I via `bracket_quote` | Safe; executed directly |
| `users.rs:306` | `DROP LOGIN` | login I via `bracket_quote` | Safe; executed directly |
| `users.rs:310` | `ALTER LOGIN` password | login I; password escaped L | Safe |
| `users.rs:572` | Displayed database permission target | database I via `bracket_quote` | Safe |
| `users.rs:575` | Displayed schema permission target | schema I via `bracket_quote` | Safe |
| `users.rs:577` | Displayed object permission target | schema and object I via `bracket_quote` | Safe |
| `users.rs:593` | Displayed grant/deny SQL | verb K from state; privilege K from server catalog; target built safely; user I; suffix K | Safe |
| `users.rs:620` | Displayed role-membership SQL | role and user I via `bracket_quote`; remainder K | Safe |
| `users.rs:628` | Inherited-grant display line | role I; permission SQL internally built | Safe, display only |
| `users.rs:704` | Applied grant/revoke statement | verb and preposition K boolean branches; privilege K from scope allowlist; target safe; user I | Safe; executed transactionally |

SQL Server's `CREATE LOGIN` and `ALTER LOGIN` grammar requires the password in
the statement's `PASSWORD = 'password'` clause; it does not accept a TDS value
parameter in that grammar position. Those two statements therefore use the
small `password_literal` escape routine, and errors are redacted. Account
existence checks, user permission queries, and role queries bind user/login
literals with `@P1` and `@P2`.

## Parameter binding and static-query review

All ordinary scalar data uses TDS parameters:

- insert, update, delete, and BLOB key/value paths use
  `value_to_sql_param` and generated `@Pn` markers;
- BLOB preview size is bound as `@P1`;
- every interpolated metadata search in `introspection.rs` is instead a static
  query with bound parameters;
- account existence, permissions, roles, schemas, tables, routines, views,
  indexes, and trigger listing use bound parameters.

Escaping remains only where binding is impossible or there is no binding
channel: generated DDL scripts returned to the host, routine-call SQL returned
to the host, and login password DDL. Identifiers are never treated as values;
SQL Server does not permit parameter markers in identifier positions.

Complete SQL text is accepted only by APIs whose purpose is editing or running
SQL: `execute_query`, batch execution, view definitions, trigger definitions,
routine edit scripts, startup scripts, EXPLAIN's input query, column type and
default expressions, routine arguments explicitly marked `is_raw`, and row
edits explicitly shaped as `{ "value": "<SQL expression>", "is_raw": true }`.
These boundaries do not weaken identifier handling elsewhere.

## Regression coverage

`src/driver/sql_audit_tests.rs` passes identifiers containing `]`, embedded
double and single quotes, Unicode, leading digits, and the reserved word
`order` through the pure CRUD, DDL, view, routine, trigger, drop, and account
statement builders. `tests/live_db.rs` creates the table literally named
`[weird"name]]`, with columns `order` and `9Δ"value]`, then creates, inserts,
updates, selects, deletes, and drops it through the JSON-RPC boundary against
SQL Server 2022.
