# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, a true async SQLite library for Python built with Rust, Tokio, and sqlx.

## Current Status

**Current Version: v0.3.0-dev** 🚧  
**Phase 1: Complete** ✅  
**Phase 2: Complete** ✅  
**Phase 3: In Development** ⏳  
**Phase 4: Planned** 📋

---

## Phase 1 — Core Functionality (v0.1.x) ✅ Complete

Core functionality and production readiness:

- ✅ Connection lifecycle management (async context managers)
- ✅ Transaction support (begin, commit, rollback, transaction context managers)
- ✅ Type system (proper Python types: int, float, str, bytes, None)
- ✅ Error handling (custom exception classes matching aiosqlite)
- ✅ API compatibility (~95% aiosqlite compatibility)
- ✅ Connection pooling with configurable size and timeouts
- ✅ Input validation and security improvements
- ✅ Type stubs for IDE support

---

## Phase 2 — Feature-Complete Drop-in Replacement (v0.2.0) ✅ Complete

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

---

## Phase 3 — Advanced Features & aiosqlite Parity (v0.3.0) ⏳ In Development

**Goal**: Complete aiosqlite API compatibility and add advanced query/transaction features for the v0.3.0 release.

### Current Stats

- **Test Coverage**: 563+ tests passing (7 skipped)
- **API Compatibility**: ~95% with aiosqlite
- **aiosqlite Test Suite**: perf.py: 6/10 passing, smoke.py: 3/30 passing; intentional differences and per-test categories documented in `docs/AIOSQLITE_TEST_RESULTS.md`
- **Python Support**: 3.10–3.14
- **Code Quality**: Full mypy type checking and Ruff formatting/linting

### 3.1 API Completeness ✅ Complete

- ✅ `execute_fetchall`, `execute_insert` helper methods
- ✅ Cursor properties (`arraysize`, `connection`, `description`, `lastrowid`, `rowcount`, `row_factory`)
- ✅ `Cursor.close()`, `Cursor.execute()`/`executemany()`/`executescript()` return self
- ✅ `Connection.isolation_level` get/set
- ✅ `Connection.__await__()` support
- ✅ `Connection.interrupt()` full implementation
- ✅ `Connection.stop()` no-op for compatibility
- ✅ `total_changes` and `in_transaction` as sync properties

### 3.2 Query Helpers ✅ Complete

- ✅ `explain_query_plan(sql, parameters)` — Run EXPLAIN QUERY PLAN
- ✅ `analyze_query_plan(conn, sql, parameters)` — Structured query plan analysis
- ✅ `suggest_indexes(conn, sql, parameters)` — Index recommendations
- ✅ `paginate(conn, sql, parameters, page_size, offset)` — Page-based results
- ✅ `execute_iter(conn, sql, parameters, chunk_size)` — Streaming results
- ✅ `rows_to_dicts(rows, columns)` — Result transformation
- ✅ `in_clause_query(sql, values)` — IN clause expansion

### 3.3 Transaction Features ✅ Complete

- ✅ Savepoints (`async with db.savepoint()`)
- ✅ `transaction_with_timeout(conn, work, timeout_secs)`
- ✅ `transaction_retry(conn, work, max_retries, ...)`
- ✅ `set_slow_query_threshold(conn, threshold_secs, callback)`

### 3.4 Framework Integration ✅ Complete

- ✅ **SQLAlchemy** — `sqlite+rapsqlite` dialect
- ✅ **FastAPI** — Examples and documentation
- ✅ **Starlette** — Examples and documentation
- ✅ **aiohttp** — Examples and documentation
- ✅ **Alembic** — Migration support documented

### 3.5 SQLite Features ✅ Complete

- ✅ FTS5 full-text search support
- ✅ JSON1 extension support
- ✅ `create_function` with `deterministic` parameter

### 3.6 Connection Pooling ✅ Complete

- ✅ Session-connection reuse for performance
- ✅ `pool_health()` health check
- ✅ `pool_metrics()` for monitoring
- ✅ `idle_timeout` configuration
- ✅ `pool_metrics_gauges()` for Prometheus

### 3.7 Monitoring ✅ Complete

- ✅ `timed_fetch_all()` query timing
- ✅ `set_trace_callback` for query logging
- ✅ Slow query detection

### 3.8 Remaining for v0.3.0 Release

#### aiosqlite Compatibility (High Priority)
- ⏳ Improve aiosqlite test suite pass rate (target: >80%) — *or intentional differences documented (done)*
- ✅ Document remaining intentional differences (see `docs/AIOSQLITE_TEST_RESULTS.md` and compatibility/migration guides)
- ✅ Row format compatibility option: `connect(..., aiosqlite_compat=True)` sets default row_factory to tuple

#### Documentation (High Priority)
- ✅ Complete migration guide from aiosqlite (audit complete; "If you see test failures" and aiosqlite_compat documented)
- ✅ Best practices and anti-patterns guide (expanded in advanced-usage; connection lifecycle, blocking, transaction boundaries)
- ✅ Performance tuning guide completion (single connection vs pool, measuring performance, regression tests, cross-links)

