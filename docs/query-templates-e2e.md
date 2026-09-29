# Table query templates: end-to-end verification

This checklist verifies the joint fix for [issue #26](https://github.com/TabularisDB/tabularis-sqlserver-plugin/issues/26):
SQL Server-native Generate SQL previews and preservation of explicit row limits.
It requires the plugin changes from PRs #30/#31 and the host extension from
[Tabularis PR #818](https://github.com/TabularisDB/tabularis/pull/818).

## Automated checks

From the plugin checkout:

```bash
cargo test --locked --bins --test conformance
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
just test-explain
```

Start a local SQL Server with `just run-sqlserver`, or use an existing disposable
instance. Run live tests against a dedicated database, not production:

```bash
export SQLSERVER_TEST_HOST=127.0.0.1
export SQLSERVER_TEST_PORT=1433
export SQLSERVER_TEST_USER=sa
# Set SQLSERVER_TEST_PASSWORD to your local container password if not the default.
export SQLSERVER_TEST_DATABASE=tabularis_pr31_test
cargo test --locked --test live_db -- --test-threads=1
```

The suite creates the database and its own scratch schema. Coverage includes:

- SELECT previews executed with conflicting host pagination parameters;
- explicit outer TOP, TOP PERCENT, WITH TIES and OFFSET/FETCH, in queries and batches;
- normal pagination for unbounded SELECT statements;
- escaped table/column identifiers containing `]`;
- unique UPDATE placeholders for similarly named columns;
- preview generation without writes, and zero affected rows when executing guarded
  UPDATE/DELETE statements after filling their parameters.

In the matching Tabularis checkout, run frontend tests and compile **all** Rust
test targets so optional-capability additions also check integration fixtures:

```bash
pnpm exec vitest run
pnpm build
cd src-tauri
cargo test --locked --all-targets --no-run
cargo test --locked --lib plugins::
```

## Prepare the desktop fixture

1. Build and install the plugin with `just dev-install`, then restart Tabularis.
   If Cargo uses a global target directory, use `CARGO_TARGET_DIR=target just
   dev-install`: the recipe expects the binary under `target/debug`.
2. Run a host containing PR #818, for example `pnpm tauri dev` from the matching
   Tabularis checkout. A browser-only `pnpm dev` does not exercise Tauri commands.
3. Enable SQL Server in Settings → Plugins.
4. Create a SQL-authenticated connection to the dedicated `tabularis_pr31_test`
   database. For a local container with a self-signed certificate, use SSL mode
   `require`; use certificate verification for real servers.
5. Execute `tests/fixtures/query_templates_e2e.sql` in that database, via sqlcmd
   or the editor. It creates `pr31.orders` (150 rows) and `pr31.order]details`
   (2 rows), without dropping or resetting existing tables. Refresh the explorer.

Debug hosts may show a runtime-floor warning when their development version
predates the plugin minimum. Do not lower the packaged manifest's runtime floor
just to suppress this warning; use a compatible host for release verification.

## SELECT previews and pagination

Right-click `pr31.orders` → **Generate SQL**:

- **SELECT \*** must show `SELECT * FROM [pr31].[orders];` (line breaks may vary).
  It is unbounded SQL; normal host pagination still applies when executed.
- **SELECT [fields]** must start with `SELECT TOP (100)`, bracket-quote every
  column, and target `[pr31].[orders]`. There must be **no LIMIT clause**.
- **Run in console** must open an editor on this connection without automatically
  executing the statement.
- Set the editor page size to **50** and explicitly execute the generated TOP
  statement. Expect **100 rows**, not 50, and no TOP/OFFSET syntax error.
- Execute the unbounded SELECT instead: paging should still work through the 150
  rows. Add `ORDER BY [id]` for deterministic page contents.

The CREATE TABLE tab is deliberately unchanged by the template extension.

## Guarded UPDATE and DELETE

- UPDATE must target `[pr31].[orders]`, use four distinct `:value_1` … `:value_4`
  placeholders, and end with `WHERE 1 = 0;`. `[a b]` and `[a-b]` must not share
  a placeholder.
- Open it in the editor and fill the parameter dialog with SQL literals `999`,
  `N'closed'`, `20`, `30`. Values are substituted verbatim; quote string values.
- **Keep `WHERE 1 = 0` unchanged.** Execute: expect **0 affected rows**.
- DELETE must also end with `WHERE 1 = 0;`. Execute unchanged: expect **0 affected
  rows**.
- Verify the fixture remains unchanged:

```sql
SELECT COUNT(*) AS rows_remaining,
       SUM([a b]) AS sum_ab,
       SUM([a-b]) AS sum_dash
FROM [pr31].[orders];
-- Expected: 150, 113250, 1132500
```

The fixtures deliberately avoid IDENTITY columns. The template contract accepts
column names, not metadata for excluding read-only columns; remove IDENTITY or
computed assignments before executing UPDATE previews on other tables.

## Escaping and alternate entry points

Generate SELECT fields on `pr31.order]details`. Expect
`FROM [pr31].[order]]details]` and `[order]]status]`; execution returns 2 rows.

With page size 50, explicitly execute each statement separately:

```sql
SELECT TOP (3) [id] FROM [pr31].[orders] ORDER BY [id];
-- 1, 2, 3

SELECT [id] FROM [pr31].[orders]
ORDER BY [id] OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY;
-- 3, 4, 5
```

Also open Generate SQL through the Command Palette with split editors or multiple
connections. The selected table's connection and schema must be retained.
A legacy/non-opted-in driver, such as built-in SQLite, must still open Generate
SQL normally. Automated host tests cover missing-method fallback and propagation
of real RPC errors; failures must not silently substitute another SQL dialect.

## Registry rollout

Local installation bypasses the registry. Register the optional capability in
Tabularium before publishing, as described in [query-templates.md](query-templates.md),
and keep the release's documented host minimum aligned with its manifest.
