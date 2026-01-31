# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, a true async SQLite library for Python built with Rust, Tokio, and sqlx.

## Current Status

**Current Version: v0.3.0-dev** 🚧  
**Phase 1: Complete** ✅  
**Phase 2: Complete** ✅  
**Phase 3: In Development** ⏳

### What's Complete

**Phase 1 (v0.1.x)** — Core functionality and production readiness:
- ✅ Connection lifecycle management (async context managers)
- ✅ Transaction support (begin, commit, rollback, transaction context managers)
- ✅ Type system (proper Python types: int, float, str, bytes, None)
- ✅ Error handling (custom exception classes matching aiosqlite)
- ✅ API compatibility (~95% aiosqlite compatibility)
- ✅ Connection pooling with configurable size and timeouts
- ✅ Input validation and security improvements
- ✅ Type stubs for IDE support

**Phase 2 (v0.2.0)** — Feature-complete drop-in replacement:
- ✅ Parameterized queries (named and positional parameters)
- ✅ Cursor improvements (fetchmany, result caching, state management)
- ✅ Connection configuration (PRAGMAs, connection strings, constructor parameters)
- ✅ Pool configuration (pool_size, connection_timeout getters/setters)
- ✅ Row factory compatibility (dict, tuple, callable)
- ✅ Transaction context managers (`async with db.transaction()`)
- ✅ Advanced SQLite callbacks (create_function, set_trace_callback, set_authorizer, set_progress_handler)
- ✅ Database dump (`iterdump()`) and backup (`backup()`)
- ✅ Schema introspection (9 methods: get_tables, get_table_info, get_indexes, etc.)
- ✅ Database initialization hooks (`init_hook` parameter)
- ✅ Prepared statement caching (verified and documented)
- ✅ SQLite busy_timeout support (`timeout` parameter matching aiosqlite)
- ✅ Comprehensive documentation and benchmarking

**Test Coverage**: 560 tests passing (7 skipped)  
**API Compatibility**: ~95% with aiosqlite (Phase 3.9 additions improve compatibility)  
**Python Support**: 3.10–3.14  
**Code Quality**: Full mypy type checking and Ruff formatting/linting

**Phase 3 (v0.3.0-dev) — In progress:**
- ✅ **3.9 API completeness**: `execute_fetchall`, `execute_insert`, Cursor properties (`arraysize`, `connection`, `description`, `lastrowid`, `rowcount`, `row_factory`), `Cursor.close()`, `isolation_level`, `__await__`, `interrupt` (full implementation), `Connection.stop()` (no-op), `Cursor.execute()`/`executemany()`/`executescript()` return self (aiosqlite chaining)
- ✅ **3.1**: `explain_query_plan` helper
- ✅ **3.2**: `pool_health` helper
- ✅ **3.3**: `isolation_level` applied to transactions
- ✅ **init_hook** — Runs after transaction is active in `begin()` and `transaction()`; execute during init_hook uses transaction connection (avoids pool timeout when pool size is 1)
- ✅ **True Async DBAPI** — `rapsqlite.dbapi`: `AsyncConnection`, `AsyncCursor`, `connect()`, eager SELECT, cancellation handling, one-op-per-connection; see `docs/true_async_dbapi_spec.md`
- ✅ **SQLAlchemy dialect** — `sqlite+rapsqlite` via `rapsqlite.sqlalchemy`; optional `rapsqlite[sqlalchemy]`; `create_async_engine("sqlite+rapsqlite:///...")`
- ✅ **Test isolation** — `unique_table_prefix` fixture; `xdist_group` for init_hook, concurrency, pool_exhaustion; CI uses `--timeout 90` and `--dist loadgroup`; optional test deps (`.[test]`) for FastAPI/SQLAlchemy tests
- ✅ aiosqlite test suite runner; `tests/test_phase3_api.py` for Phase 3 APIs
- ✅ **Session-connection reuse** — Each Connection reuses one pool connection for non-transaction, non-callback operations; released on `close()` and when starting a transaction. Concurrent Reads benchmark (10 workers × 2000 queries) wins vs aiosqlite; benchmarks/README.md updated with latest results.
- ✅ **Query helpers** — `paginate()`, `analyze_query_plan()`, `transaction_with_timeout()`, `set_slow_query_threshold()`; documented in advanced-usage.
- ✅ **Starlette and aiohttp** — Integration examples (`examples/starlette_db.py`, `examples/aiohttp_db.py`) and compatibility docs.
- ✅ **FTS5 and JSON1** — Tests (`tests/test_fts.py`, `tests/test_json1.py`); FTS/JSON usage documented in advanced-usage (Streaming section).
- ✅ **Cursor chaining** — `Cursor.executemany()` and `Cursor.executescript()` return self (aiosqlite compatibility).