**v0.3.0 Release Criteria**:
- aiosqlite test suite pass rate >80% **or intentional differences documented** — met via documented differences and per-test categories in AIOSQLITE_TEST_RESULTS.md
- All Phase 3 features tested and documented
- Migration guide complete
- No breaking changes from v0.2.0

---

## Phase 4 — Production Ready (v1.0.0) 📋 Planned

**Goal**: Production-grade stability, advanced tooling, and comprehensive platform support for the stable v1.0.0 release.

### 4.1 Advanced Connection Pooling

- ⏳ Dynamic pool sizing (scale up/down based on load)
- ⏳ Read/write connection separation
- ⏳ Connection routing strategies
- ⏳ Failover and recovery patterns
- ⏳ Connection state tracking and diagnostics

### 4.2 Type System Enhancements

- ⏳ `register_adapter(type, adapter)` — Python-to-SQLite type adapter
- ⏳ `register_converter(typename, converter)` — SQLite-to-Python converter
- ⏳ Date/time type handling utilities
- ⏳ UUID type support
- ⏳ Decimal type support

### 4.3 Developer Tools

- ⏳ Database introspection CLI
- ⏳ Migration generation utilities
- ⏳ Testing utilities and fixtures
- ⏳ Database mocking for tests
- ⏳ Query profiling utilities

### 4.4 Advanced Monitoring

- ⏳ Transaction tracing
- ⏳ Connection pool diagnostics
- ⏳ Performance profiling utilities
- ⏳ Resource usage tracking
- ⏳ Query execution visualization

### 4.5 Advanced Transaction Features

- ⏳ Deadlock detection and automatic retry
- ⏳ Long-running transaction monitoring
- ⏳ Transaction conflict resolution strategies

### 4.6 Platform & Testing

- ⏳ Cross-platform validation (Linux, macOS, Windows)
- ⏳ Python version matrix testing (3.10–3.14+)
- ⏳ Stress testing and performance regression tests
- ⏳ 100% aiosqlite test suite compatibility (where applicable)
- ⏳ Fake Async Detector validation under load

### 4.7 Additional Framework Integration

- ⏳ Tortoise ORM async SQLite backend
- ⏳ Peewee async SQLite support
- ⏳ Django async database backend
- ⏳ Quart async database support
- ⏳ Sanic async database patterns

### 4.8 Advanced Database Features

- ⏳ Database encryption support
- ⏳ Multi-database transaction support
- ⏳ Custom SQLite extensions support
- ⏳ Enhanced backup and restore utilities
- ⏳ Schema validation tools
- ⏳ Bulk operation optimizations
- ⏳ Window functions utilities
- ⏳ CTE utilities

**v1.0.0 Release Criteria**:
- Phase 3 complete
- All Phase 4 "Must Have" features implemented
- Cross-platform CI passing
- Performance benchmarks meet targets
- Comprehensive documentation
- Production stability validated

---

## Versioning Strategy

Following semantic versioning:

| Version | Phase | Status |
|---------|-------|--------|
| v0.1.x | Phase 1 — Core functionality | ✅ Complete |
| v0.2.x | Phase 2 — Feature-complete drop-in | ✅ Complete (v0.2.0 released) |
| v0.3.x | Phase 3 — Advanced features & aiosqlite parity | ⏳ In Progress |
| v1.0.0 | Phase 4 — Production ready | 📋 Planned |

**Current Version: v0.3.0-dev**

---

## v1.0.0 Release Requirements

### Must Have (Blocking)
- ✅ Phase 1 and Phase 2 complete
- ⏳ Phase 3 complete (v0.3.0 released)
- ⏳ Type system: `register_adapter` and `register_converter`
- ⏳ Cross-platform CI (Linux, macOS, Windows)
- ⏳ Performance regression tests

### Should Have (Target)
- ⏳ Dynamic pool sizing
- ⏳ CLI tools for introspection
- ⏳ Advanced monitoring features
- ⏳ Additional ORM integrations

### Nice to Have (Post v1.0.0)
- ⏳ Database encryption
- ⏳ Schema migration generation
- ⏳ Query execution visualization

---

## Cross-Package Dependencies

- **Phase 1–2**: ✅ Independent development (complete)
- **Phase 3–4**: Potential integration with:
  - `rap-core` for shared primitives
  - `rapfiles` for database file operations
  - `rapcsv` for import/export patterns
  - Serve as database foundation for rap ecosystem

---

## Contributing

We welcome contributions! See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

**Priority Areas for Contributors**:
1. aiosqlite compatibility improvements
2. Test coverage improvements
3. Documentation and examples
4. Framework integrations
5. Performance optimizations

---

## Notes

- **API Stability**: v0.2.0+ provides a stable API for production use. Phase 3 and 4 additions maintain backward compatibility.
- **Migration Path**: Migration from aiosqlite is straightforward with ~95% compatibility. See [migration guide](guides/migration-guide.rst) for details.
- **Performance**: rapsqlite provides true async performance with GIL-independent operations. Benchmarks available in `benchmarks/README.md`.

---

*Last Updated: 2026-01-31*
