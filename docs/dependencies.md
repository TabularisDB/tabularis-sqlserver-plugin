# Dependency and supply-chain review

Updated 2026-10-10 for the locked `mssql-tiberius-bridge` 0.1.0 release. The
bridge and protocol dependencies are exact-pinned because the plugin also uses
the protocol API directly through `Client::inner_mut()`.

## TDS dependency provenance

### `mssql-tiberius-bridge`

- **Resolved version:** [`0.1.0`](https://crates.io/crates/mssql-tiberius-bridge/0.1.0),
  released 2026-09-11. This is the latest published bridge version as of this
  review.
- **Upstream:** [`saurabh500/mssql-tiberius-bridge`](https://github.com/saurabh500/mssql-tiberius-bridge).
  The MIT-licensed crate provides a Tiberius-compatible API over Microsoft's
  `mssql-tds` protocol implementation.
- **Upgrade impact:** 0.1.0 replaces the preview TDS crate with published
  `mssql-tds` 0.1.0 and changes its result-set and error APIs. It also adds
  `Client::reset_session()`, which performs a native TDS session reset and
  restores `READ COMMITTED`; the pool uses this API before reapplying the
  configured startup script.

### `mssql-tds`

- **Resolved version:** [`0.1.0`](https://crates.io/crates/mssql-tds/0.1.0),
  the protocol API required by bridge 0.1.0. Although `mssql-tds` 0.2.0 is now
  published, bridge 0.1.0 requires the 0.1 API, so the plugin pins 0.1.0.
- **Upstream and licence:** Microsoft's
  [`microsoft/mssql-rs`](https://github.com/microsoft/mssql-rs), MIT.
- The direct dependency exposes result-set metadata and row iteration not
  re-exported by the bridge. Its default authentication features remain enabled
  for Windows/Kerberos support.

## Historical review notes

The sections below retain findings from the 2026-08-30 review of preview.3 and
preview.1. Their issue-status and license-inventory details were not
re-audited as part of this dependency upgrade; do not treat them as current
findings for bridge 0.1.0.

## Open upstream issues relevant to this plugin

The following issues were recorded in the historical review. Re-triage them
against the exact source versions before relying on their status.

- [Bridge #104](https://github.com/saurabh500/mssql-tiberius-bridge/issues/104)
  reports pooled connections intermittently returning no rows after 5–15
  minutes. This directly affects the deadpool usage here and is not yet
  explained. Pool recycling calls `sp_reset_connection`, but that has not been
  demonstrated to prevent this report.
- [Bridge #1](https://github.com/saurabh500/mssql-tiberius-bridge/issues/1)
  reports `execute()` returning zero affected rows for DML. The plugin does not
  trust that API: its raw TDS batch appends a `@@ROWCOUNT` sentinel and parses
  the exact count.
- [Bridge #52](https://github.com/saurabh500/mssql-tiberius-bridge/issues/52)
  tracks a missing session-reset API. The pool explicitly executes
  `sp_reset_connection` and then reapplies the configured startup script on
  every recycle.
- [Bridge #63](https://github.com/saurabh500/mssql-tiberius-bridge/issues/63)
  tracks incomplete column metadata in the compatibility API. The query path
  uses `inner_mut()` and reads `mssql-tds` result-set metadata directly, which
  also preserves headers for zero-row result sets.
- [Bridge #88](https://github.com/saurabh500/mssql-tiberius-bridge/issues/88)
  says cancellation safety under `tokio::time::timeout` has not been audited.
  The plugin now applies its configured query timeout with Tokio and marks the
  connection non-recyclable on timeout, so no later request receives a stream
  with unread packets. The live suite verifies timeout categorization and
  replacement-session recovery. Re-audit this boundary on every bridge update.
- [Bridge #89](https://github.com/saurabh500/mssql-tiberius-bridge/issues/89)
  tracks the unverified encryption-off handshake. It is relevant to the
  plugin's `ssl_mode=disable` mapping and must be included in TLS live tests.
- [Bridge #90](https://github.com/saurabh500/mssql-tiberius-bridge/issues/90)
  tracks missing malformed UTF-16 regression coverage. The underlying decoder
  is expected to substitute U+FFFD, but the plugin has no independent wire
  fixture for that case.

The publishing fork's open issues concern metadata test constructors and
`sp_prepare`; the plugin uses neither prepared statements nor those private
constructors. Microsoft's current repository has newer issues, but they do not
necessarily describe the immutable preview.1 source. Any candidate upgrade
must triage the issues for its exact source commit rather than assuming fixes
or regressions carry across forks.

## Upgrade procedure

For future bridge upgrades:

1. Read the bridge and protocol changelogs and compare both source tags against
   the currently recorded versions. Triage issues touching pooling, TLS, query
   draining, metadata, values, or DML counts.
2. Inspect the bridge manifest and update `mssql-tiberius-bridge` and the
   direct `mssql-tds` pin together to the exact compatible protocol version.
3. Run `cargo update` only for those packages, review the complete
   `Cargo.lock` and `cargo tree -p mssql-tiberius-bridge` diffs, and repeat the
   licence inventory below for every newly resolved package.
4. Run `cargo audit`, unit tests, clippy, formatting, a release build, and the
   live SQL Server integration suite. The live suite must cover zero-row
   metadata, DML row counts, `IDENTITY_INSERT`, pagination, error recovery,
   pool reuse, TLS modes, and SHOWPLAN capture.
5. Land the upgrade as an explicit dependency change. Keep exact pins even for
   stable releases so the direct protocol dependency stays aligned.

## Fallback plan

If the bridge is abandoned or develops a blocking correctness, security, or
reliability bug that cannot be fixed promptly, the fallback is the stable
`tiberius 0.12` implementation that this branch replaced. It remains
available in the parent of client-swap commit
[`f2afb7b`](https://github.com/TabularisDB/tabularis-sqlserver-plugin/commit/f2afb7b)
and can be restored with an explicit rollback.
Restore that implementation rather than carrying an indefinite private fork of
both preview crates.

The rollback must restore `Cargo.toml` and `Cargo.lock`, then move the client
API adaptations back in `src/main.rs` and these driver files:

- `src/driver/mod.rs`, `ops.rs`, `pool.rs`, and `helpers.rs`;
- `src/driver/explain.rs`, `introspection.rs`, and `triggers/mod.rs`;
- `src/driver/extract/mod.rs` and `extract/temporal.rs`;
- the corresponding helper, introspection, and extraction tests.

`README.md`, `CHANGELOG.md`, and `CLAUDE.md` must again name Tiberius. Preserve
JSON-RPC behaviour and the post-swap correctness tests where their semantics
apply, then run the same unit, live-database, audit, clippy, formatting, and
release gates before publishing the rollback.

## Licence inventory

`cargo metadata --locked` was compared with `main` by package name and
version. The bridge swap introduces or upgrades the following 74 package
identities. Every SPDX expression offers MIT, Apache-2.0, or both; those
choices are compatible with an Apache-2.0 binary. Historical
`MIT/Apache-2.0` metadata means dual-licensed. For `r-efi`, the MIT alternative
is selected, not LGPL.

<!-- markdownlint-disable MD013 -->

| Declared licence | New or upgraded packages in the lock graph |
| --- | --- |
| `MIT OR Apache-2.0` | `asn1-rs 0.7.2`, `asn1-rs-derive 0.6.0`, `chacha20 0.10.2`, `core-foundation 0.10.1`, `cpufeatures 0.3.0`, `der-parser 10.0.0`, `deranged 0.5.8`, `displaydoc 0.2.7`, `getrandom 0.4.3`, `hashbrown 0.15.5`, `native-tls 0.2.18`, `num-bigint 0.4.8`, `num-conv 0.2.2`, `num-integer 0.1.46`, `oid-registry 0.8.1`, `openssl-probe 0.2.1`, `openssl-src 300.6.1+3.6.3`, `pkg-config 0.3.33`, `powerfmt 0.2.0`, `rand 0.10.2`, `rand_core 0.10.1`, `security-framework 3.7.0`, `socket2 0.5.10`, `tempfile 3.27.0`, `thiserror 2.0.19`, `thiserror-impl 2.0.19`, `time 0.3.54`, `time-core 0.1.9`, `time-macros 0.2.32`, `windows 0.58.0`, `windows-core 0.58.0`, `windows-implement 0.58.0`, `windows-interface 0.58.0`, `windows-result 0.2.0`, `windows-strings 0.1.0`, `windows-sys 0.60.2`, `windows-targets 0.53.5`, `windows_aarch64_gnullvm 0.53.1`, `windows_aarch64_msvc 0.53.1`, `windows_i686_gnu 0.53.1`, `windows_i686_gnullvm 0.53.1`, `windows_i686_msvc 0.53.1`, `windows_x86_64_gnu 0.53.1`, `windows_x86_64_gnullvm 0.53.1`, `windows_x86_64_msvc 0.53.1`, `x509-parser 0.18.1` |
| `MIT/Apache-2.0` | `asn1-rs-impl 0.2.0`, `bigdecimal 0.4.10`, `dns-lookup 2.1.1`, `foreign-types 0.3.2`, `foreign-types-shared 0.1.1`, `minimal-lexical 0.2.1`, `openssl-macros 0.1.1`, `rusticata-macros 4.1.0`, `vcpkg 0.2.15`, `winapi 0.3.9`, `winapi-i686-pc-windows-gnu 0.4.0`, `winapi-x86_64-pc-windows-gnu 0.4.0` |
| `MIT` | `async-stream 0.3.6`, `async-stream-impl 0.3.6`, `data-encoding 2.11.0`, `hostname 0.4.2`, `libm 0.2.16`, `mssql-tds-preview 0.1.0-preview.1`, `mssql-tiberius-bridge 0.1.0-preview.3`, `nom 7.1.3`, `openssl-sys 0.9.117`, `pretty-hex 0.4.2`, `synstructure 0.13.2`, `tokio-native-tls 0.3.1` |
| `Apache-2.0 OR MIT` | `fastrand 2.5.0` |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `linux-raw-sys 0.12.1`, `rustix 1.1.4` |
| `Apache-2.0` | `openssl 0.10.81` |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `r-efi 6.0.0` |

<!-- markdownlint-enable MD013 -->

On Linux targets the plugin enables the `vendored` feature of `openssl`, which
compiles OpenSSL 3.6 from the source bundled in `openssl-src` and links it
statically. The crate is `MIT OR Apache-2.0`; the bundled OpenSSL itself is
Apache-2.0 and its notice must ship with the Linux archives. This keeps the
linux-arm64 cross build independent of an aarch64 `libssl-dev` in the container
and the linux-x64 binary independent of the runner's OpenSSL major version.
macOS and Windows keep the platform TLS stack selected by `native-tls`.

This inventory is based on package manifests as resolved by Cargo, including
platform-specific and lockfile-only entries. Release packaging must retain the
applicable third-party notices; this review does not replace that packaging
step. The unresolved archive-policy work is tracked in issue #4.

## RustSec audit

`cargo audit 0.22.2` scanned 208 locked dependencies against 1,226 advisories.
It found:

- **RUSTSEC-2026-0235 (`rkyv 0.7.46`):** `rkyv` is present only because
  `rust_decimal` declares it as an optional feature. The plugin does not enable
  that feature and `cargo tree -i rkyv` reports no dependency path, so the
  vulnerable code is not compiled or reachable in the shipped binary. CI
  ignores this one advisory with an inline rationale. Re-check the ignore on
  every `rust_decimal` upgrade or feature change.
- **Yanked `chacha20 0.10.1`:** resolved by updating the lockfile to the
  non-yanked compatible `0.10.2`. It enters through
  `mssql-tds-preview -> uuid -> rand`.

After that lockfile update,
`cargo audit --ignore RUSTSEC-2026-0235` passes with no other vulnerability or
yank finding. CI runs the same audit on pushes and pull requests and on a
weekly off-peak schedule; scheduled runs have permission to file tracking
issues for new informational findings.
