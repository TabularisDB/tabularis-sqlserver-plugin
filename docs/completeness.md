# Plugin completeness

This document records the SQL Server plugin's release-candidate state against
the Tabularis host protocol and registry. The implementation work is complete;
publication and cross-repository rollout remain tracked in
[issue #4](https://github.com/TabularisDB/tabularis-sqlserver-plugin/issues/4).

## Host protocol

The plugin supports connection testing, schema introspection, query execution,
CRUD, DDL, views, routines, triggers, BLOB handling, database users and
privileges, and Visual EXPLAIN. In particular:

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
- `shutdown` is implemented as a courtesy method that closes and removes every
  cached pool, although the current host normally terminates the subprocess.
- `explain_query` returns raw `sqlserver-showplan-xml`; Tabularis loads the
  plugin-owned TypeScript parser declared in `.tabularium`.

The only deliberate protocol exclusions are the four materialized-view
methods. SQL Server indexed views are synchronously maintained views with
clustered indexes, not refreshable materialized views, so those methods return
a reasoned `-32601` instead of misrepresenting their lifecycle. A host-method
coverage test requires every host RPC to be dispatched or included in that
reasoned table. Truly unknown methods return `-32601` naming the method and
plugin.

## Host model conformance

`tests/conformance.rs` carries verbatim response-model definitions from
Tabularis host commit `ba0463d3b861ec8fad110126c67e3fc12bac9839` and checks a
live-captured fixture for every implemented RPC. Regenerate all 54 responses
with `python3 tests/capture_conformance.py` whenever the host models or RPC
surface changes.

The conformance sync found two additive host fields that had been missing from
the plugin wire models. SQL Server column introspection now emits
`is_generated` from `sys.columns.is_computed`, including the schema snapshot
and batch path. Index metadata emits `is_expression: false` because SQL Server
does not support arbitrary expression index keys. Parameterized character
types retain `character_maximum_length`; absent lengths and defaults remain
omitted and deserialize through the host's optional fields.

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
the field, the plugin uses Tabularis's 100 MiB default.

## Connection parameters

Discrete host, port, username, password and database fields work alongside URL
and ADO.NET/ODBC `connection_string` forms. Explicit string values are
reconciled against discrete fields, passwords are redacted in conflicts, and
equivalent forms normalize to the same pool key. TLS modes, startup scripts and
connection ids are included in pool identity where they affect sessions.

SQL authentication remains the supported authentication mechanism. Azure AD,
Windows Integrated Authentication, custom CA files and client certificates are
outside the completion scope and documented as limitations.

## Manifest, settings and packaging

`.tabularium` carries registry metadata, SQL Server branding, the runtime
floor, generic-to-native type mappings, process settings, capabilities and the
`explain_parsers` declaration from core PR #688. The currently deployed live
registry schema predates that additive field and rejects it; schema deployment
and a clean live validation are publication prerequisites in issue #4. A unit
test keeps all 37 `data_types` synchronized with
`driver/types.rs::get_data_types()`.

The release workflow enforces tag/version equality and builds five platform
archives. Each archive stages the binary, manifest, screenshots and
`explain/dist/index.iife.js`; `.tabularium` is also staged as a standalone
release asset. The independent `explain-v*` workflow validates and publishes
`@tabularis/explain-sqlserver`.

CI covers Rust build, tests, Clippy and formatting; SQL Server 2022 integration;
manifest and Markdown validation; the TypeScript parser package; Conventional
Commit titles and version suggestions; and RustSec advisories. Until the live
schema is deployed, CI validates deployed fields with the live endpoint and the
pending `explain_parsers` declaration against its exact frozen contract.
Dependabot tracks Cargo, Actions and npm dependencies.

## Distribution status

The code and workflows are release-ready, but no plugin tag, GitHub release or
npm package has been published yet. The core parser-loader PR and standalone
site PR are also still open. Consequently no registry PR has been opened and
issue #2 remains open. Issue #4 contains the ordered publication, artifact
inspection, real-desktop validation, registry submission, site deployment and
issue-close checklist; it prevents incomplete or 404-backed registry metadata
from being submitted.
