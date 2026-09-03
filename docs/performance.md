# Performance baseline

This is a diagnostic baseline, not a performance target or guarantee. Results
will vary with SQL Server placement, TLS, schema size, query shape, and host
load. Repeat the same procedure when changing the TDS bridge, pool, runtime, or
worker topology.

## Environment and method

Measurements were taken on 2026-09-02 from release-candidate commit
`45a90b9`, using an optimized `cargo build --release` binary and a local Docker
container over `127.0.0.1`:

- Linux 5.4.0 x86-64, 8 logical CPUs, 31.2 GiB RAM;
- Rust 1.98.0;
- SQL Server 2022 Developer, 16.0.4265.3 (CU26);
- `mcr.microsoft.com/mssql/server:2022-latest`, image
  `sha256:90488d58c6a5c19f24ff716e14330b85b5b26ee54b44a36ea24e6206533e7edd`;
- TLS required with the development certificate trusted;
- default pool size 10, four JSON-RPC workers, and a 16 MiB Tokio thread stack.

The harness drove newline-delimited JSON-RPC over the plugin's real stdin and
stdout. Timings use a monotonic clock around complete request/response pairs.
RSS and virtual size came from `/proc/<pid>/status`. Cold latency is 20 fresh
processes. Steady latency is 100 sequential `SELECT CAST(1 AS INT)` calls after
one warm-up. Worker comparisons rebuild only `WORKER_POOL_SIZE`, warm the
available worker and pool paths with one request group, then report 20 groups
of eight simultaneous `get_tables` calls. Reported p95 values use the nearest
observed sample rather than interpolation.

## Results

### Query latency

| Measurement | Median | p95 |
| --- | ---: | ---: |
| First query with a cold process and pool | 15.07 ms | 18.80 ms |
| Steady-state query on a warm pool | 6.42 ms | 6.95 ms |

The steady path includes deadpool checkout and the session reset performed on
reuse. It is therefore a plugin round-trip baseline, not raw server execution
time.

### Concurrent metadata and worker count

| JSON-RPC workers | Eight `get_tables` calls median | p95 |
| ---: | ---: | ---: |
| 1 | 70.59 ms | 74.94 ms |
| 2 | 41.55 ms | 44.83 ms |
| **4 — default** | **26.45 ms** | **29.63 ms** |
| 8 | 19.21 ms | 21.02 ms |

Four workers reduced the median group time by 63% relative to one. Eight
workers improved another 27%, but can create twice as many simultaneous TDS
sessions and showed diminishing absolute returns. Four remains the default;
SQL connection concurrency remains independently configurable through
`max_pool_size`.

### Idle pools and eviction

A release process used 5,868 KiB RSS before opening a connection and 15,180
KiB after one idle connection in each of `master` and three scratch databases:
a 9,312 KiB high-water increase for four pools. A separate run measured 5,260
KiB from process start to one pool and roughly 1.3 MiB for each of the next
three. These figures include lazily touched runtime, TLS, and allocator pages,
so they are not a per-session allocation formula. Virtual size remained
829,648 KiB before and after opening pools; most of it is reserved address
space, including runtime stacks, rather than resident memory.

Eviction was checked with `pool_idle_eviction_minutes` set to 1 and a unique
`application_name`. Four pools were opened against four databases and released
back to deadpool. This server-side query initially returned 4 and returned 0
at 60.0 seconds without another plugin request:

```sql
SELECT COUNT(*)
FROM sys.dm_exec_sessions
WHERE program_name = N'Tabularis SS-045 eviction';
```

The cleanup path now explicitly closes every fully idle deadpool before
removing it from the cache. Checked-out pools survive that pass and are
considered again on the next interval.

### Pool identity

Unit and live tests verified all three key properties using pool identity and
SQL Server `@@SPID` values:

- repeated identical parameters reused the same pool and physical session;
- changing only `database` produced a different pool and session, even with
  the same `connection_id`;
- URL connection-string parameters reused the same session as equivalent
  discrete parameters.

Connection strings are resolved to canonical fields before key construction,
so syntax does not create duplicate pools.

### Large results

JSON-RPC returns one JSON line and cannot stream rows incrementally. The old
collector retained every row when `limit` was absent, allowing a million-row
query to grow until the process or host ran out of memory. The release
candidate applies a hard budget of 10,000 retained rows across a statement's
result sets. On the next
row it sets `truncated: true`, closes the remaining TDS stream, and keeps the
pooled session reusable. An explicit page limit cannot bypass this safety
budget. Result-bearing DML drains excess `OUTPUT` rows without retaining them
so it can still read the trailing affected-row sentinel.

The million-row live fixture now returned 10,000 rows with `truncated: true` in
379.10 ms. The response was 69,017 bytes and plugin RSS rose by 5,340 KiB at
peak from a warm baseline. A follow-up query on the same pool succeeded. The
ceiling bounds row accumulation; an individual SQL value can still be large,
so callers should continue to paginate and use the dedicated bounded BLOB
preview RPC where applicable.

### Queue backpressure and responsiveness

Both the request and response channels now hold at most 64 messages. Including
four active workers and the reader and writer payloads, at most 134 payloads
are queued or active inside the dispatcher. Bounding only the input queue was
insufficient: a slow stdout consumer could previously move all completed
responses into an unbounded output channel.

In the release measurement, a two-second `WAITFOR` followed by 200 `ping`
requests produced a ping response after 8.55 ms rather than waiting for the
slow query. All 201 responses arrived with matching ids. RSS rose from 11,588
to a sampled peak of 19,304 KiB while sessions and worker paths warmed, a
7,716 KiB increase. The live regression test repeats the 200-request burst;
the fixed channel capacities provide the actual memory backpressure rather
than relying on that one observed RSS value.

### Worker stack

The earlier debug probe overflowed Tokio's 2 MiB default stack and completed
at 4 MiB. For this task, the optimized binary was rebuilt with a 4 MiB stack
and the full 25-test live suite passed in 21.88 seconds, including type
extraction, metadata, DDL, errors, EXPLAIN, BLOBs, million-row cancellation,
and concurrent requests. This establishes that 4 MiB worked for this Linux
release build; it does not prove safety for debug binaries or every target.

The configured 16 MiB stack is retained as a four-times margin over the
smallest observed successful size. Tokio reserves this virtual address range
per runtime thread but commits resident pages on demand. Reduce it only after
the preview TDS client changes or equivalent debug and release stress coverage
is green on all release platforms.

## Reproduction checks

The committed automated checks are:

```bash
cargo test --bins pool_manager::tests -- --test-threads=1
SQLSERVER_PLUGIN_BIN="$PWD/target/debug/sqlserver-plugin" \
  cargo test --test live_db \
  million_row_query_is_bounded_and_marks_truncation -- --test-threads=1
SQLSERVER_PLUGIN_BIN="$PWD/target/debug/sqlserver-plugin" \
  cargo test --test live_db \
  request_burst_is_bounded_and_slow_query_does_not_block_ping \
  -- --test-threads=1
SQLSERVER_PLUGIN_BIN="$PWD/target/debug/sqlserver-plugin" \
  cargo test --test live_db \
  pool_keys_reuse_identical_and_equivalent_forms_but_separate_databases \
  -- --test-threads=1
```

The one-minute DMV eviction timing and worker-count comparison are intentionally
recorded measurements rather than always-on CI tests. Making every CI run wait
for a wall-clock eviction interval would add a minute while testing timer
accuracy more than cleanup behavior.
