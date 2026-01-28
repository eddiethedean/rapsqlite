# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, aligned with the [RAP Project Strategic Plan](../rap-project-plan.md). `rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx.

## Current Status

**Current Version (v0.2.0)** — Phase 1 Complete, Phase 2 Complete:

**Phase 1 Complete:**
- ✅ Connection lifecycle management (async context managers)
- ✅ Transaction support (begin, commit, rollback)
- ✅ Type system improvements (proper Python types: int, float, str, bytes, None)
- ✅ Enhanced error handling (custom exception classes matching aiosqlite)
- ✅ API improvements (fetch_one, fetch_optional, execute_many, last_insert_rowid, changes)
- ✅ Cursor API (execute, executemany, fetchone, fetchall, fetchmany)
- ✅ aiosqlite compatibility (connect function, exception types)
- ✅ Connection pooling (basic implementation with reuse)
- ✅ Input validation and security improvements
- ✅ Type stubs for IDE support

**Phase 2 Complete (v0.2.0):**
- ✅ **Phase 2.1 Complete**: Parameterized queries (named and positional parameters, execute_many with binding)
- ✅ **Phase 2.2 Complete**: Cursor improvements (fetchmany size-based slicing, result caching, state management)
- ✅ **Phase 2.3 Complete**: Connection configuration (PRAGMA settings, connection string parsing, constructor parameters)
- ✅ **Phase 2.4 Complete**: Pool configuration (`pool_size` and `connection_timeout` getters/setters; used when creating pool; `tests/test_pool_config.py`)
- ✅ **Phase 2.5 Complete**: Row factory compatibility (`Connection.row_factory` getter/setter; dict/tuple/callable; fetch_* and Cursor; `tests/test_row_factory.py`)
- ✅ **Phase 2.6 Complete**: Transaction context managers (`Connection.transaction()` async CM, execute_many in transactions, fetch_* use transaction connection)
- ✅ **Phase 2.7 Complete**: Advanced SQLite callbacks (enable_load_extension, set_progress_handler, create_function, set_trace_callback, set_authorizer)
- ✅ **Phase 2.8 Complete**: Database dump (`Connection.iterdump()`)
- ✅ **Phase 2.9 Complete**: Database backup (`Connection.backup()`)
- ✅ **Phase 2.10 Complete**: Schema operations and introspection (`get_tables`, `get_table_info`, `get_indexes`, `get_foreign_keys`, `get_schema`, `get_views`, `get_index_list`, `get_index_info`, `get_table_xinfo`)
- ✅ **Phase 2.11 Complete**: Database initialization hooks (`init_hook` parameter for automatic schema setup and data seeding)
- ✅ **Phase 2.12 Complete**: Bug fixes & compatibility (set_pragma fix, README updates)
- ✅ **Phase 2.13 Complete**: Prepared statements & performance optimization — Verified and documented sqlx prepared statement caching, added performance tests
- ✅ **Phase 2.14 Complete**: Drop-in replacement validation (aiosqlite compatibility features, migration guide) — All high-priority compatibility features implemented
- ✅ **Phase 2.15 Complete**: Documentation & benchmarking — Benchmarks documented, advanced usage guide enhanced, all documentation updated

**Phase 3 Status:** In Planning — Advanced features, ecosystem integration, and optimizations leading to v1.0.0 release

**Goal**: Achieve drop-in replacement compatibility with `aiosqlite` to enable seamless migration with true async performance.

**Recent progress:** Phase 2 complete and released as v0.2.0. All high-priority compatibility features implemented including `total_changes()`, `in_transaction()`, `executescript()`, `load_extension()`, `text_factory`, `Row` class, and async iteration on cursors. Core API compatibility at ~95%. Migration guide and compatibility analysis updated. Fixed critical deadlock issue in `init_hook` when used with `begin()` and `transaction()` context managers. All tests passing (356 passed, 6 skipped). Database initialization hooks, schema operations, prepared statement caching, and comprehensive documentation all complete. Code quality: full mypy type checking and Ruff formatting/linting. **Phase 2 released as v0.2.0. Phase 3 planning in progress, leading to v1.0.0 release.**

## Phase 1 — Credibility

Focus: Fix critical performance issues, add essential features for production use.

### Connection Management

- **Connection pooling** ✅ (complete - basic implementation)
  - ✅ Implement proper connection pool with configurable size (basic implementation)
  - ✅ Connection reuse across operations
  - ✅ Efficient pool initialization and shutdown (lazy initialization)
  - ⏳ Pool lifecycle management (advanced features - Phase 3)
  - ⏳ Connection health checking and recovery (Phase 3)

