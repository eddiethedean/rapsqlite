# rapsqlite Roadmap

This roadmap describes the release-oriented development plan for `rapsqlite`, a true async SQLite library for Python built with Rust, Tokio, and SQLx.

The roadmap uses minor `0.x` releases as delivery phases. The latest release tag is `v0.3.3`; `0.4` is the next release phase.

## Current Status

**Latest tag:** `v0.3.3` ✅
**Current development phase:** `0.4` — Compatibility, security, and release stabilization 🔄
**Next performance phases:** `0.5` and `0.6` 📋

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

## 0.4 — Compatibility, security, and release stabilization 🔄

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

### 0.4 release work

- ⏳ Run the complete release test matrix on supported Python and platform combinations
- ⏳ Review compatibility documentation and release notes against the `v0.3.3..HEAD` change set
- ⏳ Publish the `v0.4.0` release after the post-tag changes are validated

## 0.5 — Low-latency execution and session affinity 📋

**Goal:** Reduce fixed per-operation overhead for repeated local queries and cache lookups while preserving the general DB-API path.

- [#35](https://github.com/eddiethedean/rapsqlite/issues/35) Opt-in raw SQLite low-latency execution path
- [#36](https://github.com/eddiethedean/rapsqlite/issues/36) Collapse no-op feature checks on the query hot path
- [#37](https://github.com/eddiethedean/rapsqlite/issues/37) Reusable prepared query objects
- [#38](https://github.com/eddiethedean/rapsqlite/issues/38) Retain per-Connection SQLite sessions
- [#39](https://github.com/eddiethedean/rapsqlite/issues/39) Scalar and BLOB fetch fast paths
- [#41](https://github.com/eddiethedean/rapsqlite/issues/41) Apply connection PRAGMAs once per physical connection
- [#45](https://github.com/eddiethedean/rapsqlite/issues/45) Optimize common Python parameter conversion paths
- [#46](https://github.com/eddiethedean/rapsqlite/issues/46) Make query usage tracking and SQL normalization opt-in

### 0.5 release criteria

- Preserve callback, transaction, cancellation, and connection-close semantics
- Report p50/p95/p99 latency, throughput, and event-loop delay
- Benchmark generic SQLx execution, optimized paths, synchronous `sqlite3`, and local Redis on the same machine
- Keep all optimizations opt-in when their behavior or resource trade-offs differ from the compatibility path

## 0.6 — Cache APIs, batching, and concurrent workloads 📋

**Goal:** Provide explicit APIs for cache workloads and improve aggregate throughput without requiring one async call per item.

- [#42](https://github.com/eddiethedean/rapsqlite/issues/42) Cache-specific low-latency `get`/`set` operations with TTL handling
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
| 0.3.x | Advanced features and aiosqlite parity | ✅ Complete; latest tag `v0.3.3` |
| 0.4.x | Post-0.3.3 compatibility, security, and release stabilization | 🔄 In progress |
| 0.5.x | Low-latency execution and session affinity | 📋 Planned |
| 0.6.x | Cache APIs, batching, and concurrent workloads | 📋 Planned |
| 0.7.x | Pooling, observability, and reliability | 📋 Planned |
| 0.8.x | Type, framework, and database tooling | 📋 Planned |
| 0.9.x | Stabilization toward 1.0 | 📋 Planned |

`1.0.0` will follow the 0.9 stabilization phase once the public API, compatibility guarantees, and production support policy are ready.
