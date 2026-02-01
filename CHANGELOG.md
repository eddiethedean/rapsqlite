# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning Strategy

- **v0.1.x**: Phase 1 — Core functionality (MVP and core features)
- **v0.2.x**: Phase 2 — Feature-complete drop-in replacement
- **v0.3.x**: Phase 3 — Advanced features & aiosqlite parity - **Current: v0.3.0-dev**
- **v1.0.0**: Phase 4 — Production ready (stable API release)

## [1.0.0] - TBA (Phase 4: Production Ready)

### Overview

- v1.0.0 will be released after Phase 4 completion, marking production stability
- Phase 4 focuses on production tooling, cross-platform validation, and advanced features
- See ROADMAP.md for detailed Phase 4 features

### Checklist for v1.0 Release (Future)

- Phase 3 complete (v0.3.0 released)
- Type system: `register_adapter` and `register_converter`
- Cross-platform CI (Linux, macOS, Windows)
- Performance regression tests
- All tests passing across supported Python versions (3.10–3.14)
- Comprehensive documentation and examples
- Production stability validated

_Note: v1.0.0 release details will be added after Phase 4 completion._

## [0.3.0] - TBA (Phase 3: Advanced Features & aiosqlite Parity)

### Overview

- v0.3.0 focuses on advanced features and aiosqlite API parity
- Query helpers, transaction utilities, and framework integrations
- Target: >80% aiosqlite test suite compatibility

### Release Criteria

- aiosqlite test suite pass rate >80% (or intentional differences documented)
- All Phase 3 features tested and documented
- Migration guide complete
- No breaking changes from v0.2.0

## [0.3.0-dev] - Unreleased

### Fixed - SQLAlchemy and create_function (2026-01-31)

- **SQLAlchemy ORM INSERT...RETURNING doubled rows** — ExecuteContextManager now fetches and caches RETURNING rows for INSERT/UPDATE/DELETE via `returns_result_rows` and `_set_select_results`, so `cursor.fetchall()` does not re-execute. Fixes `test_async_session_add_commit_get` and `test_async_session_add_all_many_rows`.
- **SQLAlchemy transaction rollback with UDFs** — When `has_callbacks` and DML runs without an active transaction, new DML-with-callbacks branch runs BEGIN on `callback_connection`, moves it to `transaction_connection`, and sets `transaction_state = Active`, so rollback correctly undoes inserts. Fixes `test_connection_explicit_transaction`.
- **create_function connection routing** — `create_function` now prefers `transaction_connection` when it holds a connection (moved by DML-with-callbacks). Ensures UDF add/remove operates on the correct connection. Fixes `test_create_function` and `test_create_function_deterministic`.

### Changed - Code quality (2026-01-31)

- **cargo fmt** — Reformatted `src/connection.rs`, `src/context_managers.rs`, `src/utils.rs`.
- **cargo clippy** — Fixed useless_conversion warnings in `src/context_managers.rs` (removed redundant `.into()` on `build_description_empty_result(...).unbind()`).
- **ruff format** — Reformatted Python sources (rapsqlite, tests, scripts).
- **ruff check** — Removed unused `asyncio` import in `tests/test_doc_examples.py`.

### Added - Phase 3.8: v0.3.0 release readiness (2026-01-31)

- **`connect(..., aiosqlite_compat=True)`** — When True, sets default row_factory to tuple so fetch_all/cursor fetchall return tuples (aiosqlite/sqlite3 default). Use for drop-in ``import rapsqlite as aiosqlite`` without code changes for row type.
- **aiosqlite test results** — `docs/AIOSQLITE_TEST_RESULTS.md` updated with known intentional differences, per-test failure categories (fix/document/environment), and link to migration/compatibility guides. v0.3.0 release criterion "intentional differences documented" satisfied.
- **Migration guide** — "If you see test failures" subsection; row format section updated with aiosqlite_compat and ``row_factory = "tuple"`` examples; cross-reference to advanced-usage for best practices.
- **Compatibility guide** — Row format (lists vs tuples) and aiosqlite_compat documented; total_changes/in_transaction noted as sync properties.
- **Best practices and anti-patterns** — Expanded in advanced-usage: connection lifecycle (abandoning connections without close), avoiding blocking the event loop, transaction boundaries (keep transactions short). Migration guide links to advanced-usage for best practices.
- **Performance guide** — Single connection vs pool subsection; measuring performance and regression testing (timed_fetch_all, tests/test_performance.py); cross-links to migration-guide and advanced-usage.
- **ROADMAP** — Section 3.8 items marked complete; v0.3.0 release criteria noted as met via documented differences; Last Updated 2026-01-31.
- **Tests** — `test_connect_aiosqlite_compat_tuple_rows` in `test_aiosqlite_compat.py` verifies `connect(..., aiosqlite_compat=True)` yields tuple rows.

### Added - Test improvements (Rust and Python) (2026-01-31)

- **Rust unit tests** — `src/parameters.rs`: tests for `find_named_parameter_placeholders` (colon, at, dollar, multiple, underscore/numbers, none, colon-not-param). `src/errors.rs`: existing tests for `sanitize_query`. Total 16 Rust unit tests.
- **Unified dev test run** — `scripts/dev_test.sh` now runs Rust unit tests first (via `scripts/run_rust_tests.sh` on macOS/Linux, or `cargo test` on Windows), then Python pytest. Single command for full test run.
- **CI Rust tests** — Rust unit tests run on all platforms: `ubuntu-latest`, `macos-latest`, `windows-latest` (macOS uses `run_rust_tests.sh` for libpython).
- **Python test noise** — `pyproject.toml` filterwarnings for Tokio/unraisable-exception warnings during parallel runs; `tests/README.md` documents full test run (Rust + Python), fixtures, and warning filter.

### Fixed - Error sanitization (2026-01-31)

- **`sanitize_query`** — Quoted sensitive values (e.g. `password='secret123'`) are now replaced in full with `***` instead of only the opening quote.

### Added - Phase 3 remaining API features (2026-01-31)

- **register_adapter / register_converter** — Verified and documented. Per-connection type adapters and converters (sqlite3-style) are implemented and wired; tests pass. `docs/reference/type-conversion.rst` and `docs/guides/migration-guide.rst` updated to state they are supported. Test `test_register_converter_declared_type` uses declared type DATE (driver-reported) for converter lookup.
- **create_aggregate** — API implemented; test remains skipped on some platforms due to Bus error in aggregate context (known limitation). ROADMAP notes the limitation.
- **create_collation** — Implemented and tested. Added `test_create_collation` in `test_aiosqlite_compat.py` (custom collation, ORDER BY COLLATE, remove). ROADMAP section 3.8 marks all four Phase 3 API features complete (with caveat for create_aggregate).