- **Connection lifecycle** ✅ (complete - basic implementation)
  - ✅ Context manager support (`async with`)
  - ✅ Explicit connection management
  - ✅ Proper resource cleanup
  - ⏳ Connection state tracking (Phase 3)
  - ⏳ Connection timeout handling (Phase 3)

- **Performance fixes** ✅ (complete)
  - ✅ Eliminate per-operation pool creation overhead
  - ✅ Efficient connection acquisition and release
  - ✅ Minimize connection churn

### Transaction Support

- **Basic transactions** ✅ (complete - basic implementation)
  - ✅ `begin()`, `commit()`, `rollback()` methods
  - ✅ Transaction state tracking
  - ✅ Transaction context managers (`Connection.transaction()`, execute_many and fetch_* in transactions)
  - ⏳ Nested transaction handling (savepoints) (Phase 3)
  - ⏳ Transaction isolation level configuration (Phase 3)

- **Error handling in transactions** ✅ (complete - basic implementation)
  - ✅ Automatic rollback on connection close
  - ✅ Transaction state management
  - ⏳ Deadlock detection and handling (Phase 3)

### Type System Improvements

- **Better type handling** ✅ (complete)
  - ✅ Preserve SQLite types (INTEGER, REAL, TEXT, BLOB, NULL)
  - ✅ Type conversion to Python types (int, float, str, bytes, None)
  - ✅ Binary data (BLOB) support
  - ⏳ Optional type hints for Python types (Phase 3)
  - ⏳ Type conversion utilities (Phase 3)

- **Return value improvements** ✅ (complete - basic implementation)
  - ✅ Return proper Python types where appropriate
  - ⏳ Configurable type conversion (Phase 3)
  - ⏳ Type inference from schema (Phase 3)
  - ⏳ Date/time type handling (Phase 3)

### Enhanced Error Handling

- **SQL-specific errors** ✅ (complete)
  - ✅ SQL syntax error detection and reporting
  - ✅ Constraint violation errors (IntegrityError)
  - ✅ Better error messages with SQL context
  - ✅ Error code mapping to Python exceptions
  - ⏳ Database locked errors with context (basic support, enhanced - Phase 3)

- **Connection errors** ✅ (complete - basic implementation)
  - ✅ Database file errors
  - ✅ Permission errors (via OperationalError)
  - ⏳ Connection timeout errors (Phase 3)
  - ⏳ Recovery strategies (Phase 3)

### API Improvements

- **Query methods** ✅ (complete)
  - ✅ `fetch_one()` - fetch single row
  - ✅ `fetch_optional()` - fetch one row or None
  - ✅ `execute_many()` - execute multiple parameterized statements (works in transactions; list-of-tuples/list-of-lists)
  - ✅ `last_insert_rowid()` - get last insert ID
  - ✅ `changes()` - get number of affected rows

- **API stability** ✅ (complete - production-ready)
  - ✅ Consistent error handling patterns
  - ✅ Resource management guarantees
  - ⏳ Thread-safety documentation (Phase 3)
  - ✅ Performance characteristics documented (Phase 2.15 complete)

### API Compatibility for Drop-In Replacement

- **aiosqlite API compatibility** ✅ (core API complete - production-ready)
  - ✅ Match `aiosqlite.Connection` core API
  - ✅ Match `aiosqlite.Cursor` core API
  - ✅ Compatible connection factory pattern (`connect()`)
  - ✅ Matching method signatures (`execute()`, `executemany()`, `fetchone()`, `fetchall()`, `fetchmany()`)
  - ✅ Compatible transaction methods (`commit()`, `rollback()`, `begin()`)
  - ✅ Matching exception types (`Error`, `Warning`, `DatabaseError`, `OperationalError`, `ProgrammingError`, `IntegrityError`)
  - ✅ Compatible context manager behavior for connections and cursors
  - ✅ Optional `parameters` on `execute` / `fetch_*` / `Cursor.execute` (PyO3 `signature`; aiosqlite compat)
  - ✅ Row factory compatibility (`row_factory` parameter) (Phase 2.5 complete)
  - ✅ Drop-in replacement validation: `import rapsqlite as aiosqlite` compatibility tests (Phase 2.14 complete)

