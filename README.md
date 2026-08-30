<div align="center">
  <img src="https://raw.githubusercontent.com/TabularisDB/tabularis/main/public/logo-sm.png" width="120" height="120" alt="Tabularis logo" />
  <img src="https://raw.githubusercontent.com/TabularisDB/tabularis-sqlserver-plugin/main/sqlserver-icon.svg" width="120" height="120" alt="SQL Server plugin icon" />
</div>

# tabularis-sqlserver-plugin

<p align="center">

![Release](https://img.shields.io/github/release/TabularisDB/tabularis-sqlserver-plugin.svg?style=flat)
![Downloads](https://img.shields.io/github/downloads/TabularisDB/tabularis-sqlserver-plugin/total.svg?style=flat)
![Build & Release](https://github.com/tabularisDB/tabularis-sqlserver-plugin/workflows/Release/badge.svg)
[![Discord](https://img.shields.io/discord/1502944695808950282?color=5865F2&logo=discord&logoColor=white)](https://discord.com/invite/K2hmhfHRSt)

</p>

A [Microsoft SQL Server](https://www.microsoft.com/sql-server) plugin for [Tabularis](https://github.com/TabularisDB/tabularis), the lightweight database management tool.

This plugin enables Tabularis to connect to SQL Server instances, providing schema introspection, query execution, full CRUD, DDL, trigger and stored-routine management, BLOB handling, database-user management, and visual execution plans through a JSON-RPC 2.0 over stdio interface. It is written in Rust on top of Microsoft's [`mssql-tds`](https://github.com/microsoft/mssql-rust) protocol implementation (via [`mssql-tiberius-bridge`](https://crates.io/crates/mssql-tiberius-bridge)) with [`deadpool`](https://crates.io/crates/deadpool) connection pooling.

> **Requires Tabularis v0.20.0 or later.** This plugin relies on the plugin
> runtime introduced in that release and will not load on earlier versions.

**Discord** — [Join our Discord server](https://discord.com/invite/K2hmhfHRSt) and chat with the maintainers.

## Table of Contents

- [Features](#features)
- [Screenshots](#screenshots)
- [Connection Configuration](#connection-configuration)
- [Plugin Settings](#plugin-settings)
- [Supported Data Types](#supported-data-types)
- [Database Users and Privileges](#database-users-and-privileges)
- [Installation](#installation)
- [How It Works](#how-it-works)
- [Supported Operations](#supported-operations)
- [Known Limitations](#known-limitations)
- [Building from Source](#building-from-source)
- [Development](#development)
- [Contributing](#contributing)
- [Changelog](#changelog)
- [Credits](#credits)
- [License](#license)

## Features

- Microsoft's `mssql-tds` protocol implementation through `mssql-tiberius-bridge`, with `deadpool` connection pooling, session reset (`sp_reset_connection`), startup scripts, and pool lifecycle handling
- Schema, table, column, PK/FK, index, view, routine, and trigger introspection
- Query execution with pagination, CTE/DML classification, multiple result sets, and session-preserving batches
- Accurate affected rows, including multi-statement DML and DML `OUTPUT`
- INSERT/UPDATE/DELETE with composite primary keys and safe `IDENTITY_INSERT` recovery
- Table/view/index/foreign-key DDL and safe `ALTER COLUMN` generation
- Trigger creation, editing, and removal
- SQL-authenticated database-user, login, role, and privilege management
- Procedure/function management, typed `OUT`/`INOUT` variables, and table-valued functions
- Static and runtime execution plans through `SHOWPLAN_XML` / `STATISTICS XML`, rendered in Tabularis's Visual EXPLAIN
- JavaScript-safe `BIGINT` extraction and broad SQL Server type handling
- Pre-built release targets for Linux x86_64, macOS x86_64 and Apple Silicon, and Windows x86_64

## Screenshots

<table>
<tr>
<td><img src="https://raw.githubusercontent.com/TabularisDB/tabularis-sqlserver-plugin/main/assets/screenshots/02-database-picker.png" alt="SQL Server listed in the database picker" width="400" /><br />SQL Server in the database picker</td>
<td><img src="https://raw.githubusercontent.com/TabularisDB/tabularis-sqlserver-plugin/main/assets/screenshots/03-connection-form.png" alt="SQL Server connection configuration form" width="400" /><br />Connection configuration</td>
</tr>
<tr>
<td><img src="https://raw.githubusercontent.com/TabularisDB/tabularis-sqlserver-plugin/main/assets/screenshots/06-schema-browser.png" alt="SQL Server schema browser with tables, views, routines, and triggers" width="400" /><br />Multi-schema browsing</td>
<td><img src="https://raw.githubusercontent.com/TabularisDB/tabularis-sqlserver-plugin/main/assets/screenshots/08-visual-explain.png" alt="Visual EXPLAIN graph of a SQL Server SHOWPLAN" width="400" /><br />Visual EXPLAIN for SHOWPLAN</td>
</tr>
</table>

## Connection Configuration

| Parameter | Default | Required | Description |
| --- | --- | --- | --- |
| `host` | `localhost` | Yes unless using `connection_string` | SQL Server hostname or IP address |
| `port` | `1433` | No | TDS port |
| `database` | — | Yes unless using `connection_string` | Database the pool connects to |
| `username` | `sa` | Yes unless using `connection_string` | SQL-authenticated login |
| `password` | — | If required by the server | Login password; redacted from connection errors |
| `ssl_mode` | `prefer` | No | `disable`, `prefer`, `require`, or `verify-full` |
| `ssl_ca` | — | No | Rejected; strict TLS uses the system trust store |
| `ssl_cert` / `ssl_key` | — | No | Rejected; client-certificate authentication is not supported |
| `connection_string` | — | No | `sqlserver://…` URL or ADO.NET/ODBC keyword syntax |
| `startup_script` | — | No | SQL run on every new pooled connection, such as session `SET` options |

### Connection strings

The connection string accepts either URL syntax:

```text
sqlserver://sa:p%40ssword@localhost:1433/master?Encrypt=true&TrustServerCertificate=true
```

or ADO.NET/ODBC keyword syntax. Keyword names are case-insensitive, common
aliases (`Data Source`, `Initial Catalog`, `UID`, and `PWD`) are accepted, and
braces preserve semicolons inside values:

```text
Server=tcp:localhost,1433;Database=master;User Id=sa;Password={p;assword};Encrypt=true;TrustServerCertificate=true;
```

A connection string may be combined with discrete fields. Values explicitly
present in the string are authoritative, while discrete fields fill only
fields the string omits. Repeating the same value is allowed; contradictory
values are rejected with an error that identifies the discrete and
connection-string values instead of silently choosing one. Password values
are redacted in contradiction errors.

`Encrypt=false` maps to `ssl_mode=disable`; encrypted connections with
`TrustServerCertificate=true` map to `require`; encrypted connections that
verify the certificate map to `verify-full`. Custom CA and client-certificate
keywords are rejected under the same limitations as their discrete-field
counterparts.

### TLS modes

The standard Tabularis `ssl_mode` values map onto the TDS encryption policy:

| Mode | Behaviour |
|------|-----------|
| `disable` | Encryption off |
| `prefer` (default) | Encrypted, server certificate accepted |
| `require` | Encryption required, server certificate accepted |
| `verify-full` | Encryption required, certificate and hostname verified against the **system trust store** |
| `verify-ca` | Rejected — use `verify-full` |

Custom CA files and client certificates are rejected explicitly; strict verification uses the system trust store.

## Plugin Settings

Tabularis sends these process-wide settings through `initialize` when the
plugin starts:

| Setting | Default | Effect |
|---------|---------|--------|
| `max_pool_size` | `10` | Maximum physical SQL Server sessions in each connection pool |
| `connect_timeout_seconds` | `15` | Maximum time to establish and authenticate a new session |
| `query_timeout_seconds` | `0` | Maximum query duration in seconds; `0` disables the timeout |
| `application_name` | `Tabularis` | TDS application name visible to DBAs in SQL Server session metadata |
| `trust_server_certificate` | `false` | Forces acceptance of a self-signed certificate without validation; use only for trusted development servers |
| `pool_idle_eviction_minutes` | `10` | Interval for removing pools with no checked-out sessions |

Malformed values produce a warning in the plugin log and fall back to the
default; unknown settings are ignored for forward compatibility. Settings are
snapshotted when a pool is created. Changing a setting takes effect on the next
connection after the plugin is restarted, not on live pooled sessions.

`trust_server_certificate` is an explicit escape hatch for self-signed
certificates in a verifying TLS mode. The `prefer` and `require` modes already
accept the server certificate as described above.

## Supported Data Types

All common SQL Server types are supported for column creation and value extraction, including exact/approximate numerics (`TINYINT` … `BIGINT`, `DECIMAL`, `MONEY`, `FLOAT`), strings (`CHAR`/`VARCHAR`/`NVARCHAR` incl. `MAX`, `TEXT`/`NTEXT`), binary (`BINARY`/`VARBINARY`/`IMAGE`), date/time (`DATE`, `TIME`, `DATETIME`, `DATETIME2`, `SMALLDATETIME`, `DATETIMEOFFSET`), `BIT`, `UNIQUEIDENTIFIER`, `XML`, `SQL_VARIANT`, `ROWVERSION`, `HIERARCHYID`, and spatial (`GEOGRAPHY`, `GEOMETRY`).

Generic DDL types emitted by Tabularis map to SQL Server-native spellings. In
particular, generic `TIMESTAMP` maps to `DATETIME2`; SQL Server's own
`TIMESTAMP` type remains a deprecated `ROWVERSION` synonym, not a date/time.

`BIGINT` values outside JavaScript's safe integer range are delivered as strings so they round-trip without precision loss.

### Binary export and preview

`BINARY`, `VARBINARY` including `VARBINARY(MAX)`, and legacy `IMAGE` values can
be exported as raw files or previewed with MIME detection. `NULL` returns a
clear error instead of creating an empty file. `ROWVERSION` and its deprecated
`TIMESTAMP` synonym are excluded because they are server-generated concurrency
tokens, not user BLOB data.

BLOB previews are bounded by `max_blob_size` (100 MiB when the host does not
provide a value). SQL Server checks `DATALENGTH` before returning the bytes; an
oversized value produces an error with the actual and configured sizes and can
still be exported directly to a file without passing through base64 or a
JSON-RPC response.

## Database Users and Privileges

For this plugin a **database user** means a database-scoped SQL user mapped to
a server-scoped SQL login. In Tabularis's account display, `user` is the
principal in the connected database and the host-shaped field after `@` is the
mapped login name; it is not a network host. Windows, Azure AD, certificate,
contained, orphaned, and login-less users are intentionally not listed or
managed. Creating an account creates the login first and then its mapped user;
dropping it drops the user first and then the login. SQL Server's own ownership
checks are preserved, so a user that owns a schema or object must have that
ownership transferred before it can be dropped.

The host protocol's three MySQL-named scope shapes map to SQL Server as follows:

| Host wire scope | SQL Server scope |
|-----------------|------------------|
| `database = null`, `table = null` | Connected database |
| `database = schema`, `table = null` | Schema |
| `database = schema`, `table = object` | Object |

The privilege catalog follows the same mapping: its `global` entries are the
extra database-only permissions, `database` entries are permissions shared by
database and schema scopes, and `table` entries are object permissions.
Tabularis computes a requested checkbox diff, and the plugin checks the current
direct permissions again before applying only the required `GRANT` or `REVOKE`
statements in a transaction.

The parsed checkbox view contains direct grants only. The raw grants view also
labels role memberships, permissions inherited through roles, grants with
grant option, and direct `DENY` entries, so inherited rights are never shown as
if they were direct grants. Because SQL Server `DENY` overrides `GRANT`, the
plugin refuses to alter a denied permission; remove that `DENY` explicitly in
SQL before managing the permission through Tabularis.

## Installation

### Automatic (via Tabularis)

Open **Settings → Plugins** in Tabularis and install *SQL Server* from the plugin registry.

### Manual Installation

1. Download the ZIP for your platform from the [releases page](https://github.com/TabularisDB/tabularis-sqlserver-plugin/releases).
2. Extract it into the Tabularis plugins directory:
   - **Linux:** `~/.local/share/tabularis/plugins/sqlserver/`
   - **macOS:** `~/Library/Application Support/com.debba.tabularis/plugins/sqlserver/`
   - **Windows:** `%APPDATA%\debba\tabularis\data\plugins\sqlserver\`
3. On Linux/macOS, make the binary executable: `chmod +x sqlserver-plugin`
4. Restart Tabularis — *SQL Server* appears in the connection picker.

## How It Works

The plugin is a standalone Rust binary that communicates with Tabularis through
**newline-delimited JSON-RPC 2.0 over stdio**:

1. Tabularis starts `sqlserver-plugin` as a child process and calls
   `initialize` with the manifest-backed process settings.
2. The plugin normalizes the discrete connection fields or connection string,
   then reuses a matching in-process `deadpool` pool.
3. New sessions connect through Microsoft's `mssql-tds` implementation, run
   the optional startup script, and are reset with `sp_reset_connection`
   before reuse.
4. Requests and responses stay on stdin and stdout; diagnostics go to stderr.
   The plugin opens no listening port and keeps no persistent state.

Pool identity includes every connection and TLS field that changes session
behaviour, plus the startup script. The idle-eviction task removes unused
pools at the configured interval, and the courtesy `shutdown` RPC drains all
remaining pools.

## Supported Operations

| Method group | Operations |
| --- | --- |
| Lifecycle and connection | `initialize`, `shutdown`, `ping`, `test_connection`, database discovery |
| Schema metadata | Schemas, tables, columns, keys, indexes, views, routines, triggers, snapshots, and batch metadata |
| Query execution | Paginated queries, session-preserving batches, affected rows, and Visual EXPLAIN |
| Row editing | Insert, update, and delete with composite primary keys and type-aware values |
| DDL | Table, column, index, foreign-key, view, routine, and trigger generation or lifecycle operations |
| BLOBs | Raw file export and bounded MIME-sniffed data-URL preview |
| Security | SQL login and mapped database-user lifecycle, password changes, privilege catalog, grants, roles, and inherited rights |

## Known Limitations

- SQL authentication only; Azure AD and Windows Integrated Authentication are follow-up work.
- Primary-key membership changes are disabled: the single-column alteration API cannot safely preserve composite PKs and referencing foreign keys.
- Custom CA files are rejected explicitly; strict verification uses the system trust store.
- SQL Server has indexed views, not materialized views. Indexed views are maintained synchronously and have no refresh operation, so `get_materialized_views`, `get_materialized_view_columns`, `get_materialized_view_definition`, and `refresh_materialized_view` deliberately return `-32601` rather than pretending the features are equivalent.
- All host RPC methods outside those four materialized-view operations are implemented, including the courtesy `shutdown` method even though the current host terminates the process directly. Truly unknown JSON-RPC methods return `-32601` with an error naming both the method and the SQL Server plugin.

## Building from Source

### Prerequisites

- Rust (stable, see `rust-toolchain.toml`)
- [`just`](https://github.com/casey/just) (optional, wraps the common cargo invocations)

### Build

```bash
just build      # debug build
just release    # release build (what the GitHub Actions workflow ships)
```

### Install Locally

```bash
just dev-install   # build + copy binary and manifest into the Tabularis plugins dir
just uninstall     # remove the installed plugin
```

## Development

### Testing the Plugin

Unit tests need no database:

```bash
just test
just lint
just fmt
```

You can drive the plugin directly over stdio:

```bash
echo '{"jsonrpc":"2.0","method":"get_create_table_sql","params":{"table_name":"users","schema":"dbo","columns":[{"name":"id","data_type":"INT","is_nullable":false,"is_pk":true,"is_auto_increment":true,"default_value":null}]},"id":1}' \
  | ./target/debug/sqlserver-plugin
```

or use the interactive REPL:

```bash
just repl
```

### Setting Up a Local SQL Server

```bash
just run-sqlserver    # SQL Server 2022 in Docker (sa / Str0ng!Passw0rd)
just seed-sqlserver   # create and seed the tabularis_test database
```

The live JSON-RPC integration suite uses the same container:

```bash
SQLSERVER_PLUGIN_BIN="$PWD/target/debug/sqlserver-plugin" \
SQLSERVER_TEST_HOST=127.0.0.1 \
SQLSERVER_TEST_PASSWORD='Str0ng!Passw0rd' \
cargo test --test live_db -- --test-threads=1
```

## Contributing

Pull-request titles must follow [Conventional Commits](https://www.conventionalcommits.org/):
`type: subject`, `type(scope): subject`, or `type!: subject` for a breaking
change. Add a `BREAKING CHANGE:` footer to the PR description when the title
cannot communicate the full impact.

Every PR must have exactly one `prerelease:alpha`, `prerelease:beta`,
`prerelease:rc`, or `prerelease:stable` label. CI uses the title and that label
to suggest the next version and release channel; there is no default channel,
so a missing or ambiguous label fails the version-suggestion check.

| PR title type | Version impact |
| --- | --- |
| `feat` | minor |
| `fix`, `refactor`, `perf` | patch |
| `docs`, `style`, `chore`, `test`, `ci`, `build` | none |
| any type with `!` or a `BREAKING CHANGE:` footer | major |

Before opening a PR, run:

```bash
just fmt
just lint
just test
npx markdownlint-cli "**/*.md"
```

## [Changelog](./CHANGELOG.md)

## Credits

The SQL Server driver implementation was contributed by [Fabio Malpezzi](https://github.com/FabioMalpezzi), originally developed as a built-in Tabularis driver and adapted here to the plugin architecture.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
