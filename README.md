<div align="center">
  <img src="https://raw.githubusercontent.com/debba/tabularis/main/public/logo-sm.png" width="120" height="120" />
</div>

# tabularis-sqlserver-plugin

<p align="center">

![](https://img.shields.io/github/release/tabularisDB/tabularis-sqlserver-plugin.svg?style=flat)
![](https://img.shields.io/github/downloads/tabularisDB/tabularis-sqlserver-plugin/total.svg?style=flat)
![Build & Release](https://github.com/tabularisDB/tabularis-sqlserver-plugin/workflows/Release/badge.svg)
[![Discord](https://img.shields.io/discord/1502944695808950282?color=5865F2&logo=discord&logoColor=white)](https://discord.com/invite/K2hmhfHRSt)

</p>

A [Microsoft SQL Server](https://www.microsoft.com/sql-server) plugin for [Tabularis](https://github.com/TabularisDB/tabularis), the lightweight database management tool.

This plugin enables Tabularis to connect to SQL Server instances, providing schema introspection, query execution, full CRUD, DDL, trigger and stored-routine management, and visual execution plans through a JSON-RPC 2.0 over stdio interface. It is written in Rust on top of Microsoft's [`mssql-tds`](https://github.com/microsoft/mssql-rust) protocol implementation (via [`mssql-tiberius-bridge`](https://crates.io/crates/mssql-tiberius-bridge)) with [`deadpool`](https://crates.io/crates/deadpool) connection pooling.

The client was swapped to Microsoft's protocol implementation to align the plugin with the actively developed upstream SQL Server stack while the bridge preserves the API the driver uses. This is an internal transport change: connection settings and user-facing behaviour are unchanged, and existing users do not need to migrate anything.

**Discord** - [Join our discord server](https://discord.com/invite/K2hmhfHRSt) and chat with the maintainers.

## Table of Contents

- [Features](#features)
- [Connection Configuration](#connection-configuration)
- [Supported Data Types](#supported-data-types)
- [Installation](#installation)
- [Known Limitations](#known-limitations)
- [Building from Source](#building-from-source)
- [Development](#development)
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
- Procedure/function management, typed `OUT`/`INOUT` variables, and table-valued functions
- Static and runtime execution plans through `SHOWPLAN_XML` / `STATISTICS XML`, rendered in Tabularis's Visual EXPLAIN
- JavaScript-safe `BIGINT` extraction and broad SQL Server type handling

## Connection Configuration

| Parameter | Default | Notes |
|-----------|---------|-------|
| Host | `localhost` | |
| Port | `1433` | |
| Username | `sa` | SQL authentication only |
| Password | — | |
| Database | — | The database the pool connects to |
| Startup script | — | SQL run on every new pooled connection (e.g. `SET` options) |

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

## Supported Data Types

All common SQL Server types are supported for column creation and value extraction, including exact/approximate numerics (`TINYINT` … `BIGINT`, `DECIMAL`, `MONEY`, `FLOAT`), strings (`CHAR`/`VARCHAR`/`NVARCHAR` incl. `MAX`, `TEXT`/`NTEXT`), binary (`BINARY`/`VARBINARY`/`IMAGE`), date/time (`DATE`, `TIME`, `DATETIME`, `DATETIME2`, `SMALLDATETIME`, `DATETIMEOFFSET`), `BIT`, `UNIQUEIDENTIFIER`, `XML`, `SQL_VARIANT`, `ROWVERSION`, `HIERARCHYID`, and spatial (`GEOGRAPHY`, `GEOMETRY`).

`BIGINT` values outside JavaScript's safe integer range are delivered as strings so they round-trip without precision loss.

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

## Known Limitations

- SQL authentication only; Azure AD and Windows Integrated Authentication are follow-up work.
- Primary-key membership changes are disabled: the single-column alteration API cannot safely preserve composite PKs and referencing foreign keys.
- Custom CA files are rejected explicitly; strict verification uses the system trust store.

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

## Credits

The SQL Server driver implementation was contributed by [Fabio Malpezzi](https://github.com/FabioMalpezzi), originally developed as a built-in Tabularis driver and adapted here to the plugin architecture.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