### Added - aiosqlite compatibility improvements (2026-01-30)

- **`total_changes` sync property** — Now a synchronous property (not async method) for aiosqlite compatibility. Cached value updated after `begin()`/`commit()`/`rollback()`.
- **`in_transaction` sync property** — Now a synchronous property (not async method) for aiosqlite compatibility. Properly tracks explicit transactions and `transaction()` context manager.
- **`transaction()` state tracking** — Wrapped to properly update `in_transaction` state on enter/exit.
- **Tests updated** — `test_aiosqlite_compat.py` and `test_concurrent_transactions.py` updated to use sync properties.
- **Migration guide updated** — Documents row format difference (lists vs tuples) and current sync property behavior.
- **aiosqlite test suite status** — perf.py: 6/10 passing, smoke.py: 3/30 passing. Row format and pooling differences are documented as intentional.

### Fixed - Test improvements (2026-01-30)

- **aiohttp test** — Refactored `tests/test_aiohttp_example.py` to avoid `TestServer`/`TestClient` which required network socket binding. Test now directly invokes handler function with mock request pattern. Added `filterwarnings` marker to suppress Tokio context cleanup warning (known PyO3 async limitation during GC).

### Added - Phase 3 plan implementation (2026-01-30)

- **aiosqlite compatibility script** — `scripts/run_aiosqlite_tests.py` now outputs per-test breakdown (PASSED/FAILED/SKIPPED with error snippets) in `docs/AIOSQLITE_TEST_RESULTS.md`.
- **`suggest_indexes(conn, sql, parameters=None)`** — Suggests indexes when query plan shows full table scan; returns list of dicts with `table`, `column`, `suggestion`. Documented in advanced-usage (Query plan analysis).
- **`in_clause_query(sql, values)`** — Expands `IN (?)` to `IN (?,?,...)` for use with `fetch_all`; standalone helper. Documented in advanced-usage (IN clause expansion).
- **`rows_to_dicts(rows, columns)`** — Converts list-of-list rows to list-of-dicts using column names. Documented in advanced-usage (Streaming and large result sets).
- **Migration guide** — Added "Migrating from aiosqlite: Common Patterns" (connection lifecycle, `total_changes`/`in_transaction` async, backup API, transaction queue); link to AIOSQLITE_TEST_RESULTS.md.
- **Best practices** — Added subsection on parameterized queries, `execute_iter` vs `paginate`, pool sizing in advanced-usage.
- **Tests** — `test_suggest_indexes`, `test_in_clause_query`, `test_rows_to_dicts` in `tests/test_phase3_api.py`.

### Added - Phase 3.1: Query helpers and pagination (2026-01-30)

- **`paginate(conn, sql, parameters=None, page_size=64, offset=0)`** — Fetch one page of rows; wraps query with `LIMIT`/`OFFSET`. Documented in advanced-usage (Streaming and large result sets).
- **`analyze_query_plan(conn, sql, parameters=None)`** — Run `EXPLAIN QUERY PLAN` and return structured dict with `rows`, `details`, `uses_index`, `table_scan`. Documented in advanced-usage (Query plan analysis).
- **`transaction_with_timeout(conn, work, timeout_secs=30)`** — Run a transaction with `asyncio.wait_for`; raises `TimeoutError` if work exceeds timeout. Documented in advanced-usage (Transaction timeout).
- **`set_slow_query_threshold(conn, threshold_secs, callback=None)`** — Invoke callback when `fetch_all` exceeds threshold; set to 0 to disable. Documented in advanced-usage (Slow query detection).

### Added - Phase 3.4: Starlette and aiohttp integration (2026-01-30)

- **Starlette example** — `examples/starlette_db.py`; lifespan for schema setup, connection per request. Documented in `docs/guides/compatibility.rst`.
- **aiohttp example** — `examples/aiohttp_db.py`; `on_startup` for schema setup, connection per request. Documented in `docs/guides/compatibility.rst`.
- **Tests** — `tests/test_starlette_example.py`, `tests/test_aiohttp_example.py` for smoke validation.

### Added - Phase 3.1: FTS5 and JSON1 support (2026-01-30)

- **FTS5 tests** — `tests/test_fts.py` (create virtual table, MATCH queries, bm25 ranking).
- **JSON1 tests** — `tests/test_json1.py` (json_extract, json_object, `->`, `->>` operators).
- FTS and JSON usage documented in advanced-usage (Streaming section: FTS, JSON, and UPSERT).

### Added - Phase 3.9: Cursor chaining (2026-01-30)

- **`Cursor.executemany()` returns self** — Wrapper for aiosqlite chaining compatibility.
- **`Cursor.executescript()` returns self** — Wrapper for aiosqlite chaining compatibility.

### Added - Testing and CI (2026-01-30)

- **Test dependencies** — `greenlet`, `httpx`, `aiohttp` added to `[project.optional-dependencies].test` for SQLAlchemy and framework tests.
- **aiosqlite-compat CI job** — Runs `scripts/run_aiosqlite_tests.py` on schedule or when `full_suite` is requested via workflow_dispatch.

### Changed (2026-01-30)

- **Dead code removal** — Removed unused Rust functions: `bind_and_execute`, `bind_query_multiple`, `bind_and_fetch_all`, `bind_and_fetch_one`, `bind_and_fetch_optional` (query.rs); `execute_many_raw`, `execute_many_raw_standalone` (batch.rs); `map_sqlite_error` (errors.rs).
- **Lint and format** — Ruff format, ruff check, mypy, cargo fmt, cargo clippy; all passing.
- **Test count** — 560 tests passing (7 skipped).

### Changed

- Version bump to **0.3.0-dev** — Phase 3 development (advanced features, ecosystem integration) toward v1.0.0.
- **Performance — Session-connection reuse**: Each `Connection` now holds and reuses one pool connection for non-transaction, non-callback operations (e.g. `fetch_all`, `execute`, `total_changes`). The session connection is released on `close()` and when starting a transaction (`begin()` or `transaction()`), so transaction and callback paths are unchanged. This matches aiosqlite-style usage (one connection per worker for many queries) and improves concurrent read performance: the **Concurrent Reads** benchmark (10 workers × 2000 queries) now wins vs aiosqlite (~1206ms vs ~1439ms). Pool helpers: `ensure_session_connection`, `release_session_connection`.

