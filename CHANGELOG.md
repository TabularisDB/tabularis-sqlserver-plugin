# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Replaced the `tiberius` TDS client with Microsoft's `mssql-tds` implementation through `mssql-tiberius-bridge`, preserving the plugin's user-facing connection and query behaviour

### Added

- Automated live SQL Server 2022 JSON-RPC integration tests for TLS, DDL, CRUD, result-set metadata, affected rows, identity recovery, pagination, error recovery, execution plans, and startup scripts
- Initial SQL Server driver with `deadpool` pooling, TLS modes, session reset, and startup scripts
- Schema, table, column, PK/FK, index, view, routine, and trigger introspection
- Query execution with pagination, CTE/DML classification, multiple result sets, and accurate affected rows (incl. DML `OUTPUT`)
- INSERT/UPDATE/DELETE with composite primary keys and safe `IDENTITY_INSERT` recovery
- Table/view/index/foreign-key DDL and safe `ALTER COLUMN` generation
- Procedure/function management, typed `OUT`/`INOUT` variables, and table-valued functions
- Static and runtime execution plans through `SHOWPLAN_XML` / `STATISTICS XML`, parsed into the visual-plan model
- JavaScript-safe `BIGINT` extraction and broad SQL Server type handling
