# rapsqlite Roadmap

This roadmap describes the release-oriented development plan for `rapsqlite`, a true async SQLite library for Python built with Rust, Tokio, and SQLx.

The roadmap uses minor `0.x` releases as delivery phases. The latest release tag is `v0.4.0`; the 0.5 implementation is complete and is in release validation.

## Current Status

**Latest tag:** `v0.4.0` ✅
**Current development phase:** `0.5` — Low-latency execution and session affinity ✅ implementation complete; release validation
**Next performance phase:** `0.6` — Cache APIs, batching, and concurrent workloads 📋

## Completed Release Phases

### 0.1 — Core functionality ✅

- Connection lifecycle and async context managers
- Transactions and transaction context managers
- Python/SQLite type handling
- aiosqlite-compatible exceptions and core API
- Basic connection pooling and timeouts
- Input validation and security improvements
- Type stubs and initial documentation

### 0.2 — Feature-complete drop-in foundation ✅

- Named and positional parameters
- Cursor state management and result caching
- Connection and pool configuration
- Row factories and transaction helpers
- SQLite callbacks and initialization hooks
- Backup, dump, and schema introspection APIs
- Prepared-statement caching documentation and benchmarks

### 0.3 — Advanced features and aiosqlite parity ✅

- aiosqlite-compatible helper methods and cursor properties
- Savepoints, transaction timeouts, retries, and slow-query callbacks
- True Async DBAPI and the `sqlite+rapsqlite` SQLAlchemy dialect
- Alembic, FastAPI, Starlette, and aiohttp integration support
- FTS5 and JSON1 support
- Type adapters, converters, aggregates, and collations
- Pool metrics, health checks, idle timeouts, and Prometheus gauges
- Interrupt handling and callback-backed query support

## 0.4 — Compatibility, security, and release stabilization ✅

**Goal:** Consolidate all work completed after `v0.3.3` and produce the next compatible minor release.

### Completed since `v0.3.3`