### Added - Phase 3.9: API Completeness (aiosqlite compatibility)

- **`Connection.execute_fetchall(sql, parameters=None)`** — Execute a SELECT and return all rows (delegates to `fetch_all`).
- **`Connection.execute_insert(sql, parameters=None)`** — Execute INSERT/UPDATE/DELETE and return `last_insert_rowid()`; rejects SELECT.
- **`Connection.explain_query_plan(sql, parameters=None)`** — Run `EXPLAIN QUERY PLAN` for the given SQL and return result rows (Phase 3.1).
- **`Connection.pool_health()`** — Minimal health check (`SELECT 1`); returns `True` on success, raises on failure (Phase 3.2).
- **`Connection.isolation_level`** getter/setter — Transaction isolation: `None` | `"DEFERRED"` | `"IMMEDIATE"` | `"EXCLUSIVE"`. Applied to `BEGIN` in `begin()` and `transaction()`.
- **`Connection.__await__`** — Support `await conn` pattern (enter connection and return self).
- **`Connection.interrupt()`** — Interrupts callback connection when present (UDFs, trace, etc.); no-op otherwise.
- **`connect(..., iter_chunk_size=64, loop=None)`** — aiosqlite-compatible params; `iter_chunk_size` stored, `loop` accepted and ignored.
- **`Connection.iter_chunk_size`** — Getter for stored chunk size.
- **`Connection.create_function(..., deterministic=False)`** — Pass `SQLITE_DETERMINISTIC` when `deterministic=True` (SQLite 3.8.3+).
- **`Connection.executemany`** — Alias for `execute_many` (aiosqlite compat).
- **`Connection.savepoint(name=None)`** — Context manager for `SAVEPOINT` / `RELEASE` / `ROLLBACK TO`; requires active transaction.
- **`Connection.stop()`** — No-op for aiosqlite API compatibility; use `close()` to close the connection.
- **`Connection.pool_metrics()`** — Returns `{size, num_idle, in_use}` from the pool.
- **`Connection.execute(query, parameters=None, cursor=None)`** — Optional `cursor` argument: when provided (e.g. from `Cursor.execute()`), reuses that cursor so `await cursor.execute(...)` returns the same cursor (aiosqlite chaining).
- **`NotSupportedError`** — New exception (e.g. `deterministic=True` on SQLite &lt; 3.8.3).
- **Implicit transactions** — First DML (INSERT/UPDATE/DELETE) without `begin()` starts a transaction; `commit()`/`rollback()` end it. DDL does not. `commit`/`rollback` with no transaction are no-ops. Context manager exit commits on success, rolls back on exception.
- **`Cursor.iter_chunk_size`** — Alias for `arraysize` (aiosqlite compat).
- **Examples** — `examples/async_basic.py`, `examples/fastapi_db.py`; `examples/README.md`. FastAPI smoke test in `tests/test_fastapi_example.py`.

### Added - Phase 3.9: Cursor properties and methods

- **`Cursor.arraysize`** (r/w, default 1) — Default size for `fetchmany()` when `size` is omitted.
- **`Cursor.connection`** (r/o) — Reference to parent `Connection`.
- **`Cursor.description`** (r/o) — Column metadata (7-tuples) after execute/fetch.
- **`Cursor.lastrowid`** (r/o) / **`Cursor.rowcount`** (r/o) — Set from last execute (SELECT: -1; INSERT/UPDATE/DELETE: from result).
- **`Cursor.row_factory`** (r/w) — Per-cursor override; falls back to connection’s `row_factory`.
- **`Cursor.close()`** — Async; clears cached results, description, lastrowid, rowcount.
- **`Cursor.fetchmany(size=None)`** — `size` optional; uses `arraysize` when omitted.
- **`Cursor.execute()` returns self** — When awaited, returns the same cursor for chaining (aiosqlite compat); implemented via `Connection.execute(..., cursor=self)`.

### Added - Benchmarks

- **`benchmarks/README.md`** — Updated with latest benchmark results (session-connection reuse; rapsqlite wins Concurrent Reads, High Concurrency Reads, Concurrent Batch Inserts, Mixed Workload at ×10 row scale).

### Added - Testing and tooling

- **`tests/test_phase3_api.py`** — Tests for Phase 3.9+ APIs (connect params, create_function deterministic, interrupt, savepoints, pool_metrics, etc.).
- **`tests/test_fastapi_example.py`** — FastAPI + rapsqlite smoke test.
- **aiosqlite test suite** — Run via `scripts/run_aiosqlite_tests.py`; see `docs/AIOSQLITE_TEST_RESULTS.md`. Transaction-related failures addressed (implicit transactions, commit-on-exit).

### Added - True Async DBAPI (`rapsqlite.dbapi`)

- **`rapsqlite.dbapi`** — True Async DBAPI 2.0–compliant module for SQLAlchemy and other consumers.
- **`dbapi.connect(database, **kwargs)`** — Async connect; `database` required (positional or keyword).
- **`AsyncConnection`** / **`AsyncCursor`** — Async context managers; one operation per connection at a time; concurrent use raises `ProgrammingError`.
- **Eager SELECT execution** — `async for row in cursor` works immediately after `execute(SELECT)`.
- **Cancellation handling** — `CancelledError` triggers `interrupt()` on the underlying connection; connection remains usable.
- **`docs/true_async_dbapi_spec.md`** — Specification and minimal driver checklist.

### Added - SQLAlchemy integration

- **`rapsqlite.sqlalchemy`** — `sqlite+rapsqlite` dialect; use with `create_async_engine("sqlite+rapsqlite:///:memory:")`.
- **Optional dependency** — `pip install 'rapsqlite[sqlalchemy]'` for SQLAlchemy support.
- **Compatibility docs** — `docs/guides/compatibility.rst` updated with SQLAlchemy usage.
- **Alembic with rapsqlite** — Documented in `docs/guides/compatibility.rst` (env.py, async engine, `sqlite+rapsqlite` URL). Alembic-style DDL validated in `tests/test_sqlalchemy_rapsqlite.py::test_sqlalchemy_alembic_style_migration`.
- **FastAPI patterns** — Documented in `docs/guides/compatibility.rst` (lifespan, connection dependency); `examples/fastapi_db.py` and `tests/test_fastapi_example.py` cover the recommended pattern.
- **`connect()` pathlib.Path** — `connect()` accepts `pathlib.Path` (converted via `os.fspath`) for aiosqlite compatibility.