- **Migration support** ✅ (Phase 2.14 complete)
  - ⏳ Compatibility shim/adapter layer if needed for exact API matching (not needed - ~95% compatibility achieved)
  - ✅ Migration guide documenting any differences (Phase 2.14 complete)
  - ✅ Backward compatibility considerations (Phase 2.14 complete)
  - ✅ Support for common aiosqlite patterns and idioms (Phase 2.14 complete)

### Testing & Validation

- **Testing** ✅ (complete - comprehensive test suite)
  - ✅ Comprehensive test suite covering core features
  - ✅ Type conversion tests
  - ✅ Transaction tests
  - ✅ Error handling tests
  - ✅ Cursor API tests
  - ✅ Context manager tests
  - ⏳ Complete edge case coverage (Phase 3)
  - ⏳ Fake Async Detector validation passes under load (Phase 3)
  - ⏳ Pass 100% of aiosqlite test suite as drop-in replacement validation (Phase 3)
  - ✅ Drop-in replacement compatibility tests (Phase 2.14 complete)
  - ✅ Benchmark comparison with existing async SQLite libraries (Phase 2.15 complete)
  - ✅ Documentation improvements including migration guide (Phase 2.15 complete)

## Phase 2 — Expansion

Focus: Feature additions, performance optimizations, and broader SQLite feature support.

**Current Phase 2 Status**: **Phase 2 Complete (100%) - Released as v0.2.0** ✅ All phases 2.1-2.15 complete. All core features implemented including parameterized queries, cursor improvements, connection/pool configuration, row factory, transaction context managers, advanced callbacks, database dump, backup, comprehensive schema introspection, database initialization hooks, prepared statement caching verification, and comprehensive documentation. Fixed critical deadlock issue in `init_hook` with `begin()` and `transaction()` context managers. Each phase has dedicated robust test suites. Code quality: full mypy type checking and Ruff formatting/linting. Test suite: 356+ passing tests (6 skipped). Benchmarks documented. **Phase 2 released as v0.2.0. Phase 3 will lead to v1.0.0 release.**

### Path to Phase 2 Completion

**Progress: 15/15 phases complete (100%) - Phase 2 Complete! ✅**

Phase 2 is now complete. All work to achieve full drop-in replacement compatibility with `aiosqlite` has been finished:

#### Phase 2.12: Bug Fixes & Compatibility (High Priority - Quick Wins) ✅

**Estimated effort: 1-2 days**

1. **Fix `set_pragma` test expectation** ✅
   - **Issue**: `test_set_pragma` expects incorrect value (2 instead of 1 for NORMAL)
   - **Fix**: Update test assertion to match SQLite semantics (NORMAL = 1)
   - **Plan**: [docs/PLAN_OPTION_B_SET_PRAGMA_FIX.md](PLAN_OPTION_B_SET_PRAGMA_FIX.md)
   - **Impact**: Resolves compatibility test failure, improves PRAGMA behavior documentation
   - **Status**: Test already fixed and passing

2. **Update README limitations** ✅
   - Remove outdated limitations (parameterized queries, fetchmany already complete)
   - Update status to reflect Phase 2.1-2.11 completion
   - Clarify remaining gaps vs. completed features
   - **Status**: README updated with accurate limitations and Phase 2.1-2.11 improvements

**Success criteria**: ✅ All compatibility tests pass, documentation accurately reflects current state

#### Phase 2.13: Prepared Statements & Performance (Medium Priority) ✅

**Estimated effort: 3-5 days**  
**Status: Complete (2026-01-26)**

1. **Statement preparation and caching** ✅
   - ✅ Verified sqlx's automatic prepared statement caching per connection
   - ✅ Documented how sqlx handles prepared statement reuse
   - ✅ Added comprehensive documentation in code and ADVANCED.md
   - ✅ Query normalization ensures maximum cache hit rates
   - **Result**: sqlx automatically caches prepared statements - no additional implementation needed

2. **Performance testing and validation** ✅
   - ✅ Created comprehensive test suite (`tests/test_prepared_statements.py`)
   - ✅ Added performance comparison tests (repeated vs unique queries)
   - ✅ Verified prepared statement reuse through performance testing
   - ✅ Tests demonstrate 2-5x performance improvement for repeated queries
   - **Result**: Prepared statements verified to be cached and reused automatically

**Success criteria**: ✅ All met
- ✅ Prepared statements cached and reused for identical queries (verified via sqlx)
- ✅ Performance tests show improvement for repeated queries
- ✅ Memory usage remains reasonable (sqlx handles cache management internally)
- ✅ Tests demonstrate prepared statement reuse

