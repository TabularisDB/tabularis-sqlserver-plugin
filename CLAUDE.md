# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A [Tabularis](https://github.com/TabularisDB/tabularis) driver plugin, written in Rust, that lets Tabularis connect to Microsoft SQL Server. Tabularis launches the compiled binary as a subprocess and talks to it over stdio using JSON-RPC (one JSON object per line in, one JSON object per line out). The plugin has no server of its own and no persistent state beyond an in-process connection-pool cache.

Full plugin contract (required RPC methods, manifest schema) lives in the upstream guide: `https://github.com/TabularisDB/tabularis/blob/main/plugins/PLUGIN_GUIDE.md`. The frozen contract for plugin-owned EXPLAIN parser work is [`docs/explain-architecture.md`](docs/explain-architecture.md).

## Commands

All common tasks are `just` recipes (see `justfile`); most just wrap `cargo`.

```bash
just build           # cargo build (debug)
just release         # cargo build --release
just test            # cargo test (no database required)
just lint            # cargo clippy --all-targets -- -D warnings
just fmt             # cargo fmt --all
just repl            # cargo run --bin test_plugin — local JSON-RPC sandbox
just dev-install     # build + copy binary/manifest into the local Tabularis plugins dir
just uninstall       # remove the installed plugin
just run-sqlserver   # SQL Server 2022 in Docker (sa / Str0ng!Passw0rd)
just seed-sqlserver  # create and seed the tabularis_test database
```

Run a single test: `cargo test <test_name>`.

## Architecture

```text
src/
  main.rs           # tokio entrypoint: stdin reader → worker pool → stdout writer
  rpc.rs            # JSON-RPC dispatch + response/param helpers
  models.rs         # serde shapes mirroring the Tabularis host's models
  common.rs         # query classification + JS-safe integer helpers
  pool_manager.rs   # per-connection-key deadpool cache
  handlers/         # thin JSON adapters, one module per RPC area
  driver/           # SQL Server logic
    ops.rs          # one free function per host RPC method
    pool.rs         # Microsoft mssql-tds client via bridge + deadpool Manager (TLS modes, startup scripts)
    introspection.rs, helpers.rs, ddl/, routines/, triggers/, types.rs, version.rs
    extract/        # row → JSON value extraction (incl. temporal types)
    explain.rs      # SHOWPLAN_XML / STATISTICS XML capture
    showplan.rs     # SHOWPLAN XML → visual-plan JSON (plugins return parsed plans)
```

Key invariants:

- JSON emitted by handlers must deserialize into the host's model structs — `models.rs` mirrors the host's serde shapes; don't change field names or nullability casually.
- `.tabularium` `data_types` mirrors `driver/types.rs::get_data_types()`; keep them in sync.
- `update_record`/`delete_record` receive a `pk_map` (composite PKs supported); ordering is normalized by sorting column names.
- Pending `SS-035`, this plugin still returns an in-process parsed SHOWPLAN. After `SS-035`, it returns raw `sqlserver-showplan-xml`; the host wraps it as raw EXPLAIN output and the plugin-owned TypeScript parser registered from `explain/dist/index.iife.js` produces the visual plan. Keep the raw shape, manifest declaration, parser bundle and runtime version floor aligned with `docs/explain-architecture.md`.
