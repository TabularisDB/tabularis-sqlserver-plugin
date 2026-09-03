# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The release candidate version is `1.0.0-beta.1`; it has not yet been tagged or
published. Publication is tracked in
[issue #4](https://github.com/TabularisDB/tabularis-sqlserver-plugin/issues/4).

### Breaking

- Visual EXPLAIN now returns raw `sqlserver-showplan-xml` for the plugin-owned
  TypeScript parser instead of parsing SHOWPLAN in the Rust process. This
  requires Tabularis 0.23.0 or later, the first intended runtime with raw
  plugin EXPLAIN output and plugin parser-bundle loading.

### Changed

- Replaced the `tiberius` TDS client with Microsoft's `mssql-tds`
  implementation through `mssql-tiberius-bridge`, preserving result-set
  metadata, affected-row reporting, session recovery, pagination, and static
  and runtime execution-plan capture.
- Adopted the `1.0.0-beta.N` prerelease line for the completed driver instead
  of retaining the scaffold's `0.1.0` version. Pull-request
  `prerelease:alpha`, `prerelease:beta`, `prerelease:rc`, and
  `prerelease:stable` labels drive version suggestions.
- Aligned paging with host lookahead semantics, made totals explicitly
  on-demand, and capped each statement at 10,000 retained rows across result
  sets.

### Added

- URL and ADO.NET/ODBC connection strings with deterministic reconciliation
  against discrete connection fields and normalized pool keys.
- Raw BLOB export and bounded MIME-sniffed previews for SQL Server binary
  types, including composite-primary-key lookup and oversized-value guards.
- Manifest-backed initialization settings for pool sizing, connection and
  query timeouts, TDS application identity, certificate trust, and idle pool
  eviction.
- SQL-authenticated login and database-user lifecycle management, privilege
  catalogs, direct and inherited grant reporting, and transactional privilege
  changes.
- Registry-grade manifest metadata, SQL Server branding and screenshots,
  native type mappings, synchronized data-type declarations, and release
  workflows for five desktop platforms.
- A browser-safe SQL Server SHOWPLAN parser built as both the plugin IIFE and
  the independently publishable `@tabularis/explain-sqlserver` package.
- CI checks for formatting, Clippy, unit and live SQL Server 2022 tests,
  manifest and Markdown validation, Conventional Commit pull-request titles,
  version suggestions, dependency updates, RustSec advisories, release
  tag/version agreement, and the TypeScript parser package.
- Host-model conformance fixtures for every implemented RPC and a live type
  matrix for all 37 manifest-advertised SQL Server types.

### Fixed

- Hardened identifier quoting and separated identifier, bound-value and
  explicit raw-SQL boundaries throughout CRUD, DDL, routine, trigger and user
  management.
- Preserved exact numeric, temporal, BLOB, UDT and `SQL_VARIANT` values across
  reads and row edits; concurrency-token types remain read-only.
- Added structured SQL Server error categories, credential redaction and safe
  replacement of timed-out, failed, transactional or dead pooled sessions.
- Reset SHOWPLAN, `IDENTITY_INSERT`, startup-script and temporary-table state
  before pooled session reuse.

### Performance

- Added bounded request and response queues, explicit idle-pool closing and
  regression coverage for pool identity, million-row truncation and concurrent
  responsiveness.

[Unreleased]: https://github.com/TabularisDB/tabularis-sqlserver-plugin/compare/main...HEAD