#### Phase 2.14: Drop-In Replacement Validation (High Priority - Critical for v1.0) ✅

**Estimated effort: 5-7 days**  
**Status: Complete (2026-01-26)**

1. **aiosqlite compatibility features** ✅
   - ✅ Implemented `Connection.total_changes()` method
   - ✅ Implemented `Connection.in_transaction()` method
   - ✅ Implemented `Cursor.executescript()` method
   - ✅ Implemented `Connection.load_extension(name)` method
   - ✅ Implemented `Connection.text_factory` property
   - ✅ Created `rapsqlite.Row` class with dict-like access
   - ✅ Implemented async iteration on cursors (`async for row in cursor`)
   - ✅ Enhanced `async with db.execute(...)` pattern support
   - **Result**: Core API compatibility increased from ~85% to ~95%

2. **Comprehensive compatibility validation** ✅
   - ✅ `import rapsqlite as aiosqlite` compatibility validated
   - ✅ All major aiosqlite patterns and idioms supported
   - ✅ Edge case coverage in compatibility tests
   - ✅ Type stubs complete for all new APIs
   - **Result**: Verified seamless migration path

3. **Migration guide and documentation** ✅
   - ✅ Migration guide updated with all new features
   - ✅ Compatibility analysis document updated
   - ✅ API differences documented
   - ✅ Code examples provided for all patterns
   - **Result**: Easy migration enabled for existing projects

**Success criteria**: ✅ All met
- ✅ All high-priority compatibility features implemented
- ✅ Migration guide published and updated
- ✅ Compatibility analysis demonstrates ~95% compatibility
- ✅ Type stubs complete
- ✅ All new features tested and documented

#### Phase 2.15: Documentation & Benchmarking (Medium Priority) ✅

**Estimated effort: 3-4 days**  
**Status: Complete (2026-01-26)**

1. **Performance benchmarking** ✅
   - ✅ Ran comprehensive benchmark suite
   - ✅ Documented actual results in `benchmarks/README.md`
   - ✅ Added system information and performance analysis
   - ✅ Documented throughput and latency metrics
   - ✅ Concurrent operation benchmarks documented
   - ✅ Transaction performance analysis completed
   - **Result**: Benchmarks published showing rapsqlite performance characteristics

2. **Documentation improvements** ✅
   - ✅ Enhanced `docs/ADVANCED.md` with prepared statement caching details
   - ✅ Added performance tuning best practices
   - ✅ Updated `README.md` with benchmark summary and complete feature list
   - ✅ Enhanced prepared statement caching documentation
   - ✅ All major features documented with examples
   - **Result**: Production-ready documentation available

**Success criteria**: ✅ All met
- ✅ Benchmarks published showing performance characteristics
- ✅ Comprehensive documentation available covering all features
- ✅ Examples cover common use cases and advanced patterns
- ✅ Performance characteristics documented
- ✅ Best practices and anti-patterns documented

### Phase 2 Completion Checklist

- [x] Phase 2.1: Parameterized queries
- [x] Phase 2.2: Cursor improvements
- [x] Phase 2.3: Connection configuration
- [x] Phase 2.4: Pool configuration
- [x] Phase 2.5: Row factory compatibility
- [x] Phase 2.6: Transaction context managers
- [x] Phase 2.7: Advanced SQLite callbacks
- [x] Phase 2.8: Database dump
- [x] Phase 2.9: Database backup
- [x] Phase 2.10: Schema operations and introspection
- [x] Phase 2.11: Database initialization hooks
- [x] Phase 2.12: Bug fixes & compatibility (set_pragma fix, README updates)
- [x] Phase 2.13: Prepared statements & performance ✅ Complete (2026-01-26)
- [x] Phase 2.14: Drop-in replacement validation (compatibility tests, migration guide) ✅ Complete (2026-01-26)
- [x] Phase 2.15: Documentation & benchmarking ✅ Complete (2026-01-26)

### Recommended Implementation Order

1. **Phase 2.12** (Quick wins) → Fixes compatibility issues, updates documentation
2. **Phase 2.14** (Critical for v1.0) → Validates drop-in replacement, enables migration
3. **Phase 2.13** (Performance) → Optimizes for production use
4. **Phase 2.15** (Documentation) → Completes user-facing materials

**Phase 2 (v0.2.0) Release**: Phase 2.1-2.15 are complete and provide a production-ready, drop-in replacement for `aiosqlite` with ~95% API compatibility. **Released as v0.2.0.** Remaining enhancement items are planned for Phase 3, which will lead to the v1.0.0 release.

