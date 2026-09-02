# Plugin-owned EXPLAIN parsers — frozen contract

This document is the implementation contract for `SS-031` through `SS-036`.
It was frozen by `SS-030` on 2026-08-30 after checking Tabularis core commit
`9e6975aa5ef1d9667c0d7a27488b55adfe3cf584` and SQL Server plugin commit
`1718b3149f5e982109062c2ef40682252fbd9fb6`.

Statements in §1 describe that baseline and include source anchors. The later
sections are normative decisions for the implementation tasks; they do not
claim that the code exists at the frozen commits.

## Implementation status

The contract is implemented on the release-candidate branches. Core PR
[TabularisDB/tabularis#688](https://github.com/TabularisDB/tabularis/pull/688)
contains the registry, raw plugin protocol, manifest plumbing, author guide and
isolated desktop loader. This repository contains the TypeScript parser, ESM
package, IIFE and raw Rust handoff; `.tabularium` requires Tabularis 0.23.0.
The standalone site integration is in
[TabularisDB/explain-plan#2](https://github.com/TabularisDB/explain-plan/pull/2).

The cross-repository PRs are not merged, Tabularis 0.23.0 and
`@tabularis/explain` 0.2.0 are not published, and the SQL Server npm package is
not published. Those distribution prerequisites and the required real-desktop
check are tracked in
[issue #4](https://github.com/TabularisDB/tabularis-sqlserver-plugin/issues/4).
This status note does not alter the frozen normative contract below.

## 1. Verified baseline

The current split is real, but several details in the initial design needed
correction.

| Claim | Verified source |
| --- | --- |
| Raw built-in output has five closed format literals and is dispatched by an exhaustive `switch`. | `tabularis/packages/explain/src/raw.ts:22-27` and `:62-74` |
| Standalone source parsing has a separate four-entry parser array, a closed three-engine union and a second dispatch path. | `tabularis/packages/explain/src/parsers/source.ts:17-54` and `:72-160` |
| The only format-related switches under `packages/explain/src` are the raw-format switch and the source engine switch. | `raw.ts:63` and `parsers/source.ts:112` at the frozen core commit |
| Plugin `explain_query` output is always wrapped as `Plan`. | `tabularis/src-tauri/src/plugins/driver.rs:800-816` |
| The Rust host already has serializable `RawExplainOutput` and tagged `ExplainQueryOutput` models; `original_query` is currently required. | `tabularis/src-tauri/src/models.rs:526-548` |
| SQL Server captures estimated or runtime XML, then parses it in process. | `src/driver/explain.rs:11-50` and `src/driver/ops.rs:379-392` |
| The Rust SHOWPLAN parser uses the first `RelOp`, respects nested-operator ownership and aggregates runtime counters by thread. | `src/driver/showplan.rs:12-178`, especially `:105-138` |
| The existing parser stores `EstimatedTotalSubtreeCost` directly as `total_cost`; it does not subtract child cost or use `AvgRowSize`. | `src/driver/showplan.rs:165-168` |
| `read_plugin_file` accepts nested relative UTF-8 text paths and rejects paths containing `..` or beginning with `/` or `\`. | `tabularis/src-tauri/src/plugins/commands.rs:402-419` |
| UI IIFEs are actually read and evaluated in `PluginSlotProvider`; `pluginModuleLoader.ts` is a separate dynamic-loader abstraction and is not the production IIFE evaluator. | `tabularis/src/contexts/PluginSlotProvider.tsx:62-137` and `src/utils/pluginModuleLoader.ts:14-75` |
| The frontend already has an enabled-plugin manifest effect where parser loading can be attached. | `tabularis/src/contexts/PluginSlotProvider.tsx:140-192` |
| Runtime manifests pass `ui_extensions` through Rust and TypeScript models, even though the local legacy schema does not declare that field. | `tabularis/src-tauri/src/plugins/manager.rs:31-69`, `src-tauri/src/drivers/driver_trait.rs:210-263` and `src/types/plugins.ts:73-117` |
| The local manifest schema has `additionalProperties: false` and declares neither `ui_extensions` nor `explain_parsers`. | `tabularis/plugins/manifest.schema.json:1-259` |

`read_plugin_file` is sufficient for the parser bundle because JavaScript is
UTF-8 text and `explain/dist/index.iife.js` is a valid nested relative path.
Its validation is lexical, not canonical: the current command does not prove
that a symlink target remains below the plugin directory. This contract does
not overstate that guarantee.

Opening only `RawExplainFormat` while retaining the `switch` in `raw.ts` would
make `parseRawPayload` non-exhaustive. The registry replaces that switch.
Supporting third-party source detection also requires opening
`ExplainEngine` and `ExplainSourceFormat`; the initial design omitted those
two type changes. No other current switch needs changing.

The current Rust parser already implements the per-thread aggregation credited
to the closed core PR #560: sum `ActualRows` and `ActualExecutions`, and take
the maximum `ActualElapsedms`. Neither issue #2 nor PR #560 specifies
subtracting child subtree costs or mapping `AvgRowSize`; those were erroneous
claims in the initial design and are not part of this contract.

## 2. Goals and ownership

A SQL Server plan has one parser implementation in this repository, written in
TypeScript. It is built into:

1. an IIFE shipped in each plugin archive for the desktop; and
2. an ESM npm package consumed by the standalone visualizer.

SQL Server parsing remains owned and released by the SQL Server plugin. Core
`@tabularis/explain` gains only an engine-neutral parser registry. This differs
from issue #2's original proposal, which would put SQL Server parsing directly
in the core package.

This split allows any third-party plugin to supply a parser without waiting for
a core release, while the npm artifact makes the same parser available where
there is no plugin process. Renderer-only changes already apply to parsed
plugin plans today; the concrete problems solved here are duplicated parser
implementations, parser/model evolution tied to a Rust binary, and the
standalone site's inability to reach that binary.

## 3. `@tabularis/explain` registry (`SS-031`)

### 3.1 Public API

Add `packages/explain/src/registry.ts` and export these symbols from the
package root:

```ts
export interface RegisteredExplainParser {
  /** Canonical engine id, for example "sqlserver". */
  readonly engine: string;
  /** Globally unique wire-format tag. */
  readonly format: string;
  /** Human label for format pickers. */
  readonly label?: string;
  /** Parse the raw payload or throw an Error. */
  parse(payload: string): ExplainPlan;
  /** Cheap, side-effect-free source detection. */
  sniff?(payload: string): boolean;
}

export function registerExplainParser(
  parser: RegisteredExplainParser,
): void;
export function unregisterExplainParser(format: string): void;
export function getExplainParser(
  format: string,
): RegisteredExplainParser | null;
export function listExplainParsers(): readonly RegisteredExplainParser[];
```

The existing public types become open while preserving literal autocomplete:

```ts
export type BuiltinRawExplainFormat =
  | "postgres-json"
  | "mysql-json"
  | "mysql-analyze-text"
  | "mysql-tabular-rows"
  | "sqlite-eqp-rows";
export type RawExplainFormat =
  | BuiltinRawExplainFormat
  | (string & {});

export type BuiltinExplainEngine = "postgres" | "mysql" | "sqlite";
export type ExplainEngine = BuiltinExplainEngine | (string & {});

export type BuiltinExplainSourceFormat =
  | "postgres-json"
  | "postgres-text"
  | "mysql-json"
  | "mysql-text";
export type ExplainSourceFormat =
  | BuiltinExplainSourceFormat
  | (string & {});
```

### 3.2 Built-ins and mutation rules

The effective registry has an immutable built-in baseline and a mutable
registration overlay. The baseline contains all existing dispatch tags, not
only the five raw wire tags:

- `postgres-json`
- `postgres-text`
- `mysql-json`
- `mysql-text`
- `mysql-analyze-text`
- `mysql-tabular-rows`
- `sqlite-eqp-rows`

Aliases that share a parser remain separate format entries. Dispatch for raw
host output and standalone source parsing therefore reaches the same effective
registry.

Registration rules are exact:

- `engine` and `format` must be non-empty after trimming and `parse` must be a
  function. Invalid registrations throw `TypeError` before mutating state.
- A new custom format is appended in registration order.
- Registering an already effective format installs or replaces its overlay and
  emits exactly one `console.warn` for that call:
  `EXPLAIN parser format '<format>' is already registered; replacing it.`
- Replacement keeps the format's existing position. This makes an in-place
  plugin upgrade deterministic.
- `unregisterExplainParser` removes only the mutable overlay. It is a no-op for
  an absent overlay; removing an override reveals the immutable built-in.
- `listExplainParsers` returns an immutable snapshot of effective entries:
  built-in order first, followed by custom registration order. A built-in
  override occupies the built-in's original position.
- Parser exceptions propagate unchanged to the caller.

These rules prevent tests or plugin unloads from accidentally deleting core
parsers while still allowing a deliberate override.

### 3.3 Dispatch, source detection and exact errors

`parseRawExplain` looks up `raw.format` in the registry and invokes its
`parse`. If there is no entry, it throws exactly:

```text
No EXPLAIN parser registered for format '<format>' (engine '<engine>'). Import the parser package for '<engine>' before parsing.
```

It then stamps `driver` and `original_query` exactly as it does today.

`parsers/source.ts` also uses the registry for final parser dispatch. Detection
preserves the current built-in behavior before consulting custom sniffers:

- With a built-in engine hint, existing Postgres, MySQL and SQLite decisions
  and error text remain unchanged.
- With a custom engine hint, consider effective parsers whose `engine` matches
  case-insensitively and whose `sniff` returns true, in registration order.
- Without a hint, run the existing Postgres detection first. If it does not
  match, run custom sniffers in registration order.
- A throwing sniffer is treated as `false`; detection continues. Parsing is
  not attempted during sniffing.
- Because the historical unhinted JSON heuristic chooses Postgres, a custom
  JSON format should be parsed with an engine hint unless it can be
  distinguished before that heuristic in a future, separately reviewed
  change.

`explainEngineFromDriverName` retains the current built-in aliases first. It
then returns the canonical `engine` of the first registered parser whose
engine equals the trimmed driver name case-insensitively. Unknown names still
return `null`.

Built-in behavior must remain byte-for-byte compatible when no mutable parsers
are registered. Tests must cover raw dispatch through a custom parser,
replacement and its one warning, unregister and built-in restoration, the
exact unknown-format error, custom source detection with and without an engine
hint, a throwing sniffer, engine lookup, and all existing raw/source fixtures.

### 3.4 Import graph

The initial claim that `registry.ts` could import only `types.ts` while also
seeding built-ins was inconsistent. Use this acyclic graph instead:

```text
raw.ts ───────────────┐
parsers/source.ts ────┼──> registry.ts ──> parsers/builtins.ts
                      │                         │
                      │                         ├──> parsers/postgres.ts
                      │                         ├──> parsers/mysql.ts
                      │                         └──> parsers/sqlite.ts
                      └────────────────────────────> types.ts
```

`parsers/builtins.ts` owns row-payload JSON adapters now local to `raw.ts`.
Leaf parsers and `types.ts` must not import `raw.ts`, `source.ts` or the
registry. This graph has no cycle.

## 4. Raw plugin protocol (`SS-032`)

A plugin may return either its historical parsed-plan object or this raw
object from the JSON-RPC `explain_query` method:

```ts
interface PluginRawExplainOutput {
  engine: string;
  format: string;
  payload: string;
  original_query?: string | null;
}
```

`RpcDriver::explain_query` performs structural detection on the JSON value:

1. If `engine`, `format` and `payload` are all strings, construct
   `RawExplainOutput` and return `ExplainQueryOutput::Raw`.
2. Preserve a string `original_query`. If it is absent or `null`, fill it from
   the `query` argument supplied to the host.
3. If the three required strings identify a raw object but
   `original_query` is present with another type, return
   `Plugin raw EXPLAIN field 'original_query' must be a string or null`.
4. Additional fields are ignored.
5. If any required field is absent or is not a string, preserve the complete
   old fallback: return `ExplainQueryOutput::Plan { plan: res }` unchanged.

Detection is structural, not based on plan fields, XML contents or format
names. Tests must cover all branches, including a parsed plan that happens to
contain `engine` or `format` but not all three required strings.

Compatibility is intentionally asymmetric:

- old plugin plus new host remains a `Plan` and works unchanged;
- new plugin plus old host is wrapped as a plan object and cannot render;
- therefore `SS-035` must raise the plugin's runtime floor to the first
  Tabularis release containing both raw-plugin support and bundle loading.

`plugins/PLUGIN_GUIDE.md` documents both result shapes. This task does not add
SQL Server-specific knowledge to the Rust host.

## 5. Manifest contract (`SS-033` and `SS-034`)

The optional additive field is:

```json
"explain_parsers": [
  {
    "engine": "sqlserver",
    "format": "sqlserver-showplan-xml",
    "label": "SQL Server SHOWPLAN XML",
    "module": "explain/dist/index.iife.js"
  }
]
```

Each item requires non-empty string `engine`, `format` and `module`; `label` is
an optional non-empty string. Unknown item properties are rejected. A plugin
without the field behaves exactly as it does today.

`SS-033` adds this shape to all core surfaces that currently carry
`ui_extensions`:

- `plugins/manifest.schema.json` for the legacy/runtime manifest;
- `plugins/tabularium-extensions.schema.json` for the live merged registry
  schema used by `.tabularium`;
- Rust `ConfigManifest` and `PluginManifest`, including every constructor;
- frontend `PluginManifest` types.

`SS-034` adds the field to this plugin's `.tabularium` file. The IIFE filename
is intentionally different from the ESM package entry. Existing provisional
plugin tooling that copies `explain/dist/index.js` must be corrected in
`SS-034` to package `index.iife.js` as well as the npm files where appropriate.

The `module` value is passed to `read_plugin_file`. It is relative to the
installed plugin directory and must satisfy that command's existing path
rules. No network import or arbitrary absolute path is allowed.

## 6. Desktop bundle and loading (`SS-033` and `SS-034`)

### 6.1 Artifact convention

| Property | UI extension today | EXPLAIN parser contract |
| --- | --- | --- |
| Format | IIFE | IIFE |
| Output variable | `__tabularis_plugin__` | `__tabularis_explain_parser__` |
| External host API | `__TABULARIS_API__` | `__TABULARIS_EXPLAIN__` |
| Disk command | `read_plugin_file` | `read_plugin_file` |
| Evaluator | `PluginSlotProvider` | new `pluginExplainLoader.ts` |
| Trigger | enabled-plugin manifest effect | same enabled-plugin manifest effect |

The parser IIFE externalizes `@tabularis/explain` to
`__TABULARIS_EXPLAIN__`. The desktop passes the imported package namespace as
a `new Function` parameter, just as the UI loader passes React and the plugin
API. It returns the value assigned to `__tabularis_explain_parser__`.

Do not make one entry point both self-register and return a parser; that would
register it twice. The package uses separate thin entries over one parser:

```text
explain/src/showplan.ts  parser implementation
explain/src/parser.ts    parser descriptor
explain/src/index.ts     ESM: registers descriptor, exports direct API
explain/src/iife.ts      IIFE: default-exports descriptor, no registration
```

Build outputs are `dist/index.js`, `dist/index.d.ts` and
`dist/index.iife.js`.

### 6.2 Loader behavior

The IIFE default export may be one `RegisteredExplainParser` or an array. The
loader groups manifest entries by module so it reads and evaluates a file once.
For every manifest declaration it finds exactly one exported descriptor with
the same `engine` and `format`, validates non-empty strings and a callable
`parse`, applies the manifest label when supplied, and registers it. Undeclared
exports and invalid or missing matches are warned and skipped.

Plugin ids are processed in sorted order and manifest entries in manifest
order, making collision replacement deterministic. When the enabled set
changes, the provider unregisters formats loaded by its previous pass before
loading the new set. Built-in parsers reappear automatically because registry
unregistration removes only overlays.

Each module read/evaluation is isolated. If reading or evaluating a bundle
throws, log exactly this prefix with the error as a separate argument, skip
that module, and continue:

```text
[PluginExplain] Failed to load module "<module>" for plugin "<plugin-id>":
```

An invalid descriptor warns and skips only that descriptor. A parser's own
exception during a later parse is not swallowed; existing Visual EXPLAIN error
handling displays it. One broken plugin must not prevent other parser bundles
or built-ins from loading.

The current frontend trigger is the enabled-plugin manifest effect in
`PluginSlotProvider`, not a nonexistent JavaScript callback from Rust driver
registration. `SS-033` extends that lifecycle (or a sibling provider sharing
it) after `get_plugin_manifest` succeeds. Startup plugin loading completes in
Tauri setup before the frontend runs, and hot enable/install completes its
backend load before updating `activeExternalDrivers`, so this is the available
driver-load synchronization point.

## 7. npm package (`SS-034`)

The package is `@tabularis/explain-sqlserver`. Its version tracks the plugin
version. Its relevant metadata is:

```jsonc
{
  "name": "@tabularis/explain-sqlserver",
  "type": "module",
  "sideEffects": ["./dist/index.js"],
  "peerDependencies": {
    "@tabularis/explain": "^0.2.0"
  },
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "import": "./dist/index.js"
    }
  }
}
```

`@tabularis/explain` 0.2.0 is the first version with the registry. The XML
parser's chosen library is a normal runtime dependency of this package, not a
core dependency.

Both usage forms are supported:

```ts
import "@tabularis/explain-sqlserver";
import { parseShowplanXml } from "@tabularis/explain-sqlserver";
```

The first form must survive tree shaking, hence the explicit `sideEffects`
metadata. The package's ESM entry registers once on evaluation and exports the
parser and descriptor. The IIFE entry does not self-register.

Publishing is independent from the Rust release and is triggered by an
`explain-v*` tag. A Rust release does not force an npm publish, nor vice versa.

## 8. SQL Server parser semantics (`SS-034`)

The initial TypeScript port is behavior-compatible with
`src/driver/showplan.rs`:

- namespace-insensitive XML parsing and the first `RelOp` as root;
- direct child operators without crossing nested `RelOp` ownership;
- `PhysicalOp`, falling back to `LogicalOp`, then `Unknown`;
- ids prefixed with `sqlserver-`, including deterministic fallback ids;
- the first owned `Object@Table` with square brackets removed as `relation`;
- the first owned `ScalarOperator@ScalarString` as `filter`;
- logical operations containing `join` as `join_type` and in
  `extra.logical_operation`;
- `EstimateRows` as `plan_rows` and `EstimatedTotalSubtreeCost` directly as
  `total_cost`;
- sum per-thread `ActualRows` and `ActualExecutions`, maximum per-thread
  `ActualElapsedms`;
- root elapsed time as `execution_time_ms`, root actual rows deciding
  `has_analyze_data`, and the original XML as `raw_output`;
- `planning_time_ms`, startup costs, buffers, index/hash conditions and fields
  not listed above remain `null`;
- a multi-statement document uses its first `RelOp`, matching the Rust parser;
- malformed XML and a document without `RelOp` retain the current error
  prefixes.

There is no child-subtree cost subtraction and no `AvgRowSize` mapping in this
port. Missing-index data is not synthesized into the shared model; its fixture
proves that such a real document remains parseable and preserves raw output.
Any semantic expansion is a later, separately tested change.

Real SQL Server 2022 fixtures under `explain/tests/fixtures/` cover at least a
trivial scan, an index seek with key lookup, a parallel hash join with multiple
`RunTimeCountersPerThread` elements, `STATISTICS XML`, a missing-index
suggestion and a multi-statement batch. Fixtures are captured documents, not
hand-authored XML. Tests compare the TypeScript result with committed expected
plans and explicitly assert the aggregation and first-statement behavior.

The registered descriptor is:

```ts
{
  engine: "sqlserver",
  format: "sqlserver-showplan-xml",
  label: "SQL Server SHOWPLAN XML",
  parse: parseShowplanXml,
  sniff: (payload) => /<(?:\w+:)?ShowPlanXML(?:\s|>)/.test(payload.slice(0, 4096)),
}
```

The production parser still validates the full XML; sniffing is only a cheap
selection heuristic.

## 9. Plugin handoff and version floor (`SS-035`)

After core `SS-031` through `SS-033` and plugin `SS-034` are available, the
plugin returns:

```json
{
  "engine": "sqlserver",
  "format": "sqlserver-showplan-xml",
  "payload": "<ShowPlanXML ...>...</ShowPlanXML>",
  "original_query": "SELECT ..."
}
```

`src/driver/explain.rs` remains responsible only for safe SHOWPLAN capture and
session cleanup. `src/driver/showplan.rs` and its call from `ops.rs` are then
removed.

`min_runtime_version` and the prepared registry entry's
`min_tabularis_version` must name the first released Tabularis version that
contains all three core tasks. Do not guess that version before the core
release is assigned. This floor and the raw handoff land together.

## 10. Ordering and blast radius

```text
SS-030  freeze this contract
   │
   ├── SS-031  registry and open types              ─┐
   ├── SS-032  raw output from plugin drivers        ├─ core PR
   ├── SS-033  manifest plumbing and desktop loader ─┘
   │
   ├── SS-034  SQL Server parser package and IIFE   ─┐
   ├── SS-035  plugin returns raw SHOWPLAN XML      ─┘ plugin PR
   │
   └── SS-036  standalone site imports npm package     site PR
```

`SS-031` is behavior-preserving without mutable registrations. `SS-032` and
`SS-033` are inert for manifests without `explain_parsers`. `SS-035` is the
compatibility boundary and cannot land without the runtime floor and packaged
IIFE.

The seam is engine-neutral. Future first- or third-party plugins can ship their
own parser bundles and npm packages without adding engine code to Tabularis
core.
