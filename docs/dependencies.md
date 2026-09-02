# Dependency and supply-chain review

This review was performed on 2026-08-30 against the versions locked in
`Cargo.lock`. The SQL Server transport is intentionally treated as a
release-critical dependency because both crates are preview releases.

## TDS dependency provenance

### `mssql-tiberius-bridge`

- **Resolved version:**
  [`0.1.0-preview.3`](https://crates.io/crates/mssql-tiberius-bridge/0.1.0-preview.3),
  source commit
  [`fcd9008`](https://github.com/saurabh500/mssql-tiberius-bridge/commit/fcd9008e6ee38e6098c01a3b7547125b84b5e54a).
- **Upstream:**
  [`saurabh500/mssql-tiberius-bridge`](https://github.com/saurabh500/mssql-tiberius-bridge),
  maintained separately from Microsoft. It provides a Tiberius-compatible API
  over `mssql-tds` and pins the protocol crate itself.
- **Licence:** MIT as declared by the
  [published manifest](https://docs.rs/crate/mssql-tiberius-bridge/0.1.0-preview.3/source/Cargo.toml.orig).
  MIT is compatible with this plugin's Apache-2.0 licence. The upstream
  repository and published crate do not currently include a standalone licence
  text, so the declaration is the licence evidence. Resolving that omission
  and the release archive's third-party-notice policy before public binary
  distribution is tracked in
  [issue #4](https://github.com/TabularisDB/tabularis-sqlserver-plugin/issues/4).
- **Release cadence:** all five published previews arrived in a ten-day burst:
  preview.1 on 2026-05-08, preview.2 and preview.3 on 2026-05-10, preview.4
  later on 2026-05-10, and
  [preview.5](https://github.com/saurabh500/mssql-tiberius-bridge/releases/tag/v0.1.0-preview.5)
  on 2026-05-18. There has been no release after preview.5, and the repository's
  [latest commit](https://github.com/saurabh500/mssql-tiberius-bridge/commit/9a3d10a5b810a02cc0a59dc7cc91b2ef8835f4b0)
  was 2026-05-21.
- **Maintenance status:** the repository is public and not archived, but it is
  a young, single-owner project. The initial development burst has not been
  followed by a commit in more than three months, the generated
  [preview.6 pull request](https://github.com/saurabh500/mssql-tiberius-bridge/pull/97)
  remains open, and a recent reliability report has no response yet. Treat it
  as active-but-unproven rather than as a stable, regularly maintained client.

Preview.4 and preview.5 already exist, but this plugin stays on the exact
preview.3 build that was reviewed and smoke-tested. A caret requirement on a
`0.1.0-preview.*` compatibility layer would permit an unreviewed API or wire
behaviour change when the lockfile is refreshed. The `=` pin makes an upgrade
an explicit change with its own dependency diff and live SQL Server evidence.

### `mssql-tds-preview`

- **Resolved version:**
  [`0.1.0-preview.1`](https://crates.io/crates/mssql-tds-preview/0.1.0-preview.1),
  source commit
  [`d43bdcc`](https://github.com/saurabh500/mssql-rs/commit/d43bdcc2f2d6f16155a259e2db7240365e6271df).
- **Published upstream:**
  [`saurabh500/mssql-rs`](https://github.com/saurabh500/mssql-rs), a publishable
  fork of Microsoft's
  [`microsoft/mssql-rs`](https://github.com/microsoft/mssql-rs) until Microsoft
  publishes the official crate. The plugin therefore consumes a third-party
  crates.io package even though the protocol implementation originated at
  Microsoft.
- **Licence:** MIT in the
  [crate manifest](https://docs.rs/crate/mssql-tds-preview/0.1.0-preview.1/source/Cargo.toml.orig)
  and the
  [fork licence](https://github.com/saurabh500/mssql-rs/blob/main/LICENSE).
  MIT is compatible with Apache-2.0.
- **Maintenance status:** the publishing fork had a commit on 2026-08-21 and
  Microsoft's source remains active. The published preview line has moved to
  preview.9, but the bridge preview.3 requires protocol preview.1 exactly.
  Advancing either dependency independently is not supported.

The plugin has a direct dependency on `mssql-tds-preview` because the bridge
does not re-export the result-set traits used through `Client::inner_mut()`.
Its default integrated-authentication feature is disabled: the plugin ships
SQL authentication only.

## Protocol crate dependency surface

The exact preview.1
[manifest](https://docs.rs/crate/mssql-tds-preview/0.1.0-preview.1/source/Cargo.toml.orig)
declares these runtime dependencies:

- async/runtime and I/O: `async-trait`, `tokio` with `full`, `tokio-util` with
  `full`, `futures`, `bytes`, `byteorder`, `socket2`, and `tracing`;
- TLS and certificate parsing: `native-tls` with ALPN, `tokio-native-tls`, and
  `x509-parser`;
- SQL values and text: `bigdecimal`, `uuid` with v4 and fast RNG,
  `encoding_rs`, and `bitflags`;
- networking and support: `dns-lookup`, `hostname`, `pretty-hex`, and
  `thiserror`;
- platform dependencies: `libc` on Unix and `winapi` plus `windows` on
  Windows.

On Linux, `native-tls` means the release environment must provide OpenSSL. The
bridge disables the protocol crate's default features, and this plugin now does
so on its direct edge as well, preventing the unused `integrated-auth` feature
from being unified back into the build.

## Open upstream issues relevant to this plugin

The following open issues touch code paths the plugin uses. They are reviewed
on every bridge upgrade; issue links and status are current as of the review
date above.

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

When considering preview.4 or any later release:

1. Read the bridge and protocol changelogs and compare both source tags against
   the currently recorded commits. Triage the issues above and all new issues
   touching pooling, TLS, query draining, metadata, values, or DML counts.
2. Inspect the candidate bridge manifest and update
   `mssql-tiberius-bridge` and the direct `mssql-tds-preview` pin together to
   the exact protocol version it requires. Keep `default-features = false` on
   the protocol edge.
3. Run `cargo update` only for those packages, review the complete
   `Cargo.lock` and `cargo tree -p mssql-tiberius-bridge` diffs, and repeat the
   licence inventory below for every newly resolved package.
4. Run `cargo audit`, unit tests, clippy, formatting, a release build, and the
   live SQL Server integration suite. The live suite must cover zero-row
   metadata, DML row counts, `IDENTITY_INSERT`, pagination, error recovery,
   pool reuse, TLS modes, and SHOWPLAN capture.
5. Land the upgrade as an explicit dependency change. Never relax the exact
   pin merely because upstream labels two previews API-compatible.

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
| `MIT OR Apache-2.0` | `asn1-rs 0.7.2`, `asn1-rs-derive 0.6.0`, `chacha20 0.10.2`, `core-foundation 0.10.1`, `cpufeatures 0.3.0`, `der-parser 10.0.0`, `deranged 0.5.8`, `displaydoc 0.2.7`, `getrandom 0.4.3`, `hashbrown 0.15.5`, `native-tls 0.2.18`, `num-bigint 0.4.8`, `num-conv 0.2.2`, `num-integer 0.1.46`, `oid-registry 0.8.1`, `openssl-probe 0.2.1`, `pkg-config 0.3.33`, `powerfmt 0.2.0`, `rand 0.10.2`, `rand_core 0.10.1`, `security-framework 3.7.0`, `socket2 0.5.10`, `tempfile 3.27.0`, `thiserror 2.0.19`, `thiserror-impl 2.0.19`, `time 0.3.54`, `time-core 0.1.9`, `time-macros 0.2.32`, `windows 0.58.0`, `windows-core 0.58.0`, `windows-implement 0.58.0`, `windows-interface 0.58.0`, `windows-result 0.2.0`, `windows-strings 0.1.0`, `windows-sys 0.60.2`, `windows-targets 0.53.5`, `windows_aarch64_gnullvm 0.53.1`, `windows_aarch64_msvc 0.53.1`, `windows_i686_gnu 0.53.1`, `windows_i686_gnullvm 0.53.1`, `windows_i686_msvc 0.53.1`, `windows_x86_64_gnu 0.53.1`, `windows_x86_64_gnullvm 0.53.1`, `windows_x86_64_msvc 0.53.1`, `x509-parser 0.18.1` |
| `MIT/Apache-2.0` | `asn1-rs-impl 0.2.0`, `bigdecimal 0.4.10`, `dns-lookup 2.1.1`, `foreign-types 0.3.2`, `foreign-types-shared 0.1.1`, `minimal-lexical 0.2.1`, `openssl-macros 0.1.1`, `rusticata-macros 4.1.0`, `vcpkg 0.2.15`, `winapi 0.3.9`, `winapi-i686-pc-windows-gnu 0.4.0`, `winapi-x86_64-pc-windows-gnu 0.4.0` |
| `MIT` | `async-stream 0.3.6`, `async-stream-impl 0.3.6`, `data-encoding 2.11.0`, `hostname 0.4.2`, `libm 0.2.16`, `mssql-tds-preview 0.1.0-preview.1`, `mssql-tiberius-bridge 0.1.0-preview.3`, `nom 7.1.3`, `openssl-sys 0.9.117`, `pretty-hex 0.4.2`, `synstructure 0.13.2`, `tokio-native-tls 0.3.1` |
| `Apache-2.0 OR MIT` | `fastrand 2.5.0` |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | `linux-raw-sys 0.12.1`, `rustix 1.1.4` |
| `Apache-2.0` | `openssl 0.10.81` |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | `r-efi 6.0.0` |

<!-- markdownlint-enable MD013 -->

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