---

## Phase 3 — Advanced Features & Ecosystem (v0.3.0 → v1.0.0)

**Goal**: Transform `rapsqlite` into the industry-leading async SQLite library for Python with advanced features, ecosystem integration, and optimizations leading to a stable v1.0.0 release.

**Timeline**: Incremental releases (v0.3.0, v0.4.0, etc.) leading to v1.0.0

### 3.1 Query Optimization & Performance (High Priority)

**Focus**: Advanced query features and performance optimizations

#### Query Optimization
- ✅ **Query plan analysis** — `analyze_query_plan(conn, sql, parameters=None)` returns dict with `uses_index`, `table_scan`, `details`; documented in advanced-usage.
- ⏳ Automatic index recommendations
- ⏳ Query result caching strategies
- ⏳ Lazy query execution patterns
- ✅ **`Connection.explain_query_plan(sql, parameters=None)`** — Runs `EXPLAIN QUERY PLAN` and returns result rows

#### Result Handling
- ✅ **Streaming / chunked results** — `execute_iter(conn, sql, parameters=None, chunk_size=None)` and `conn.execute_iter(sql, ...)` return an async iterator yielding chunks of rows (LIMIT/OFFSET under the hood); documented in advanced-usage (Streaming and large result sets).
- ✅ **Page-based pagination** — `paginate(conn, sql, parameters=None, page_size=64, offset=0)` returns one page of rows; documented in advanced-usage.
- ⏳ Result set transformation utilities
- ⏳ Row-to-object mapping helpers
- ⏳ Efficient memory usage patterns for large result sets

#### SQLite-Specific Features
- ✅ **Full-text search (FTS5)** — Create virtual tables with `CREATE VIRTUAL TABLE ... USING fts5(...)`; tests in `tests/test_fts.py`; documented in advanced-usage.
- ✅ **JSON functions (JSON1)** — Use `json_extract`, `json_object`, `->`, `->>` in SQL; tests in `tests/test_json1.py`; documented in advanced-usage.
- ⏳ Window functions support
- ⏳ Common Table Expressions (CTEs) utilities
- ⏳ UPSERT operations (INSERT OR REPLACE, INSERT OR IGNORE)

**Success Criteria**:
- Query plan analysis available for all queries
- Streaming results support datasets >100MB efficiently
- FTS and JSON functions fully supported
- Performance benchmarks show 20%+ improvement for optimized queries

---

### 3.2 Advanced Connection Pooling (High Priority)

**Focus**: Production-grade connection pool management

#### Pool Management
- ⏳ Dynamic pool sizing (scale up/down based on load)
- ✅ **Session-connection reuse** — Each Connection holds one pool connection for non-transaction operations (`fetch_all`, `execute`, etc.); released on `close()` and when starting a transaction. Improves concurrent read performance (Concurrent Reads benchmark wins vs aiosqlite).
- ✅ **`Connection.pool_health()`** — Minimal health check (`SELECT 1`); raises on failure
- ✅ **Connection health and recovery** — Documented in advanced-usage (Monitoring): pool_health() for liveness; pool replaces failed connections on acquire; transient errors (e.g. SQLITE_BUSY) handled via retry.
- ✅ **Idle connection management** — `idle_timeout` (seconds) via `connect(..., idle_timeout=N)` or `conn.idle_timeout = N`; pool closes idle connections after timeout; documented in advanced-usage and API reference.
- ✅ **Pool metrics** — `pool_metrics()` returns `{size, num_idle, in_use}`; documented in API reference and advanced-usage (Monitoring section).
- ⏳ Cross-process connection sharing patterns (if applicable)