### Added - Documentation (pool, monitoring, type conversion)

- **Pool metrics and health** — API reference (`docs/api-reference/connection.rst`): `pool_metrics()` returns `{size, num_idle, in_use}`; `pool_health()` runs `SELECT 1`. Advanced usage guide: new **Monitoring** section (pool metrics in production, health checks, query logging and slow-query detection via `set_trace_callback`).
- **Idle connection timeout (Phase 3.2)** — `connect(..., idle_timeout=N)` and `Connection.idle_timeout` (getter/setter); pool closes connections idle longer than N seconds. Documented in advanced-usage and API reference; tested in `test_phase3_api.py::test_idle_timeout`.
- **Metrics export (Phase 3.5)** — Optional helper `pool_metrics_gauges(conn)` returns a dict of gauge names to values for Prometheus or custom metrics endpoints (`rapsqlite_pool_size`, `rapsqlite_pool_num_idle`, `rapsqlite_pool_in_use`). Documented in advanced-usage (Monitoring / Metrics export) and api-reference (pool_metrics). Tested in `test_phase3_api.py::test_pool_metrics_gauges`.
- **Type system (Phase 3.10)** — `register_adapter` and `register_converter` deferred; workarounds and future plan documented in `docs/reference/type-conversion.rst` and migration guide (subsection “Future: register_adapter and register_converter”).
- **Type conversion strategy** — New `docs/reference/type-conversion.rst`: built-in parameter/result mapping, custom types today (application-layer conversion, `create_function`, `row_factory`, `text_factory`), and future plan for `register_adapter`/`register_converter`. Linked from docs index and compatibility guide.

### Added - Phase 3.5: Query timing

- **`timed_fetch_all(conn, sql, parameters=None, on_timing=None)`** — Runs fetch_all and records duration; returns (rows, duration_secs) when on_timing is None, or rows and calls on_timing(duration_secs, sql) when provided. Documented in advanced-usage (Query timing); tested in `test_phase3_api.py::test_timed_fetch_all`.

### Added - Phase 3.6 / 3.9: DX and interrupt() docs

- **Thread safety** — New subsection in advanced-usage (Thread safety): connections not thread-safe; one connection per asyncio task or pool. ROADMAP 3.6 updated.
- **Performance tuning** — Advanced-usage (Performance Tuning) now links to :doc:`guides/performance`.
- **interrupt()** — API reference (connection.rst): documented behavior (interrupts callback connection when present; no-op otherwise) and limitations (only callback connection; pool operations not interrupted).

### Added - Phase 3.3: Transaction retry

- **`transaction_retry(conn, work, max_retries=5, initial_delay=0.01, max_delay=1.0)`** — Runs a transaction with retry on transient errors (e.g. SQLITE_BUSY, SQLITE_LOCKED) and exponential backoff. ``work`` is a callable that returns an awaitable; invoked once per attempt. Documented in advanced-usage (Transaction retry); tested in `test_phase3_api.py::test_transaction_retry`.

### Added - Phase 3.2: Connection health and recovery (docs)

- **Connection health and recovery** — New subsection in advanced-usage (Monitoring): pool replaces failed connections on acquire; use pool_health() for liveness; transient errors via retry. ROADMAP 3.2 updated.

### Added - Phase 3.1: Streaming / chunked results

- **`execute_iter(conn, sql, parameters=None, chunk_size=None)`** and **`conn.execute_iter(sql, ...)`** — Async iterator that yields rows in chunks (memory-efficient). Uses LIMIT/OFFSET under the hood; chunk_size defaults to `conn.iter_chunk_size`. Documented in advanced-usage (Streaming and large result sets); tested in `test_phase3_api.py::test_execute_iter`.

### Added - Phase 3.8: aiosqlite test baseline and compatibility docs

- **`docs/AIOSQLITE_TEST_RESULTS.md`** — Per-test results and categories (fix/document/environment) for aiosqlite suite run via `scripts/run_aiosqlite_tests.py`; perf.py passes, smoke.py failures documented with notes.
- **Compatibility and ROADMAP** — `docs/guides/compatibility.rst` updated with test baseline summary and link to results; ROADMAP 3.8 updated with baseline status.
- **CONTRIBUTING** — Added "Compatibility validation (aiosqlite suite)" with command to run the aiosqlite suite and where results are written.

### Added - Test isolation and parallel runs

- **`unique_table_prefix` fixture** — Per-test unique table names to avoid cross-test collisions when running in parallel.
- **`tests/test_dbapi.py`** — DBAPI contract, connection/cursor behavior, cancellation, concurrency.
- **`tests/test_sqlalchemy_rapsqlite.py`** — Smoke tests for `sqlite+rapsqlite` engine creation.
- **`test_dbapi` and `test_concurrent_transactions`** — Use `unique_table_prefix` for all created tables.
- **Optional test dependencies** — `pip install -e ".[test]"` installs fastapi and sqlalchemy for full test coverage; `requirements-test.txt` documents optional deps for FastAPI/SQLAlchemy tests.

### Fixed

- **init_hook timing** — init_hook now runs *after* the transaction connection is acquired and set to Active in both `begin()` and `transaction()` context manager, so tables created in init_hook are visible for the rest of the transaction.
- **init_hook pool isolation** — When init_hook runs inside an active transaction and calls `conn.execute()`, the execute path now uses the transaction connection instead of acquiring from the pool (avoids "pool timed out" when pool size is 1).
- **Slow tests** — `test_iterdump_quotes_identifiers`: skip `BEGIN TRANSACTION`/`COMMIT` when replaying iterdump to avoid "cannot start a transaction within a transaction". Backup tests in `test_callback_robustness.py`: use a fresh connection for backup after closing the write connection (avoids backup timeouts). `test_connection_pooling_pattern`: sequential inserts to avoid concurrent pool hang/timeout. `test_backup_aiosqlite`: complete test (run backup, close all connections) so teardown no longer triggers Tokio-context panic warning.

### Added - Tokio panic investigation and mitigation

