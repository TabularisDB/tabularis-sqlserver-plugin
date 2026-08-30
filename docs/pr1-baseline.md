# PR #1 baseline

Measured on 2026-08-30 against the fetched PR head, before making any source
changes.

## Revisions and mergeability

- PR head: `f2afb7bb17559962e18e1f4d42ba9dfd6f91c706`
- `origin/feat/mssql-tds-bridge`: the same revision; the local branch was
  neither ahead nor behind.
- Fetched `origin/main`: `1a398810e7bc94064203442193482834f0367d81`
- Merge base: `1a398810e7bc94064203442193482834f0367d81`
- `git rev-list --left-right --count origin/main...HEAD`: `0 1`

The merge base is exactly `origin/main`, so PR #1 remains mergeable as a
fast-forward with no conflicting main-branch commits.

## Measurement environment

- Linux `5.4.0-216-generic` x86-64, Ubuntu 20.04
- `rustc 1.98.0 (88d9e12ae 2026-08-18)`
- `cargo 1.98.0 (797e8a9bc 2026-08-05)`
- Docker `26.1.3`

The machine did not have `pkg-config` or OpenSSL development headers installed,
so the first plain `cargo build` stopped in `openssl-sys` with exit status 101.
`sudo` required an unavailable password. To distinguish that machine
prerequisite from branch health, Ubuntu's `pkg-config` and `libssl-dev`
packages were downloaded and extracted under `/tmp`; the Rust commands below
were then run with `OPENSSL_INCLUDE_DIR` and `OPENSSL_LIB_DIR` pointed at those
extracted files. No repository files or dependency selections were changed.

## Cargo gate

| Command | Result |
| --- | --- |
| `cargo build` | Passed; dev profile completed |
| `cargo test` | Passed; 134 unit tests passed, 0 failed, plus 0 tests in `test_plugin` |
| `cargo clippy --all-targets -- -D warnings` | Passed with no warnings |
| `cargo fmt --check` | Passed with no diff |

The measured test count confirms 134 passing tests at the PR head.

## Bridge dependency

`cargo tree -p mssql-tiberius-bridge` resolved the root package as
`mssql-tiberius-bridge v0.1.0-preview.3`; its direct TDS implementation resolved
as `mssql-tds-preview v0.1.0-preview.1`.

The maximum depth printed by that command is **6 dependency edges** (**7
levels including the bridge root**). One deepest path is:

```text
mssql-tiberius-bridge
└── mssql-tds-preview
    └── x509-parser
        └── asn1-rs
            └── asn1-rs-derive
                └── synstructure
                    └── syn
```

Depth was measured from the four-character indentation levels in the actual
`cargo tree` output, with the bridge root at depth zero.

## Release binary

`cargo build --release` passed. The resulting Linux x86-64 binary was:

```text
path:  target/release/sqlserver-plugin
size:  3,023,312 bytes (2.9 MiB as reported by du -h)
format: ELF 64-bit, dynamically linked, stripped
```

## Files changed by the measured PR head

`git diff --name-status origin/main...HEAD` reported 17 files, all modified:

```text
M  CHANGELOG.md
M  CLAUDE.md
M  Cargo.lock
M  Cargo.toml
M  README.md
M  src/driver/explain.rs
M  src/driver/extract/mod.rs
M  src/driver/extract/temporal.rs
M  src/driver/helpers.rs
M  src/driver/helpers/tests.rs
M  src/driver/introspection.rs
M  src/driver/introspection/tests.rs
M  src/driver/mod.rs
M  src/driver/ops.rs
M  src/driver/pool.rs
M  src/driver/triggers/mod.rs
M  src/main.rs
```

This list describes the client-swap commit under measurement; this baseline
document is the subsequent SS-000 addition.

## Live SQL Server smoke test

The existing `sqlserver-dev` container was running the required
`mcr.microsoft.com/mssql/server:2022-latest` image. Its measured image ID was
`sha256:90488d58c6a5c19f24ff716e14330b85b5b26ee54b44a36ea24e6206533e7edd`,
and SQL Server reported `16.0.4265.3 RTM Developer Edition (64-bit)`.
`just seed-sqlserver` completed successfully and selected `tabularis_test`.

A hand-written JSON-RPC request was piped as one line into
`target/debug/sqlserver-plugin`:

```text
> {"jsonrpc":"2.0","id":1,"method":"test_connection","params":{"params":{"driver":"sqlserver","host":"127.0.0.1","port":1433,"username":"sa","password":"Str0ng!Passw0rd","database":"tabularis_test","ssl_mode":"disable"}}}
< {"id":1,"jsonrpc":"2.0","result":{"success":true}}
```

The process exited after stdin closed and emitted nothing on stderr.

## Closing comparison

Measured on 2026-08-30 at `cb5a243`, after the review, dependency audit, and
live-suite tasks and before the close-out documentation commit.

| Measure | Opening | Closing | Difference |
| --- | ---: | ---: | ---: |
| Unit tests | 134 | 149 | +15 |
| Automated live SQL Server tests | 0 | 12 | +12 |
| Release binary | 3,023,312 bytes | 6,064,496 bytes | +3,041,184 bytes (+100.6%) |
| Release binary (`du -h`) | 2.9 MiB | 5.8 MiB | +2.9 MiB |

The closing unit count comes from `cargo test --bins`; the live count comes
from `cargo test --test live_db -- --test-threads=1`; and the release size was
measured after `cargo build --release` with `stat -c %s` and `du -h`. All 149
unit tests and all 12 live tests passed.