#### Connection Features
- ⏳ Read/write connection separation
- ⏳ Read replica patterns
- ⏳ Connection routing strategies
- ⏳ Failover and recovery patterns
- ⏳ Connection state tracking and diagnostics

**Success Criteria**:
- Pool automatically recovers from connection failures
- Metrics available for monitoring pool health
- Dynamic sizing reduces resource usage by 30%+ under low load
- Health checks prevent stale connection usage

---

### 3.3 Advanced Transaction Features (Medium Priority)

**Focus**: Enhanced transaction capabilities

#### Transaction Features
- ✅ **Nested transaction handling (savepoints)** — `Connection.savepoint(name=None)` context manager
- ✅ **`Connection.isolation_level`** — Get/set `None` | `"DEFERRED"` | `"IMMEDIATE"` | `"EXCLUSIVE"`; applied to `BEGIN`
- ⏳ Deadlock detection and automatic retry
- ✅ **Transaction timeout** — `transaction_with_timeout(conn, work, timeout_secs=30)` runs a transaction with `asyncio.wait_for`; documented in advanced-usage.
- ⏳ Long-running transaction monitoring

#### Transaction Utilities
- ✅ **Savepoint context managers** — `async with db.savepoint():` or `db.savepoint("name")`; implemented and tested
- ✅ **Transaction retry** — `transaction_retry(conn, work, max_retries=5, ...)` runs a transaction with retry on transient errors (e.g. SQLITE_BUSY, SQLITE_LOCKED) and exponential backoff; documented in advanced-usage (Transaction retry).
- ⏳ Transaction conflict resolution strategies

**Success Criteria**:
- Savepoints fully supported with context managers
- Deadlock detection prevents transaction hangs
- Isolation levels configurable per transaction
- Transaction retry utilities reduce application complexity

---

### 3.4 ORM & Framework Integration (High Priority)

**Focus**: Seamless integration with popular Python frameworks

#### ORM Support
- ✅ **SQLAlchemy async dialect** — `sqlite+rapsqlite`; `create_async_engine("sqlite+rapsqlite:///...")`; install `rapsqlite[sqlalchemy]`
- ⏳ Tortoise ORM async SQLite backend
- ⏳ Peewee async SQLite support
- ⏳ Custom ORM adapters and patterns
- ⏳ Query builder integrations

#### Web Framework Integration
- ✅ **FastAPI** — Patterns documented in `docs/guides/compatibility.rst` (lifespan, connection dependency); `examples/fastapi_db.py` and `tests/test_fastapi_example.py`.
- ⏳ Django async database backend (if applicable)
- ✅ **aiohttp** — Example (`examples/aiohttp_db.py`), tests (`tests/test_aiohttp_example.py`); documented in compatibility.rst.
- ✅ **Starlette** — Example (`examples/starlette_db.py`), tests (`tests/test_starlette_example.py`); documented in compatibility.rst.
- ⏳ Quart async database support
- ⏳ Sanic async database patterns

#### Migration Tools
- ✅ **Alembic** — Documented in `docs/guides/compatibility.rst` (async env.py, `sqlite+rapsqlite`); Alembic-style DDL test in `test_sqlalchemy_rapsqlite.py`.
- ⏳ Migration generation utilities
- ⏳ Schema migration testing tools

**Success Criteria**:
- ✅ SQLAlchemy async driver (`sqlite+rapsqlite`) available; Alembic and FastAPI documented and validated
- ✅ FastAPI integration examples and patterns documented
- ✅ Alembic with rapsqlite documented and DDL validated
- ✅ At least 3 major frameworks have integration examples (FastAPI, Starlette, aiohttp)

---

### 3.5 Observability & Monitoring (Medium Priority)

**Focus**: Production monitoring and debugging capabilities