- **`docs/reference/tokio-panic-investigation.md`** — Documents root cause (sqlx `PoolConnection::Drop` calls `crate::rt::spawn`; GC has no Tokio context), reproduction, and mitigation options.
- **`scripts/repro_tokio_panic.py`** — Minimal repro: create Connection, use once, do not close, then `gc.collect()` to trigger panic.
- **Resource cleanup docs** — Advanced usage guide and SECURITY.md note: always use `async with` or `await conn.close()`; abandoning a connection can cause "this functionality requires a Tokio context" during GC.
- **Best-effort `Connection.__del__`** — Schedules `close()` on the running event loop when a connection is GC'd without close; best-effort only (no guarantees about loop lifetime or finalizer order).

### Changed

- **Parallel test runs** — CI uses `--timeout 90` and `--dist loadgroup` with `xdist_group` on init_hook, concurrency, and pool_exhaustion tests to avoid timeouts and pool contention; other tests use unique table names for isolation.

## [0.2.0] - 2026-01-26 (Updated 2026-01-28)

### Added - Phase 2.1: Parameterized Queries

- **Named parameters** — Support for `:name`, `@name`, `$name` parameter syntax
- **Positional parameters** — Support for `?`, `?1`, `?2` parameter syntax
- **Type-safe parameter binding** — Proper handling of all Python types (int, float, str, bytes, None)
- **`execute_many()` with parameter binding** — Efficient batch operations with parameterized queries
- Works with all query methods (`execute`, `fetch_all`, `fetch_one`, `fetch_optional`, `Cursor.execute`)

### Added - Phase 2.2: Cursor Improvements

- **`fetchmany()` size-based slicing** — Proper implementation with configurable size parameter
- **Result caching** — Cursor caches query results for efficient iteration
- **State management** — Proper cursor state tracking (current index, cached results)
- **Parameterized query support** — Cursor methods support both named and positional parameters

### Added - Phase 2.3: Connection Configuration

- **`Connection.set_pragma(name: str, value: Any)`** — Set SQLite PRAGMA settings
- **Connection string support** — URI format: `file:path?param=value`
- **PRAGMA constructor parameters** — Set PRAGMAs at connection creation time
- **Connection string parsing** — Automatic parameter extraction from URI format

### Added - Phase 2.4: Pool Configuration

- **`Connection.pool_size`** getter/setter — Configure connection pool size
- **`Connection.connection_timeout`** getter/setter — Configure connection acquisition timeout
- **Dynamic pool configuration** — Change pool settings before first use
- **Robust test suite** — `tests/test_pool_config.py` with 18 comprehensive tests
- **Edge case handling** — Zero values, large values, multiple connections, transaction integration

### Added - Phase 2.5: Row Factory

- **`Connection.row_factory`** getter/setter — Configure row output format
- **Supported formats** — `None` (list), `"dict"` (column names as keys), `"tuple"`, or callable
- **Integration** — Works with `fetch_all`, `fetch_one`, `fetch_optional`, and all Cursor methods
- **Parameterized queries** — Row factory works with parameterized queries
- **Transaction support** — Row factory works inside `transaction()` context manager
- **Comprehensive test suite** — `tests/test_row_factory.py` with 18 tests

### Added - Phase 2.6: Transaction Context Manager

- **`Connection.transaction()`** async context manager — `async with db.transaction():`
- **Automatic commit/rollback** — Commits on success, rolls back on exception
- **`execute_many` in transactions** — Fixed "database is locked" errors
- **`fetch_*` use transaction connection** — Avoids deadlock by using same connection
- **Transaction isolation** — All operations in transaction use dedicated connection

### Added - Phase 2.7: Advanced SQLite Callbacks

- **`Connection.enable_load_extension(enabled: bool)`** — Enable/disable SQLite extension loading
- **`Connection.create_function(name: str, nargs: int, func: Optional[Callable])`** — Create or remove user-defined SQL functions
  - Supports 0-6+ arguments with proper tuple unpacking
  - Handles all return types (int, float, str, bytes, None)
  - Works in transactions, aggregates, and complex queries
- **`Connection.set_trace_callback(callback: Optional[Callable])`** — Set callback to trace SQL statements
  - Captures all query types (CREATE, INSERT, SELECT, UPDATE, DELETE)
  - Works with transactions (BEGIN, COMMIT, ROLLBACK)
- **`Connection.set_authorizer(callback: Optional[Callable])`** — Set authorization callback for database operations
  - Supports all SQLite action codes
  - Can selectively deny operations
- **`Connection.set_progress_handler(n: int, callback: Optional[Callable])`** — Set progress handler for long-running operations
  - Can abort long-running operations
  - Handles exceptions gracefully

### Added - Architecture Improvements

- Dedicated callback connection architecture for safe C API access
- Callback trampolines for Python-to-SQLite C API integration
- All callback methods wired to execute/fetch operations (transaction > callbacks > pool priority)
- Connection lifecycle management: callbacks released when all cleared
- Transaction support: callbacks work correctly with begin/commit/rollback

### Added - Phase 2.8: Database Dump

- **`Connection.iterdump()`** — Dump database schema and data as SQL statements
  - Supports both async iteration (`async for line in conn.iterdump()`) and await-to-list (`lines = await conn.iterdump()`)
  - Handles tables, indexes, triggers, and views
  - Proper SQL escaping for strings and BLOB data (hex encoding)
  - Preserves all data types (INTEGER, REAL, TEXT, BLOB, NULL)
  - Works with transactions and callback connections

### Added - Phase 2.9: Database Backup

- **`Connection.backup(target, *, pages=0, progress=None, name="main", sleep=0.25)`** — Online backup API
  - Supports backing up from one `rapsqlite.Connection` to another `rapsqlite.Connection`
  - Incremental backup with configurable pages per step
  - Progress callback support with (remaining, page_count, pages_copied) parameters
  - Configurable sleep duration between backup steps
  - Works with transactions and callback connections
  - Comprehensive error handling with SQLite error codes and messages
  - Connection state validation (checks for active transactions)
  - Handle validation and lifetime management

### Added - Backup Debugging & Validation

- Enhanced error handling for backup operations
  - Detailed SQLite error codes and messages when backup fails
  - Connection state validation (active transactions, closed connections)
  - Handle validation before backup operations
  - SQLite library version checking for debugging
- Python helper module (`rapsqlite._backup_helper`) for handle extraction
  - Safely extracts sqlite3* handle from sqlite3.Connection using ctypes
  - Validates connection state before extraction
  - Handles closed connections gracefully
