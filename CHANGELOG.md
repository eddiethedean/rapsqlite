# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning Strategy

- **v0.1.x**: Phase 1 development (MVP and core features)
- **v0.2.x**: Phase 2 development and release (feature-complete drop-in replacement) - **Current: v0.2.0**
- **v0.3.x+**: Phase 3 development (advanced features, ecosystem integration)
- **v1.0.0**: Stable API release after Phase 3 completion, production-ready

## [1.0.0] - TBA (After Phase 3 Completion)

### Overview

- v1.0.0 will be released after Phase 3 completion, marking production stability
- Phase 3 includes advanced features, ecosystem integration, and optimizations
- See Phase 3 roadmap for planned features leading to v1.0.0

### Checklist for v1.0 Release (Future)

- Phase 3 features complete
- All tests passing across supported Python versions (3.8–3.14)
- Comprehensive documentation and examples
- Ecosystem integrations validated
- Performance benchmarks meet targets
- Production stability validated

_Note: v1.0.0 release details will be added after Phase 3 completion._

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

- Python 3.8 through 3.14 supported
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