### Phase 2 features to implement (from compat gaps)

Prioritized from `test_aiosqlite_compat` failures and execute_many fix work:

- **2.4 — `pool_size` getter/setter** (✅ Complete): `test_pool_size_getter_setter`, `test_pool_size_edge_cases`, `test_pool_configuration_*`; robust suite `tests/test_pool_config.py` (18 tests)
- **2.4 — `connection_timeout` getter/setter** (✅ Complete): `test_connection_timeout_*`, `test_pool_configuration_*`; covered in `test_pool_config.py`
- **2.5 — `row_factory` getter/setter** (✅ Complete): `test_connection_properties`, `test_row_factory_comprehensive`, `tests/test_row_factory.py`
- **2.7 — `enable_load_extension()`** (✅ Complete): `test_enable_load_extension*`, callback-combo tests
- **2.7 — `create_function()` / `remove_function()`** (✅ Complete): `test_create_function*`, `test_custom_function_*`
- **2.7 — `set_trace_callback()`** (✅ Complete): `test_set_trace_callback*`, `test_trace_callback_with_errors`, etc.
- **2.7 — `set_authorizer()`** (✅ Complete): `test_set_authorizer*`, `test_authorizer_action_codes`, etc.
- **2.7 — `set_progress_handler()`** (✅ Complete): `test_set_progress_handler*`, `test_all_callbacks_together`
- **2.8 — `iterdump()`** (✅ Complete): `test_iterdump`, comprehensive iterdump tests in `test_callback_robustness.py`
- **2.8+ — `set_pragma` behavior** (⏳ Investigate): `test_set_pragma` (assertion `1 == 2`); PRAGMA application or return-value mismatch. **Plan:** [docs/PLAN_OPTION_B_SET_PRAGMA_FIX.md](PLAN_OPTION_B_SET_PRAGMA_FIX.md)
- **Phase 2.7 callbacks** (✅ Complete): All callbacks implemented (enable_load_extension, create_function, set_trace_callback, set_authorizer, set_progress_handler). **Plan:** [docs/PLAN_OPTION_A_PHASE_2_7_CALLBACKS.md](PLAN_OPTION_A_PHASE_2_7_CALLBACKS.md)

### Prepared Statements & Parameterized Queries

- **Prepared statements** ✅ (Phase 2.13 complete)
  - ✅ Statement preparation and caching (Phase 2.13 - verified sqlx automatic caching)
  - ✅ Parameter binding (named and positional) - complete
  - ✅ Efficient statement reuse (Phase 2.13 - verified via sqlx)
  - ✅ Statement pool management (Phase 2.13 - handled by sqlx per connection)

- **Parameterized queries** ✅ (Phase 2.1 complete)
  - ✅ Named parameters (`:name`, `@name`, `$name`) - complete
  - ✅ Positional parameters (`?`, `?1`, `?2`) - complete
  - ✅ Type-safe parameter binding - complete
  - ✅ `execute_many()` in transactions (list-of-tuples/list-of-lists) - complete
  - ⏳ Array parameter binding for IN clauses (basic support via lists, enhanced - Phase 3)
  - ✅ Complete `execute_many()` implementation with parameter binding - complete

- **Query building utilities**
  - Helper functions for common query patterns
  - Query result mapping utilities
  - Optional ORM-like convenience methods

- **Cursor improvements** ✅ (Phase 2.2 complete)
  - ✅ Complete `fetchmany()` size-based slicing implementation - complete
  - ✅ Cursor state management improvements (results caching, index tracking) - complete
  - ✅ Cursor methods support parameterized queries - complete

### Advanced SQLite Features

- **Phase 2.7 callbacks** ✅ (complete)
  - ✅ `enable_load_extension()` — `test_enable_load_extension*`
  - ✅ `create_function()` / `remove_function()` — `test_create_function*`, `test_custom_function_*`
  - ✅ `set_trace_callback()` — `test_set_trace_callback*`, etc.
  - ✅ `set_authorizer()` — `test_set_authorizer*`, etc.
  - ✅ `set_progress_handler()` — `test_set_progress_handler*`, etc.
- **iterdump** ✅ (Phase 2.8 complete): `Connection.iterdump()` — `test_iterdump`, comprehensive tests in `test_callback_robustness.py`
- **backup** ✅ (Phase 2.9 complete): `Connection.backup()` — comprehensive backup tests including rapsqlite-to-rapsqlite backup
- **SQLite-specific features**
  - Full-text search (FTS) support
  - JSON functions support
  - Window functions
  - Common Table Expressions (CTEs)
  - UPSERT operations (INSERT OR REPLACE, etc.)