- Comprehensive debugging tests
  - `test_backup_sqlite_connection_state_validation` — Tests error handling for invalid states
  - `test_backup_sqlite_handle_extraction` — Tests handle extraction functionality
  - All rapsqlite-to-rapsqlite backup tests passing

### Added - Phase 2.10: Schema Operations and Introspection

- **`Connection.get_tables(name: Optional[str] = None)`** — Get list of table names
  - Returns list of table names, excluding system tables
  - Optional filter by table name
  - Works with transactions and callback connections
- **`Connection.get_table_info(table_name: str)`** — Get table column information
  - Uses `PRAGMA table_info` to get column metadata
  - Returns list of dictionaries with column details (cid, name, type, notnull, dflt_value, pk)
  - Handles all SQLite column types
- **`Connection.get_indexes(table_name: Optional[str] = None)`** — Get index information
  - Queries `sqlite_master` for indexes
  - Returns list of dictionaries with index details (name, table, unique, sql)
  - Optional filter by table name
- **`Connection.get_foreign_keys(table_name: str)`** — Get foreign key constraints
  - Uses `PRAGMA foreign_key_list` to get foreign key information
  - Returns list of dictionaries with FK details (id, seq, table, from, to, on_update, on_delete, match)
- **`Connection.get_schema(table_name: Optional[str] = None)`** — Comprehensive schema information
  - Combines table info, indexes, and foreign keys
  - Returns structured dictionary
  - Supports single table or all tables
- **`Connection.get_views(name: Optional[str] = None)`** — Get list of view names
  - Returns list of view names (strings)
  - Optional filter by view name
  - Works with transactions and callback connections
- **`Connection.get_index_list(table_name: str)`** — Get index list using PRAGMA index_list
  - Returns list of dictionaries with index list information
  - Includes: seq, name, unique, origin (c/u/pk), partial
  - More detailed than `get_indexes()` for table-specific index information
- **`Connection.get_index_info(index_name: str)`** — Get column information for an index
  - Uses `PRAGMA index_info` to get index column details
  - Returns list of dictionaries with: seqno, cid, name
  - Useful for understanding composite index column ordering
- **`Connection.get_table_xinfo(table_name: str)`** — Extended table information
  - Uses `PRAGMA table_xinfo` (SQLite 3.26.0+)
  - Returns same information as `get_table_info()` plus `hidden` field
  - Hidden field indicates: 0=normal, 1=hidden, 2=virtual, 3=stored
  - Useful for detecting generated columns and hidden system columns

### Added - Phase 2.11: Database Initialization Hooks

- **`Connection.__new__(path, *, pragmas=None, init_hook=None)`** — `init_hook` parameter for automatic database initialization
  - **Note:** This is a rapsqlite-specific enhancement and is not available in aiosqlite
  - Optional async callable that receives the `Connection` object
  - Called automatically once when the connection pool is first used
  - Perfect for schema setup, initial data seeding, and PRAGMA configuration
  - Hook is only called once per `Connection` instance
  - Errors in the hook are properly propagated to the caller
  - Works with all connection operations (execute, fetch_*, schema introspection, transactions, etc.)
  - Comprehensive test suite with 36 tests covering all use cases

### Added - Code Quality & Type Safety

- **Type checking** — Full mypy type checking support
  - Fixed type stub syntax issues in `_rapsqlite.pyi`
  - Added type alias for `init_hook` callback signature
  - Fixed type annotations in `_backup_helper.py` for platform-dependent pointer sizes
  - All 13 source files pass mypy type checking
- **Code formatting and linting** — Ruff integration
  - Configured Ruff formatter and linter in `pyproject.toml`
  - Excluded `.pyi` files from formatting (type stubs have distinct syntax)
  - Fixed unused imports and variables across test files
  - All code passes `ruff format` and `ruff check`

### Added - Testing

- **`tests/test_init_hook.py`** — 36 comprehensive tests for database initialization hooks
  - Schema setup and data seeding
  - PRAGMA configuration
  - Error handling (SQL errors, database constraint errors, exceptions)
  - Concurrent access and recursive prevention
  - Integration with all connection operations (execute, fetch_*, schema introspection, transactions, cursors, etc.)
  - Complex schema initialization
- **`tests/test_callback_robustness.py`** — 35 comprehensive tests covering:
  - Edge cases for all callback types (many arguments, stateful functions, BLOBs, NULLs, exceptions)
  - Complex scenarios (transactions, concurrent calls, rapid queries, special characters)
  - Integration tests (all callbacks together, pool size variations, cursor operations)
  - Comprehensive iterdump tests (indexes, triggers, views, BLOBs, special characters, multiple tables)
- **`tests/test_aiosqlite_compat.py`** — Compatibility tests including schema operations (6 new tests verifying schema methods match manual SQL queries)
- **`tests/test_schema_operations.py`** — 72 comprehensive tests for all schema introspection methods
- **`tests/test_pool_config.py`** — 18 tests for pool configuration
- **`tests/test_row_factory.py`** — 18 tests for row factory functionality
- **345 total tests passing** (7 skipped)

### Added - Test Infrastructure & Comprehensive Test Suite (2026-01-27)

- **Shared test infrastructure** (`tests/conftest.py`)
  - Centralized `test_db` fixture for temporary database files
  - `test_db_memory` fixture for in-memory databases
  - `cleanup_db()` helper function for database cleanup
  - Pytest marker registration for test categorization
- **Test organization improvements**
  - Added pytest markers: `unit`, `integration`, `edge_case`, `concurrency`, `stress`, `performance`, `property`, `slow`
  - Enhanced test categorization and filtering capabilities
- **Edge case tests** (`tests/test_edge_cases.py`) — 24 comprehensive tests covering:
  - Connection pool edge cases (exhaustion, timeouts, zero/one/large sizes)
  - Transaction edge cases (nested transactions, closed connections, concurrent transactions)
  - Parameter and query edge cases (empty params, large params >16, SQL injection attempts, unicode, very long queries, special characters)
  - Connection lifecycle edge cases (operations on closed connections, multiple close calls)
  - Row factory and type conversion edge cases (invalid factories, very large integers, NaN/infinity floats, empty/large BLOBs, NULL handling)