- ✅ Correct SQL parameter parsing and error reporting (#22)
- ✅ Shared pool identity and lifecycle fixes (#23)
- ✅ Cursor, row, and SQLAlchemy result compatibility fixes (#24)
- ✅ Query-helper bounds and transaction-retry fixes (#25)
- ✅ Restorable `iterdump()` output (#26)
- ✅ Sensitive INSERT-value redaction, including comment-aware handling (#27)
- ✅ Complete SQLite `$` parameter support (#28)
- ✅ Immediate cleanup of transaction callback handles (#29)
- ✅ Active callback-query interrupt and handle synchronization (#30)
- ✅ Raw transaction cleanup and callback lifecycle hardening
- ✅ CI restoration with secure PyO3 configuration
- ✅ Ruff version pinning and current formatting of Markdown examples
- ✅ SQLAlchemy 2.1 async bridge support and typed await compatibility
- ✅ Matched `rapsqlite` versus `redis.asyncio` cache benchmark, with release-build results and methodology documented in `benchmarks/`

### 0.4 release result

- ✅ Complete release test matrix on supported Python and platform combinations
- ✅ Review compatibility documentation and release notes against the `v0.3.3..HEAD` change set
- ✅ Publish `v0.4.0` and its platform wheels after release validation

## 0.5 — Low-latency execution and session affinity ✅

**Goal:** Reduce fixed per-operation overhead for repeated, single-statement local queries—especially cache lookups—while preserving the general DB-API path.

This phase is about making one operation cheaper. It does not attempt to make SQLite a shared cache service or replace the batching work planned for 0.6.

### Scope and non-goals

In scope:

- Remove unconditional work from the normal query path.
- Preserve physical-connection and prepared-statement affinity where an application opts into it.
- Add narrow scalar/BLOB and reusable-query paths for repeated operations.
- Prototype a raw SQLite path only behind an explicit opt-in until cancellation and event-loop behavior are proven.

Out of scope for 0.5:

- Cache-specific `get`/`set` semantics and TTL APIs (#42).
- Bulk operations and pipelining-style batching (#43).
- Multiplexed concurrent reads (#44).
- A general pool redesign or a claim that rapsqlite is faster than Redis for every workload.

### 0.5 implementation result

The planned implementation work is complete. The release candidate includes:

- ✅ A release-build hot-path benchmark covering generic rows, scalar/BLOB paths, prepared-query dispatch, session affinity, synchronous `sqlite3`, and optional local `redis.asyncio`; one same-machine run is recorded in [`benchmarks/phase5_hotpath_results.md`](../benchmarks/phase5_hotpath_results.md)
- ✅ One-time PRAGMA application per physical connection, opt-in query-usage tracking, atomic callback/feature presence checks, and fast common parameter conversion
- ✅ Scalar and BLOB fetch APIs with documented behavior for misses, `NULL`, transactions, callbacks, and unsupported row factories
- ✅ Opt-in session affinity with pool-capacity, transaction, close/reopen, and shared-memory behavior covered by tests
- ✅ Connection-bound prepared query objects for repeated normal and raw scalar/BLOB operations
- ✅ An opt-in raw scalar SQLite path with transaction routing, error mapping, callback restrictions, and `Connection.interrupt()` cancellation support
- ✅ Performance guidance and API documentation that distinguish measured local results from unmatched published Redis figures
- ✅ Focused Phase 0.5 tests and full compatibility validation

### Delivery order

The issues are intentionally ordered by risk and dependency. The first workstream should establish the measurement baseline; the remaining workstreams can then be delivered independently where their prerequisites are satisfied.

#### 0.5.1 — Establish the performance baseline ✅

- Add a release-build benchmark harness covering the current generic path, synchronous `sqlite3`, each optimized candidate, and local `redis.asyncio` where available.
- Separate individual-call latency from aggregate throughput. Run sequential calls and concurrency levels such as 1, 8, and 32.
- Record p50/p95/p99 latency, throughput, event-loop delay, operation errors, and peak process RSS for representative key/value sizes; cancellation correctness remains covered by focused tests.
- Keep benchmark inputs and output stable enough for repeatable local and release-validation comparisons. Automated performance thresholds remain deferred until CI hardware and variance are controlled.

#### 0.5.2 — Remove unconditional hot-path overhead ✅

Deliver these low-risk changes before introducing new execution APIs:

- [#41](https://github.com/eddiethedean/rapsqlite/issues/41) Apply connection PRAGMAs once per physical SQLite connection.
- [#46](https://github.com/eddiethedean/rapsqlite/issues/46) Make query usage tracking and SQL normalization opt-in.
- [#36](https://github.com/eddiethedean/rapsqlite/issues/36) Collapse no-op feature checks on the query hot path.
- [#45](https://github.com/eddiethedean/rapsqlite/issues/45) Optimize common Python parameter conversion paths.

These changes must leave callback, adapter, converter, transaction, close, and `set_pragma()` behavior unchanged. Diagnostic features remain available when explicitly enabled.

#### 0.5.3 — Avoid general result construction for narrow results ✅

- [#39](https://github.com/eddiethedean/rapsqlite/issues/39) Add scalar and BLOB fetch fast paths.

Define and test the behavior for no rows, SQL `NULL`, non-scalar queries, row factories, transactions, and all supported scalar types. The existing DB-API-compatible row path remains the compatibility default.

#### 0.5.4 — Retain execution affinity when requested ✅

- [#38](https://github.com/eddiethedean/rapsqlite/issues/38) Retain per-Connection SQLite sessions for low-latency workloads.

Make the resource trade-off explicit: a retained session consumes pool capacity. Test sequential reuse, serialized concurrent use of one logical connection, transactions, close/reopen, pool exhaustion, connection replacement, and shared named in-memory databases.

#### 0.5.5 — Reuse statement and dispatch metadata ✅

- [#37](https://github.com/eddiethedean/rapsqlite/issues/37) Add prepared query objects for repeated async operations.

Prepared objects must be connection/session-aware and define behavior for pooled connections, transaction routing, invalidation, and close/reopen. They should reduce repeated SQL and dispatch setup without changing one-shot query behavior.

#### 0.5.6 — Evaluate the narrow raw SQLite path ✅

- [#35](https://github.com/eddiethedean/rapsqlite/issues/35) Add an opt-in raw SQLite low-latency execution path.

Start with one prepared statement and zero/one scalar result. Reuse existing handle locking and transaction routing. Do not make this path the default until correctness, cancellation, `sqlite3_interrupt`, error mapping, connection lifetime, and event-loop delay are demonstrated.

### 0.5 release criteria

- Every completed workstream has focused correctness tests, documentation, and before/after release-build measurements.
- The default compatibility path preserves callback, adapter, converter, row-factory, transaction, cancellation, connection-close, and error semantics.
- No optimization causes a statistically meaningful regression in representative one-shot DB-API operations or existing CI tests.
- Benchmarks report p50/p95/p99 latency, throughput, event-loop delay, lock/busy errors, cancellation behavior, and memory use at concurrency 1, 8, and 32.
- The benchmark report separates SQLite execution, pool/session acquisition, Python/Rust conversion, and async scheduling overhead where measurable.
- Opt-in paths document their resource and behavioral trade-offs, including retained pool capacity and raw-handle restrictions.
- Results are compared on the same machine and payloads for current rapsqlite, optimized rapsqlite, synchronous `sqlite3`, and local Redis; unmatched published figures are not used as speedup claims.
- 0.6 work remains API-compatible with the 0.5 foundation and is not pulled into this release merely to improve batch throughput.

## 0.6 — Cache APIs, batching, and concurrent workloads 📋

**Goal:** Provide explicit APIs for cache workloads and improve aggregate throughput without requiring one async call per item.

- ✅ [#42](https://github.com/eddiethedean/rapsqlite/issues/42) Cache-specific low-latency `get`/`set` operations with TTL handling
- [#43](https://github.com/eddiethedean/rapsqlite/issues/43) Bulk cache APIs and expiration cleanup batches
- [#44](https://github.com/eddiethedean/rapsqlite/issues/44) Multiplexed read mode for concurrent cache workloads
- Batched write and read benchmarks across representative payload sizes
- Documented transaction, miss, TTL, atomicity, and locking semantics for cache APIs

### 0.6 release criteria

- Compare sequential operations and batches separately
- Benchmark batch sizes from small request groups through large pipelines
- Measure throughput, p95/p99 latency, event-loop delay, lock errors, cancellation, and memory use
- Demonstrate behavior under mixed reads and writes rather than warm reads alone

## 0.7 — Pooling, observability, and reliability 📋

**Goal:** Improve operational control and make production behavior easier to diagnose.

- Dynamic pool sizing and connection routing strategies
- Read/write connection separation where SQLite semantics permit it
- Failover and recovery patterns for file-backed databases
- Connection state tracking and diagnostics
- Transaction tracing and long-running transaction monitoring
- Deadlock/lock detection and automatic retry policies
- Query profiling, resource-usage tracking, and execution visualization
- Stress testing and performance-regression gates in CI
- Cross-platform validation across Linux, macOS, and Windows
- Continued improvement of the aiosqlite compatibility suite, documenting intentional differences

## 0.8 — Type, framework, and database tooling 📋

**Goal:** Expand the ecosystem around the stable async core.

- Date/time, UUID, and Decimal type utilities
- Database introspection CLI
- Migration generation utilities
- Database mocking and testing helpers
- Tortoise ORM, Peewee, Django, Quart, and Sanic integration patterns
- Enhanced backup and restore utilities
- Schema validation tools
- Custom SQLite extension support
- Window-function and CTE helper utilities

## 0.9 — Stabilization toward 1.0 📋

**Goal:** Prepare a stable API and operational baseline for a future `1.0.0` release.

- Security and API-stability review
- Complete supported-platform and supported-Python validation
- Final compatibility audit against aiosqlite and SQLAlchemy
- Long-running production and failure-mode testing
- Finalize deprecations, migration guidance, and support policy
- Evaluate database encryption and multi-database transaction support

## Open Issue Allocation

All currently open GitHub issues are assigned to a future release phase:

| Release | Issues |
| --- | --- |
| 0.5 | #35, #36, #37, #38, #39, #41, #45, #46 |
| 0.6 | #42, #43, #44 |

## Versioning Strategy

| Version | Focus | Status |
| --- | --- | --- |
| 0.1.x | Core functionality | ✅ Complete |
| 0.2.x | Feature-complete drop-in foundation | ✅ Complete |
| 0.3.x | Advanced features and aiosqlite parity | ✅ Complete; latest pre-0.4 tag `v0.3.3` |
| 0.4.x | Post-0.3.3 compatibility, security, and release stabilization | ✅ Complete; latest tag `v0.4.0` |
| 0.5.x | Low-latency execution and session affinity | ✅ Implementation complete; release validation |
| 0.6.x | Cache APIs, batching, and concurrent workloads | 📋 Planned |
| 0.7.x | Pooling, observability, and reliability | 📋 Planned |
| 0.8.x | Type, framework, and database tooling | 📋 Planned |
| 0.9.x | Stabilization toward 1.0 | 📋 Planned |

`1.0.0` will follow the 0.9 stabilization phase once the public API, compatibility guarantees, and production support policy are ready.