- **Schema operations** ✅ (Phase 2.10 complete)
  - ✅ Schema introspection (tables, columns, indexes, views) - `get_tables()`, `get_table_info()`, `get_indexes()`, `get_foreign_keys()`, `get_schema()`
  - ✅ Extended schema introspection - `get_views()`, `get_index_list()`, `get_index_info()`, `get_table_xinfo()`
  - ✅ View introspection - `get_views()` for listing and filtering views
  - ✅ Index details - `get_index_list()` using PRAGMA index_list (seq, name, unique, origin, partial)
  - ✅ Index column information - `get_index_info()` using PRAGMA index_info (seqno, cid, name)
  - ✅ Extended table information - `get_table_xinfo()` using PRAGMA table_xinfo (includes hidden column flag)
  - ⏳ Migration utilities (Phase 3)
  - ⏳ Schema validation (Phase 3)
  - ✅ Foreign key constraint support - `get_foreign_keys()` implemented

- **Performance features**
  - Index recommendations
  - Query plan analysis
  - WAL mode configuration
  - Journal mode configuration
  - Synchronous mode configuration

### Connection Configuration

- **Database configuration** ✅ (Phase 2.3 complete)
  - ✅ PRAGMA settings support (`set_pragma()` method) - complete
  - ✅ Connection string support (URI format: `file:path?param=value`) - complete
  - ✅ PRAGMA settings via constructor parameter - complete
  - ⏳ `set_pragma` behavior investigation — `test_set_pragma` fails (assertion `1 == 2`); possible PRAGMA application or return-value mismatch (Phase 3)
  - ✅ Database initialization hooks (Phase 2.11 complete: `init_hook` parameter for automatic schema setup and data seeding)
  - ⏳ Custom SQLite extensions (if applicable) (Phase 3)

- **Pool configuration** ✅ (Phase 2.4 complete)
  - ✅ Configurable pool size (infrastructure and getter/setter)
  - ✅ Connection timeout settings (infrastructure and getter/setter; acquire_timeout when creating pool)
  - ✅ `pool_size` getter/setter
  - ✅ `connection_timeout` getter/setter
  - ✅ Robust test suite `tests/test_pool_config.py` (validation, config-before-use, transaction/cursor/set_pragma/execute_many/begin, zero edge cases, multiple connections, large values)
  - ⏳ Idle connection management (Phase 3)
  - ⏳ Pool monitoring and metrics (Phase 3)
  - ⏳ Pool lifecycle management (advanced features from Phase 1) (Phase 3)

### Concurrent Operations

- **Concurrent query execution**
  - Efficient concurrent reads
  - Write queue management for writes
  - Read-only connection optimization
  - Concurrent transaction handling

- **Batch operations**
  - Bulk insert operations
  - Batch transaction processing
  - Efficient multi-statement execution
  - Progress tracking for long operations

### Performance & Benchmarking

- **Performance optimizations**
  - Query result streaming for large result sets
  - Efficient memory usage patterns
  - Connection pooling optimizations
  - Statement caching strategies

- **Benchmarking**
  - Comparison with `aiosqlite`, `sqlite3`, other async SQLite libraries
  - Throughput and latency metrics
  - Concurrent operation benchmarks
  - Transaction performance analysis

### Compatibility & Integration

- **Additional API compatibility**
  - Maintain and refine aiosqlite drop-in replacement (core API achieved in Phase 1)
  - Enhanced compatibility features beyond core aiosqlite API
  - ✅ Row factory compatibility (Phase 2.5 complete: `row_factory` getter/setter, dict/tuple/callable, `tests/test_row_factory.py`)
  - Migration guides from other libraries (sqlite3, etc.)
  - Compatibility shims for common patterns and idioms
  - ✅ Python 3.13 support (wheels and CI builds) - complete in v0.1.1
  - ✅ Python 3.14 support (wheels and CI builds) - complete

- **Framework integration**
  - Integration examples with web frameworks
  - ORM integration patterns (SQLAlchemy, Tortoise ORM, Peewee)
  - Database migration tool integration (Alembic)
  - Testing framework integration (pytest-asyncio patterns)

## Phase 3 — Ecosystem (v0.3.0 → v1.0.0)

Focus: Advanced features, ecosystem integration, and query optimization leading to the v1.0.0 stable release.