#### Monitoring & Metrics
- ✅ **Metrics export** — Optional helper `pool_metrics_gauges(conn)` returns dict of gauge names (e.g. `rapsqlite_pool_size`, `rapsqlite_pool_num_idle`, `rapsqlite_pool_in_use`) for Prometheus or custom metrics endpoints; documented in advanced-usage (Monitoring) and api-reference (pool_metrics).
- ✅ **Query timing** — `timed_fetch_all(conn, sql, parameters=None, on_timing=None)` runs fetch_all and records duration; optional callback or returns (rows, duration_secs); documented in advanced-usage (Query timing).
- ✅ **Connection pool metrics** — `pool_metrics()` and `pool_health()` documented; Monitoring section in advanced-usage.
- ⏳ Resource usage tracking
- ✅ **Slow query detection** — `set_slow_query_threshold(conn, threshold_secs, callback=None)` invokes callback when `fetch_all` exceeds threshold; documented in advanced-usage.

#### Debugging Tools
- ✅ **Query logging** — Documented: use `set_trace_callback` to log SQL; slow-query detection via app-layer timing.
- ⏳ Transaction tracing
- ⏳ Connection pool diagnostics
- ⏳ Performance profiling utilities
- ⏳ Query execution visualization

**Success Criteria**:
- Metrics exportable to common monitoring systems
- Query logging helps debug production issues
- Slow query detection identifies bottlenecks
- Profiling tools reduce debugging time by 50%+

---

### 3.6 Developer Experience (Medium Priority)

**Focus**: Tools and utilities for better developer experience

#### Developer Tools
- ⏳ Query logging and profiling utilities
- ⏳ Database introspection CLI tools
- ⏳ Migration generation utilities
- ⏳ Testing utilities and fixtures
- ⏳ Database mocking for tests

#### Type System Enhancements
- ⏳ Enhanced type hints for Python types
- ⏳ Type conversion utilities
- ⏳ Configurable type conversion
- ⏳ Type inference from schema
- ⏳ Date/time type handling utilities

#### Documentation & Examples
- ⏳ Advanced usage patterns and examples
- ✅ **Performance tuning** — Advanced-usage (Performance Tuning) links to :doc:`guides/performance`; ROADMAP references performance guide.
- ⏳ Migration documentation from other libraries
- ⏳ Best practices and anti-patterns
- ⏳ Contributing guidelines
- ✅ **Thread-safety** — Documented in advanced-usage (Thread safety): connections not thread-safe; one connection per task or pool.

**Success Criteria**:
- CLI tools available for common tasks
- Type hints improve IDE experience significantly
- Comprehensive examples for all major use cases
- Migration guides enable easy adoption

---

### 3.7 Advanced Database Features (Low Priority)

**Focus**: Specialized database capabilities

#### Database Features
- ⏳ Database encryption support (if applicable)
- ⏳ Multi-database transaction support
- ⏳ Custom SQLite extensions support
- ⏳ Replication patterns
- ⏳ Enhanced backup and restore utilities

#### Schema Operations
- ⏳ Migration utilities and helpers
- ⏳ Schema validation tools
- ⏳ Schema comparison utilities
- ⏳ Automatic migration generation

#### Parameterized Queries
- ⏳ Enhanced array parameter binding for IN clauses
- ⏳ Bulk operation optimizations

**Success Criteria**:
- Encryption support available if SQLite supports it
- Migration utilities reduce manual work
- Schema validation prevents deployment errors

---

### 3.8 Testing & Validation (High Priority)

**Focus**: Comprehensive test coverage and validation

#### Test Coverage
- ⏳ Complete edge case coverage
- ⏳ Fake Async Detector validation passes under load
- ✅ **aiosqlite test suite** — Run via `scripts/run_aiosqlite_tests.py`; baseline and per-test categories in `docs/AIOSQLITE_TEST_RESULTS.md` (perf.py passes; smoke.py failures mostly **document** or **environment**; a few **fix** candidates).
- ⏳ Pass 100% of aiosqlite test suite as drop-in replacement validation (optional; intentional differences documented)
- ⏳ Stress testing and performance regression tests
- ⏳ Cross-platform testing (Linux, macOS, Windows)

