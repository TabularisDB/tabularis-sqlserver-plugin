# Plugin completeness

This document records the remaining gaps between the SQL Server plugin and the
Tabularis host protocol and plugin registry. It is the repository-local
checklist for the completeness work; task identifiers refer to the project
plan used to deliver that work.

## Host protocol

The plugin already supports connection testing, schema introspection, query
execution, CRUD, DDL, views, routines, triggers, and visual EXPLAIN. The
remaining protocol gaps are:

- `initialize` accepts the request but ignores its settings. `SS-013` will
  validate and apply plugin settings.
- `save_blob_to_file` and `fetch_blob_as_data_url` support raw export and
  MIME-sniffed preview for `BINARY`, `VARBINARY` including `VARBINARY(MAX)`,
  and legacy `IMAGE` values. Composite primary keys are parameterized and
  normalized in deterministic column order.
- The database-user and privilege surface is absent. `SS-014` will implement
  `get_db_privilege_catalog`, `get_db_users`, `create_db_user`, `drop_db_user`,
  `set_db_user_password`, `get_db_user_privileges`,
  `apply_db_user_privileges`, and `get_db_user_grants`.
- `shutdown` and the materialized-view methods return `-32601`. `SS-015` will
  make these deliberate refusals, with tests and documentation: the host does
  not send `shutdown`, and SQL Server has indexed views rather than
  materialized views.
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

The current `.tabularium` file is a scaffold manifest rather than a
registry-grade manifest. It still needs:

- the registry schema URL and the `kind`, `engine`, `paradigms`, `category`,
  `tags`, and `license` metadata;
- icon, color, and screenshot assets;
- readme, homepage, documentation, and support links;
- `min_runtime_version` and type mappings;
- a settings declaration for the `initialize` implementation; and
- the plugin-owned `explain_parsers` declaration once the host supports it.

`SS-013`, `SS-020`, `SS-021`, `SS-030`, `SS-034`, and `SS-035` cover these
items.

## CI and release packaging

The Rust build, test, Clippy, formatting, live SQL Server integration, and
scheduled security-audit checks exist. Release readiness still requires:

- registry manifest validation;
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