**Versioning Strategy:**
- Phase 2 features released as **v0.2.0** (current)
- Phase 3 features will be released incrementally as **v0.3.0, v0.4.0, etc.**
- **v1.0.0** will be released after Phase 3 is complete, marking production stability

### Advanced Query Features

- **Query optimization**
  - ⏳ Query plan analysis and optimization hints
  - ⏳ Automatic index recommendations
  - ⏳ Query result caching strategies
  - ⏳ Lazy query execution patterns
  - ⏳ Index recommendations
  - ⏳ WAL mode configuration
  - ⏳ Journal mode configuration
  - ⏳ Synchronous mode configuration

- **Advanced result handling**
  - ⏳ Streaming query results for large datasets
  - ⏳ Cursor-based pagination
  - ⏳ Result set transformation utilities
  - ⏳ Row-to-object mapping helpers
  - ⏳ Efficient memory usage patterns

- **Query building utilities**
  - ⏳ Helper functions for common query patterns
  - ⏳ Query result mapping utilities
  - ⏳ Optional ORM-like convenience methods

- **SQLite-specific features**
  - ⏳ Full-text search (FTS) support
  - ⏳ JSON functions support
  - ⏳ Window functions
  - ⏳ Common Table Expressions (CTEs)
  - ⏳ UPSERT operations (INSERT OR REPLACE, etc.)

### Async-Safe Connection Pooling

- **Advanced pooling**
  - ⏳ Dynamic pool sizing
  - ⏳ Connection health monitoring
  - ⏳ Automatic pool scaling
  - ⏳ Cross-process connection sharing patterns (if applicable)
  - ⏳ Pool lifecycle management (advanced features)
  - ⏳ Connection health checking and recovery
  - ⏳ Idle connection management
  - ⏳ Pool monitoring and metrics
  - ⏳ Connection pooling optimizations

- **Connection management**
  - ⏳ Read/write connection separation
  - ⏳ Replication patterns (read replicas)
  - ⏳ Connection routing strategies
  - ⏳ Failover and recovery patterns
  - ⏳ Connection state tracking
  - ⏳ Connection timeout handling (enhanced error handling)
  - ⏳ Recovery strategies

- **Transaction features**
  - ⏳ Nested transaction handling (savepoints)
  - ⏳ Transaction isolation level configuration
  - ⏳ Deadlock detection and handling

### Ecosystem Adapters

- **ORM integration**
  - ⏳ SQLAlchemy async driver support
  - ⏳ Tortoise ORM async SQLite backend
  - ⏳ Peewee async SQLite support
  - ⏳ Custom ORM adapters
  - ⏳ Query builder integrations
  - ⏳ Migration framework support (Alembic, etc.)

- **Framework integrations**
  - ⏳ FastAPI database dependencies
  - ⏳ Django async database backend (if applicable)
  - ⏳ aiohttp database patterns
  - ⏳ Starlette async database integration
  - ⏳ Quart async database support
  - ⏳ Sanic async database patterns
  - ⏳ Background task queue integration (Celery, RQ, Dramatiq)
  - ⏳ Testing utilities (pytest-asyncio fixtures and patterns)

### Integration & Tooling

- **rap-core integration**
  - ⏳ Shared primitives with other rap packages
  - ⏳ Common database patterns
  - ⏳ Unified error handling
  - ⏳ Performance monitoring hooks

- **Developer tools**
  - ⏳ Query logging and profiling
  - ⏳ Database introspection tools
  - ⏳ Migration generation utilities
  - ⏳ Testing utilities and fixtures
  - ⏳ Statement caching strategies

### Observability & Monitoring

- **Monitoring & metrics**
  - ⏳ Performance metrics export
  - ⏳ Query timing and profiling
  - ⏳ Connection pool metrics
  - ⏳ Resource usage tracking
  - ⏳ Slow query detection and reporting

- **Debugging tools**
  - ⏳ SQL query logging
  - ⏳ Transaction tracing
  - ⏳ Connection pool diagnostics
  - ⏳ Performance profiling utilities

### Advanced Features

- **Database features**
  - ⏳ Backup and restore utilities (basic backup exists, enhanced utilities planned)
  - ⏳ Database encryption support (if applicable)
  - ⏳ Replication patterns
  - ⏳ Multi-database transaction support
  - ⏳ Custom SQLite extensions (if applicable)
  - ⏳ `set_pragma` behavior investigation — `test_set_pragma` fails (assertion `1 == 2`); possible PRAGMA application or return-value mismatch