#### Compatibility Testing
- ⏳ Continuous compatibility testing with aiosqlite
- ⏳ Python version compatibility matrix (3.10–3.14+)
- ⏳ Platform-specific testing and validation

**Success Criteria**:
- 100% of aiosqlite test suite passes
- Edge cases comprehensively covered
- No performance regressions in benchmarks
- All supported platforms validated

---

### 3.9 API Completeness & Compatibility (High Priority)

**Focus**: Complete aiosqlite API compatibility to achieve 100% drop-in replacement status

#### Connection Helper Methods
- ✅ **`Connection.execute_fetchall(sql, parameters=None)`** — Execute SELECT and return all rows (delegates to `fetch_all`)
- ✅ **`Connection.execute_insert(sql, parameters=None)`** — Execute INSERT/UPDATE/DELETE and return `last_insert_rowid()`; rejects SELECT

#### Connection Control Methods
- ✅ **`Connection.interrupt()`** — Interrupts callback connection when present; no-op otherwise
- ✅ **`Connection.stop()`** — No-op for aiosqlite API compatibility; use `close()` to close the connection.

#### Connection Properties
- ✅ **`Connection.isolation_level`** — Get/set `None` | `"DEFERRED"` | `"IMMEDIATE"` | `"EXCLUSIVE"`; applied to `BEGIN`

#### Connection Await Support
- ✅ **`Connection.__await__()`** — Support for `await conn` pattern (enter connection and return self)

#### Cursor Properties
- ✅ **`Cursor.arraysize`** — Default size for `fetchmany()` (int, default 1, read-write)
- ✅ **`Cursor.connection`** — Reference to parent Connection (read-only)
- ✅ **`Cursor.description`** — Column metadata 7-tuples after execute/fetch (read-only)
- ✅ **`Cursor.lastrowid`** / **`Cursor.rowcount`** — From last execute (read-only)
- ✅ **`Cursor.row_factory`** — Per-cursor override (getter/setter)
- ✅ **`Cursor.fetchmany(size=None)`** — `size` optional; uses `arraysize` when omitted

#### Cursor Methods
- ✅ **`Cursor.close()`** — Async; clears cached results, description, lastrowid, rowcount

**Success Criteria**:
- ✅ Connection helper methods (`execute_fetchall`, `execute_insert`) implemented and tested
- ✅ Cursor properties and `close()` implemented and tested
- ✅ `Connection.interrupt()` — Full implementation (calls sqlite3_interrupt on callback connection)
- ✅ Cursor properties reflect query state; isolation_level applied to transactions
- ⏳ 100% aiosqlite API compatibility (progress toward ~95%+)
- ✅ New methods/properties covered in `tests/test_phase3_api.py`

---

### 3.10 Type System Enhancements (Medium Priority)

**Focus**: Enhanced type conversion and adapter support for custom types

#### create_function() Enhancement
- ✅ `Connection.create_function(name, num_params, func, deterministic=False)` - `deterministic` parameter; `NotSupportedError` on SQLite &lt; 3.8.3 (SQLite version, not Python)

#### connect() Parameters
- ✅ `connect(iter_chunk_size=64)` - Stored; used for chunked iteration when applicable
- ✅ `connect(loop=None)` - Accepted and ignored (deprecated in aiosqlite)

#### Module-Level Type Registration Functions
- ⏳ `rapsqlite.register_adapter(type, adapter)` - Register Python-to-SQLite type adapter (planned; not yet implemented)
- ⏳ `rapsqlite.register_converter(typename, converter)` - Register SQLite-to-Python type converter (planned; not yet implemented)
- These are sqlite3 compatibility features for custom type handling; require integration with Rust binding/decoding paths
- ✅ **Type conversion strategy documented** — `docs/reference/type-conversion.rst` describes built-in mapping, custom types today (application-layer conversion, `create_function`, `row_factory`, `text_factory`), and future adapter/converter plan