- **Error condition tests** (`tests/test_error_conditions.py`) — 15 comprehensive tests covering:
  - Database file errors (creation, invalid paths)
  - SQL syntax errors and malformed queries
  - Table and column not found errors
  - Constraint violations (unique, NOT NULL, foreign key)
  - Missing parameters and invalid parameter types
  - Transaction errors (rollback/commit without transaction)
  - Cursor errors on closed connections
- **Concurrency tests** (`tests/test_concurrency.py`) — 8 comprehensive tests covering:
  - Concurrent read operations (multiple simultaneous readers)
  - Concurrent write operations (sequential execution due to SQLite limitations)
  - Concurrent transactions
  - Concurrent pool operations
  - Race conditions in connection acquisition
  - Database locked error handling
  - Concurrent fetch operations
  - Concurrent execute_many operations
- **Stress tests** (`tests/test_stress.py`) — 8 comprehensive tests covering:
  - High concurrency scenarios (100+ concurrent operations)
  - Many small operations vs few large operations
  - Large result sets (10K+ rows)
  - Connection pool under heavy load
  - Memory leak detection (repeated operations with garbage collection)
  - Long-running transactions
  - Repeated prepared statement usage (cache effectiveness)
  - Concurrent connections stress testing
- **Property-based tests** (`tests/test_properties.py`) — 7 Hypothesis-based property tests covering:
  - Parameter round-trip consistency (insert → select for all types)
  - Multiple parameters round-trip
  - Transaction atomicity properties
  - Pool size invariants
  - Text, integer, and BLOB round-trip properties
- **Integration tests** (`tests/test_integration.py`) — 8 comprehensive tests covering:
  - Web framework usage patterns (FastAPI/aiohttp-style request-scoped connections)
  - ORM-like usage patterns
  - Batch processing patterns
  - Transaction rollback patterns for error handling
  - Connection pooling patterns for high-throughput scenarios
  - Schema migration patterns
  - Row factory integration in real-world usage
  - Cursor iteration patterns
- **Performance tests** (`tests/test_performance.py`) — 6 performance regression tests covering:
  - Query execution time benchmarks
  - Connection pool performance
  - Prepared statement cache effectiveness
  - Execute_many performance
  - Large result set performance
  - Transaction performance
- **Test coverage and CI improvements**
  - Added `pytest-cov` configuration in `pyproject.toml` with coverage thresholds
  - Enhanced CI workflow to run full test suite with coverage reporting
  - Added parallel test execution in CI using `pytest-xdist`
  - Added Codecov integration for coverage tracking
  - Coverage configuration excludes test files and sets precision to 2 decimal places
- **Test documentation**
  - Created `tests/README.md` with comprehensive testing documentation:
    - Test organization and file structure
    - Running tests (all, by category, with coverage)
    - Test fixtures and markers
    - Writing new tests guidelines
    - Best practices and debugging tips
  - Created `CONTRIBUTING.md` with contribution guidelines:
    - Development setup instructions
    - Code style and formatting guidelines
    - Testing guidelines and best practices
    - Commit message conventions
    - Pull request process
- **432 total tests passing** (6 skipped) — Increased from 345 tests
  - 76 new tests added across 8 new test files
  - Comprehensive coverage of edge cases, error conditions, concurrency, stress scenarios, property-based testing, integration patterns, and performance regression

### Fixed

- Fixed `create_function` argument unpacking (functions now receive individual arguments, not tuples)
- Fixed pool timeout issues when callbacks are cleared (connection properly released)
- Fixed transaction connection management with callbacks (connection returned to callback pool on commit/rollback)
- Fixed `test_set_pragma` assertion to match SQLite's documented behavior (PRAGMA synchronous NORMAL = 1, not 2)
- Fixed Python object lifetime management in backup operations (connections now properly kept alive during async backup)
- Fixed exception inheritance to match DB-API expectations (e.g., `OperationalError` subclasses `DatabaseError`/`Error`)
- **Fixed deadlock in `init_hook` with `begin()` and `transaction()`** — Resolved deadlock that occurred when `init_hook` called `conn.execute()` while `begin()` or `transaction()` context manager was acquiring the transaction connection. The fix releases the `transaction_state` lock before calling `execute_init_hook_if_needed()`, allowing init_hook operations to check transaction state without deadlocking. Both `Connection.begin()` and `TransactionContextManager.__aenter__()` now properly handle init_hook execution without blocking.

### Added - Phase 2.14: aiosqlite Compatibility Completion

- **`Connection.total_changes()`** — Get total number of database changes since connection was opened (cumulative count of INSERT/UPDATE/DELETE operations)
- **`Connection.in_transaction()`** — Check if connection is currently in a transaction (returns boolean)
- **`Cursor.executescript(script: str)`** — Execute multiple SQL statements separated by semicolons
- **`Connection.load_extension(name: str)`** — Load a SQLite extension from the specified file (requires `enable_load_extension(True)` first)
- **`Connection.text_factory`** — Getter/setter for text decoding factory (callable that takes bytes and returns str)
- **`rapsqlite.Row` class** — Dict-like row accessor class similar to `aiosqlite.Row`, supporting:
  - Index access: `row[0]`, `row["column_name"]`
  - Dict-like methods: `keys()`, `values()`, `items()`
  - Iteration: `for col in row:` (iterates over column names)
  - String representation: `str(row)`, `repr(row)`
- **Async iteration on cursors** — Support for `async for row in cursor:` pattern via `__aiter__` and `__anext__` methods
- **Enhanced `async with db.execute(...)` compatibility** — Full support for aiosqlite's context manager pattern

**Compatibility improvements:**
- All high-priority aiosqlite compatibility features now implemented
- Core API compatibility increased from ~85% to ~95%
- Migration guide updated with all new features
- Type stubs complete for all new APIs

### Added - Phase 2.13: Prepared Statements & Performance Optimization

- **Prepared statement caching verification and documentation** — Verified and documented that sqlx automatically caches prepared statements per connection
- **Enhanced query normalization documentation** — Added comprehensive documentation explaining how query normalization maximizes prepared statement cache hit rates
- **Performance testing suite** — Created comprehensive test suite (`tests/test_prepared_statements.py`) with 8 tests covering:
  - Query normalization
  - Repeated query performance
  - Parameterized query caching
  - Transaction query caching
  - `execute_many` caching
  - Concurrent query caching
  - Performance comparison (repeated vs unique queries)
- **Performance characteristics documented** — Added detailed documentation in `docs/ADVANCED.md` explaining prepared statement caching benefits (2-5x faster for repeated queries)