- **Type system & conversion**
  - ⏳ Optional type hints for Python types
  - ⏳ Type conversion utilities
  - ⏳ Configurable type conversion
  - ⏳ Type inference from schema
  - ⏳ Date/time type handling

- **Parameterized queries**
  - ⏳ Array parameter binding for IN clauses (enhanced - basic support via lists exists)

- **Schema operations**
  - ⏳ Migration utilities
  - ⏳ Schema validation

- **Error handling**
  - ⏳ Database locked errors with context (enhanced)
  - ⏳ Connection timeout errors (enhanced)

- **Testing & Development**
  - ⏳ In-memory database support (basic support exists, enhanced planned)
  - ⏳ Testing utilities and fixtures
  - ⏳ Database mocking for tests
  - ⏳ Migration testing tools
  - ⏳ Complete edge case coverage
  - ⏳ Fake Async Detector validation passes under load
  - ⏳ Pass 100% of aiosqlite test suite as drop-in replacement validation

### Documentation & Community

- **Comprehensive documentation**
  - ⏳ Advanced usage patterns and examples
  - ⏳ Performance tuning guides
  - ⏳ Migration documentation from other libraries
  - ⏳ Best practices and anti-patterns
  - ⏳ Contributing guidelines
  - ⏳ Thread-safety documentation

- **Ecosystem presence**
  - ⏳ PyPI package optimization
  - ⏳ CI/CD pipeline improvements
  - ⏳ Community examples and tutorials
  - ⏳ Blog posts and case studies
  - ⏳ Conference talks and presentations

### Concurrent Operations

- **Concurrent query execution**
  - ⏳ Efficient concurrent reads
  - ⏳ Write queue management for writes
  - ⏳ Read-only connection optimization
  - ⏳ Concurrent transaction handling

- **Batch operations**
  - ⏳ Bulk insert operations
  - ⏳ Batch transaction processing
  - ⏳ Efficient multi-statement execution
  - ⏳ Progress tracking for long operations

## Cross-Package Dependencies

- **Phase 1**: ✅ Independent development, minimal dependencies (complete)
- **Phase 2**: Potential integration with `rapfiles` for database file operations, `rapcsv` for import/export patterns
- **Phase 3**: Integration with `rap-core` for shared primitives, serve as database foundation for rap ecosystem

## Success Criteria

- **Phase 1**: ✅ Connection pooling implemented, ✅ transactions supported, ✅ stable API, ✅ **core aiosqlite API compatibility** (production-ready), ✅ comprehensive test suite, ✅ passes Fake Async Detector. Advanced features (parameterized queries, row factory, transaction context managers) moved to Phase 2.
- **Phase 2 (v0.2.0)**: ✅ Feature-complete for common SQLite use cases, competitive performance benchmarks, excellent documentation, seamless migration from aiosqlite, complete drop-in replacement with advanced features
- **Phase 3 (v0.3.0+)**: Industry-leading performance, ecosystem integration, adoption as primary async SQLite library for Python and preferred aiosqlite alternative, leading to **v1.0.0 stable release**

## Versioning Strategy

Following semantic versioning:
- `v0.1.x`: Phase 1 development (MVP and core features)
- `v0.2.x`: Phase 2 development and release (feature-complete drop-in replacement)
- `v0.3.x+`: Phase 3 development (advanced features, ecosystem integration)
- `v1.0.0`: Stable API release after Phase 3 completion, production-ready
- `v1.x+`: Backwards-compatible additions and enhancements

**Current Version: v0.2.0** — Phase 1 complete; **Phase 2 complete (100%)** ✅. All phases 2.1-2.15 complete. Core features: parameterized queries, cursor improvements, connection/pool configuration, row factory, transaction context managers, advanced callbacks, database dump, backup, comprehensive schema introspection, database initialization hooks, aiosqlite compatibility completion, prepared statement caching verification, comprehensive documentation and benchmarking. Fixed critical deadlock issue in `init_hook` with `begin()` and `transaction()` context managers. Core aiosqlite API compatibility at ~95% (increased from ~85%). All high-priority compatibility features implemented including `total_changes()`, `in_transaction()`, `executescript()`, `load_extension()`, `text_factory`, `Row` class, and async iteration. Prepared statement caching verified and documented. Benchmarks documented with actual results. Python 3.8–3.14 supported. 356+ tests passing (6 skipped). Full mypy type checking and Ruff formatting/linting. **Phase 2 released as v0.2.0. Phase 3 will lead to v1.0.0 release.**