#### Enhanced Type Conversion Utilities
- ⏳ Date/time type handling utilities
- ⏳ UUID type support
- ⏳ Decimal type support
- ✅ Custom type approach documented (application-layer, row_factory, text_factory until adapter/converter exist)

**Success Criteria**:
- `deterministic` parameter works correctly with create_function()
- `iter_chunk_size` and `loop` parameters accepted (even if not fully utilized)
- ⏳ `register_adapter` and `register_converter` implemented (deferred; current approach and plan documented in type-conversion.rst and migration guide)
- ✅ Custom type conversions possible via documented workarounds (application-layer, row_factory, text_factory, create_function)
- Type conversion strategy documented with examples
- All type system features have test coverage

---

## Versioning Strategy

Following semantic versioning:

- **v0.1.x**: Phase 1 (MVP and core features) ✅ Complete
- **v0.2.x**: Phase 2 (feature-complete drop-in replacement) ✅ Complete (v0.2.0 released)
- **v0.3.x+**: Phase 3 (advanced features, ecosystem integration) ⏳ In Progress
- **v1.0.0**: Stable API release after Phase 3 completion, production-ready ⏳ Planned

**Current Version: v0.3.0-dev** — Phase 1 and Phase 2 complete. Phase 3 in development, leading to v1.0.0 release.

---

## Success Criteria for v1.0.0

### Must Have (Blocking v1.0.0)
- ✅ Phase 1 and Phase 2 complete (achieved in v0.2.0)
- ⏳ Phase 3.1 (Query Optimization) — `explain_query_plan`, `analyze_query_plan`, streaming, `paginate`, FTS5, JSON1 done; caching, index recommendations pending
- ⏳ Phase 3.2 (Advanced Pooling) — `pool_health`, metrics, session-connection reuse done; dynamic sizing pending
- ✅ Phase 3.4 (ORM Integration) — SQLAlchemy, FastAPI, Starlette, aiohttp integration complete (3+ frameworks)
- ⏳ Phase 3.8 (Testing) — aiosqlite suite runnable; 100% pass not yet achieved
- ⏳ Phase 3.9 (API Completeness) — Helpers, Cursor props, isolation_level, __await__, `interrupt`, cursor return-self (execute/executemany/executescript) done

### Should Have (Target for v1.0.0)
- ⏳ Phase 3.3 (Advanced Transactions) — `isolation_level`, savepoints, `transaction_retry`, `transaction_with_timeout` done; deadlock detection pending
- ⏳ Phase 3.5 (Observability) — Basic monitoring and metrics
- ⏳ Phase 3.6 (Developer Experience) — Core tools and documentation
- ⏳ Phase 3.10 (Type System) — Core type conversion features complete

### Nice to Have (Post v1.0.0)
- ⏳ Phase 3.7 (Advanced Database Features) — Can be added incrementally
- ⏳ Additional framework integrations beyond core set
- ⏳ Advanced monitoring features

---

## Cross-Package Dependencies

- **Phase 1**: ✅ Independent development (complete)
- **Phase 2**: ✅ Independent development (complete)
- **Phase 3**: Potential integration with:
  - `rap-core` for shared primitives
  - `rapfiles` for database file operations
  - `rapcsv` for import/export patterns
  - Serve as database foundation for rap ecosystem

---

## Contributing

We welcome contributions! See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

**Priority Areas for Contributors**:
1. Framework integrations (FastAPI, SQLAlchemy, etc.)
2. Test coverage improvements
3. Documentation and examples
4. Performance optimizations
5. Bug fixes and compatibility improvements

---

## Notes

- **API Stability**: v0.2.0 provides a stable API for production use. Phase 3 additions will maintain backward compatibility.
- **Migration Path**: Migration from aiosqlite is straightforward with ~95% compatibility. See [migration guide](guides/migration-guide.rst) for details.
- **Performance**: rapsqlite provides true async performance with GIL-independent operations. Benchmarks available in `benchmarks/README.md`.

---

*Last Updated: 2026-01-30*
