# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0-beta.1] - 2026-08-30

### Changed

- Replaced the `tiberius` TDS client with Microsoft's `mssql-tds`
  implementation through `mssql-tiberius-bridge`, preserving result-set
  metadata, affected-row reporting, session recovery, pagination, and static
  and runtime execution-plan capture.
- Adopted the `1.0.0-beta.N` prerelease line for the completed driver instead
  of retaining the scaffold's `0.1.0` version. Pull-request
  `prerelease:alpha`, `prerelease:beta`, `prerelease:rc`, and
  `prerelease:stable` labels drive version suggestions.

### Added

- URL and ADO.NET/ODBC connection strings with deterministic reconciliation
  against discrete connection fields and normalized pool keys.
- Raw BLOB export and bounded MIME-sniffed previews for SQL Server binary
  types, including composite-primary-key lookup and oversized-value guards.
- Manifest-backed initialization settings for pool sizing, connection and
  query timeouts, TDS application identity, certificate trust, and idle pool
  eviction.
- SQL-authenticated login and database-user lifecycle management, privilege
  catalogs, direct and inherited grant reporting, and transactional
  privilege changes.
- Registry-grade manifest metadata, SQL Server branding and screenshots,
  native type mappings, synchronized data-type declarations, and release
  archives for five desktop platforms.
- CI checks for formatting, Clippy, unit and live SQL Server 2022 tests,
  manifest and Markdown validation, Conventional Commit pull-request titles,
  version suggestions, dependency updates, RustSec advisories, and release
  tag/version agreement.
- Schema and object introspection, query and batch execution, CRUD, DDL,
  views, routines, triggers, visual execution plans, JavaScript-safe integer
  extraction, and broad SQL Server type handling.

[Unreleased]: https://github.com/TabularisDB/tabularis-sqlserver-plugin/compare/v1.0.0-beta.1...HEAD
[1.0.0-beta.1]: https://github.com/TabularisDB/tabularis-sqlserver-plugin/releases/tag/v1.0.0-beta.1
