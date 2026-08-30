# Plugin completeness

This document records the remaining gaps between the SQL Server plugin and the
Tabularis host protocol and plugin registry. It is the repository-local
checklist for the completeness work; task identifiers refer to the project
plan used to deliver that work.

## Host protocol

The plugin already supports connection testing, schema introspection, query
execution, CRUD, DDL, views, routines, triggers, and visual EXPLAIN. The
remaining protocol gaps are:

- `initialize` applies forgiving process settings for pool sizing, connection
  and query timeouts, the TDS application name, certificate trust, and idle
  pool eviction. Unknown keys are ignored and malformed values use defaults.
- `save_blob_to_file` and `fetch_blob_as_data_url` support raw export and
  MIME-sniffed preview for `BINARY`, `VARBINARY` including `VARBINARY(MAX)`,
  and legacy `IMAGE` values. Composite primary keys are parameterized and
  normalized in deterministic column order.
- All eight database-user and privilege methods manage mapped SQL
  login/database-user pairs, direct and inherited grants, and DENY-safe
  privilege changes.
- The host currently terminates plugin processes directly and never sends an
  RPC `shutdown`; the courtesy method is implemented anyway and closes and
  removes every cached pool before replying `null`.
- The four materialized-view methods deliberately return a reasoned `-32601`.
  SQL Server indexed views are synchronously maintained views with clustered
  indexes, not refreshable materialized views, so mapping between them would
  misrepresent both lifecycle and semantics.
- A host-method coverage test snapshots every RPC sent by the host and requires
  each one to be dispatched or included in the reasoned `NOT_IMPLEMENTED`
  table. Unknown methods also return `-32601` naming the method and plugin.
- `explain_query` currently returns an in-process parsed plan. `SS-035` will
  return raw `sqlserver-showplan-xml` after the plugin-owned parser contract
  and host support are available.

## BLOB policy

`NULL` binary values return an explicit error and never become an empty file.
`ROWVERSION` and its deprecated `TIMESTAMP` synonym are not offered as BLOBs:
their eight bytes are server-generated concurrency tokens rather than user
file data. Direct RPC attempts against those types return an explanatory
error.

Preview requests accept the same top-level `max_blob_size` byte ceiling used
by BLOB write paths. The query checks `DATALENGTH` and omits the binary value
from the SQL result when it exceeds the ceiling, so an oversized
`VARBINARY(MAX)` is neither transferred over TDS nor base64-encoded into the
JSON-RPC line. The error reports the actual and configured sizes and suggests
file export, which remains unbounded. For compatibility with hosts that omit
the field, the plugin uses Tabularis' 100 MiB default.

## Connection parameters

The manifest advertises connection-string support, but `ConnectionParams`
does not contain `connection_string`; only discrete host, port, username,
password, and database fields work. `SS-011` will make the declared capability
functional.

SQL authentication remains the supported authentication mechanism. Azure AD,
Windows Integrated Authentication, custom CA files, and client certificates
are outside the completion scope.

## Manifest and settings

The `.tabularium` file validates against the live registry driver schema. It
now carries the registry metadata and links, SQL Server branding, runtime
floor, generic-to-native type mappings, and capabilities verified against the
implemented RPC surface. A unit test keeps its `data_types` list synchronized
with `driver/types.rs`; the registry-compatible `string` and `date` categories
replace the scaffold's unsupported `text` and `datetime` labels.

The manifest now links the scalable SQL Server icon and the complete eight-image
registry screenshot set. Remaining work is limited to the plugin-owned
`explain_parsers` declaration and corresponding runtime-version bump in
`SS-030`, `SS-034`, and `SS-035`.

## CI and release packaging

The Rust build, test, Clippy, formatting, live SQL Server integration,
registry manifest validation, and scheduled security-audit checks exist.
Release readiness still requires:

- Conventional Commit pull-request title checks;
- version-suggestion and Markdown lint jobs;
- Dependabot configuration;
- release validation that the tag and manifest versions agree;
- build and test coverage for the future `explain/` package;
- corrected developer install recipes; and
- registry assets and per-platform release archives.

These gaps are covered by `SS-022` through `SS-024` and `SS-034`.

## Distribution

There is no SQL Server entry in the Tabularis plugin registry, no published
GitHub release containing per-platform archives and a manifest asset, and no
published `@tabularis/explain-sqlserver` package. `SS-024` and `SS-034` add
those distribution paths after their prerequisites land.