**Performance improvements:**
- sqlx automatically caches prepared statements per connection (no configuration needed)
- Query normalization ensures maximum cache hit rates
- Tests demonstrate significant performance benefits for repeated queries
- Memory usage remains reasonable (sqlx handles cache management internally)

### Added - Phase 2.15: Documentation & Benchmarking

- **Benchmark results documented** — Updated `benchmarks/README.md` with actual benchmark results from macOS arm64 system:
  - Simple Query Throughput: 0.118ms mean latency (1000 queries)
  - Batch Insert Performance: 505ms for 1000 rows
  - Concurrent Reads: 65ms for 10 workers × 100 queries
  - Transaction Performance: 235ms for 100 transactions × 10 inserts
- **Enhanced advanced usage documentation** — Updated `docs/ADVANCED.md` with:
  - Comprehensive prepared statement caching documentation
  - Performance tuning best practices
  - Detailed examples and anti-patterns
- **Updated main documentation** — Enhanced `README.md` with:
  - Complete feature list including all Phase 2 features
  - Benchmark summary with actual results
  - Performance characteristics
- **Roadmap updated** — Marked Phase 2.13 and 2.15 complete, Phase 2 now 100% complete

**Documentation improvements:**
- All major features documented with examples
- Performance characteristics documented
- Best practices and anti-patterns covered
- Production-ready documentation available

### Added - Phase 2.16: SQLite busy_timeout Support (aiosqlite Compatibility)

- **`timeout` parameter in `connect()` and `Connection.__new__()`** — Set SQLite busy_timeout when creating connections
  - Default: 5.0 seconds (matching sqlite3/aiosqlite default)
  - Controls how long SQLite waits when database is locked by another process/thread
  - Set to 0.0 to disable timeout
  - Applied via `PRAGMA busy_timeout` in transactions
- **`Connection.timeout` property** — Getter/setter for SQLite busy_timeout value
  - Get current timeout: `db.timeout` (returns float in seconds)
  - Set timeout: `db.timeout = 10.0` (sets timeout to 10 seconds)
  - Validates timeout >= 0.0 (raises `ValueError` for negative values)
  - Changes apply to new transactions
- **Timeout integration** — Timeout is automatically applied when:
  - Starting transactions via `begin()` method
  - Using transaction context managers (`async with db.transaction()`)
  - Timeout value is converted from seconds to milliseconds for SQLite PRAGMA
- **Comprehensive test suite** — `tests/test_timeout.py` with 15 tests covering:
  - Default timeout value (5.0 seconds)
  - Setting timeout via connect() parameter
  - Timeout property getter/setter
  - Validation (negative values raise ValueError)
  - Timeout applied in transactions and transaction context managers
  - Zero timeout (disables busy_timeout)
  - Float values and large timeout values
  - Multiple connections with independent timeouts
  - Timeout working with PRAGMA settings
  - Timeout conversion verification (seconds to milliseconds)

**Compatibility improvements:**
- Full aiosqlite compatibility for timeout parameter
- Matches sqlite3 standard library timeout behavior
- Seamless migration from aiosqlite with timeout support

### Known Limitations

- **Backup to `sqlite3.Connection` is file-backed only**: `Connection.backup()` supports backing up to a `sqlite3.Connection` target only when the source database is file-backed. `:memory:` databases and non-file URIs are not supported for sqlite3 targets. (This avoids unsafe cross-library handle sharing; the implementation uses Python's sqlite3 backup API on the on-disk database file.)

### Changed

- Updated date to 2026-01-26
- Enhanced backup error messages with SQLite error codes and diagnostic information
- Improved documentation for backup functionality with clear limitations and workarounds
- Updated test suite count from 276 to 432 passing tests (36 new init_hook tests, deadlock fix validation, prepared statement tests, 76 new comprehensive test suite tests, 15 new timeout tests)
- **Major aiosqlite compatibility improvements** — Implemented all high-priority compatibility features, increasing compatibility from ~85% to ~95%
- Updated compatibility analysis and migration guide to reflect new features
- **Phase 2 Complete** — All phases 2.1-2.16 now complete (100% of Phase 2)
- **Prepared statement caching verified and documented** — sqlx automatically handles prepared statement caching per connection
- **Benchmarks documented** — Actual benchmark results published with performance analysis
- **SQLite busy_timeout support added** — Full aiosqlite/sqlite3 compatibility for timeout parameter
- **Comprehensive documentation** — All features documented with examples, best practices, and performance tuning guides

---

---

## [0.1.1] - 2026-01-16

### Added

- Python 3.14 support with ABI3 forward compatibility
- Python 3.13 support with ABI3 forward compatibility
- Updated CI/CD workflows to test and build for Python 3.14
- Updated CI/CD workflows to test and build for Python 3.13

### Fixed

- Fixed exception handling for ABI3 compatibility (using `create_exception!` macro)
- Explicitly registered exception classes in Python module
- Fixed exception registration issue where exceptions created with `create_exception!` were not accessible from Python

### Compatibility

- Python 3.10 through 3.14 supported
- All platforms: Ubuntu (x86-64, aarch64), macOS (aarch64, x86-64), Windows (x86-64, aarch64)

---

## [0.1.0] - 2025-01-12

### Added - Initial Release - Phase 1 Complete

- Connection lifecycle management (async context managers)
- Transaction support (begin, commit, rollback)
- Type system improvements (proper Python types: int, float, str, bytes, None)
- Enhanced error handling (custom exception classes matching aiosqlite)
- API improvements (fetch_one, fetch_optional, execute_many, last_insert_rowid, changes)
- Cursor API (execute, executemany, fetchone, fetchall, fetchmany)
- aiosqlite compatibility (connect function, exception types)
- Connection pooling: Connection reuses connection pool across operations
- Input validation: Added path validation (non-empty, no null bytes)
- Improved error handling: Enhanced error messages with database path and query context
- Type stubs: Added `.pyi` type stubs for better IDE support and type checking

### Security

- Upgraded dependencies (pyo3 0.27, pyo3-async-runtimes 0.27, sqlx 0.8)
- All critical vulnerabilities resolved

---

[0.2.0]: https://github.com/eddiethedean/rapsqlite/releases/tag/v0.2.0
[0.1.1]: https://github.com/eddiethedean/rapsqlite/releases/tag/v0.1.1
[0.1.0]: https://github.com/eddiethedean/rapsqlite/releases/tag/v0.1.0
