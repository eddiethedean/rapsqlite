# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, aligned with the [RAP Project Strategic Plan](../rap-project-plan.md). `rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx.

## Current Status

**Current Version (v0.0.2)** - Current limitations:

- No transaction support
- No prepared statements or parameterized queries
- Limited SQL dialect support
- All values returned as strings (limited type support)
- No connection lifecycle management
- Basic SQL execution only (execute, fetch_all)
- Not yet a drop-in replacement for `aiosqlite`

**Recent improvements (v0.0.2):**
- ✅ Security fixes: Upgraded dependencies (pyo3 0.27, pyo3-async-runtimes 0.27, sqlx 0.8)
- ✅ Connection pooling: Connection now reuses connection pool across operations (major performance improvement)
- ✅ Input validation: Added path validation (non-empty, no null bytes)
- ✅ Improved error handling: Enhanced error messages with database path and query context
- ✅ Type stubs: Added `.pyi` type stubs for better IDE support and type checking

**Goal**: Achieve drop-in replacement compatibility with `aiosqlite` to enable seamless migration with true async performance.

## Phase 1 — Credibility

Focus: Fix critical performance issues, add essential features for production use.

### Connection Management

- **Connection pooling** (partially complete - basic pooling implemented)
  - ✅ Implement proper connection pool with configurable size (basic implementation)
  - ✅ Connection reuse across operations
  - ⏳ Pool lifecycle management (planned)
  - ⏳ Connection health checking and recovery (planned)
  - ✅ Efficient pool initialization and shutdown (lazy initialization)

- **Connection lifecycle** (planned)
  - ⏳ Context manager support (`async with`) (planned)
  - ✅ Explicit connection management
  - ⏳ Connection state tracking (planned)
  - ⏳ Proper resource cleanup (planned)
  - ⏳ Connection timeout handling (planned)

- **Performance fixes** (complete)
  - ✅ Eliminate per-operation pool creation overhead
  - ✅ Efficient connection acquisition and release
  - ✅ Minimize connection churn

### Transaction Support

- **Basic transactions**
  - `begin()`, `commit()`, `rollback()` methods
  - Transaction context managers
  - Nested transaction handling (savepoints)
  - Transaction isolation level configuration

- **Error handling in transactions**
  - Automatic rollback on errors
  - Transaction state management
  - Deadlock detection and handling

### Type System Improvements

- **Better type handling**
  - Preserve SQLite types (INTEGER, REAL, TEXT, BLOB, NULL)
  - Type conversion utilities
  - Optional type hints for Python types
  - Binary data (BLOB) support

- **Return value improvements**
  - Return proper Python types where appropriate
  - Configurable type conversion
  - Type inference from schema
  - Date/time type handling

### Enhanced Error Handling

- **SQL-specific errors**
  - SQL syntax error detection and reporting
  - Constraint violation errors
  - Database locked errors with context
  - Better error messages with SQL context
  - Error code mapping

- **Connection errors**
  - Database file errors
  - Permission errors
  - Connection timeout errors
  - Recovery strategies

### API Improvements

- **Query methods**
  - `fetch_one()` - fetch single row
  - `fetch_optional()` - fetch one row or None
  - `execute_many()` - execute multiple statements
  - `last_insert_rowid()` - get last insert ID
  - `changes()` - get number of affected rows

- **API stability**
  - Consistent error handling patterns
  - Resource management guarantees
  - Thread-safety documentation
  - Performance characteristics documented

### API Compatibility for Drop-In Replacement

- **aiosqlite API compatibility**
  - Match `aiosqlite.Connection` and `aiosqlite.Cursor` APIs
  - Compatible connection factory pattern (`aiosqlite.connect()`)
  - Matching method signatures (`execute()`, `executemany()`, `fetchone()`, `fetchall()`, `fetchmany()`)
  - Compatible transaction methods (`commit()`, `rollback()`, `execute()` in transaction context)
  - Matching exception types (`Error`, `Warning`, `DatabaseError`, `OperationalError`, etc.)
  - Compatible context manager behavior for connections and cursors
  - Row factory compatibility (`row_factory` parameter)
  - Drop-in replacement validation: `import rapsqlite as aiosqlite` compatibility tests

- **Migration support**
  - Compatibility shim/adapter layer if needed for exact API matching
  - Migration guide documenting any differences
  - Backward compatibility considerations
  - Support for common aiosqlite patterns and idioms

### Testing & Validation

- Comprehensive test suite covering edge cases
- Fake Async Detector validation passes under load
- **Pass 100% of aiosqlite test suite** as drop-in replacement validation
- Drop-in replacement compatibility tests (can swap `aiosqlite` → `rapsqlite` without code changes)
- Benchmark comparison with existing async SQLite libraries
- Documentation improvements including migration guide

## Phase 2 — Expansion

Focus: Feature additions, performance optimizations, and broader SQLite feature support.

### Prepared Statements & Parameterized Queries

- **Prepared statements**
  - Statement preparation and caching
  - Parameter binding (named and positional)
  - Efficient statement reuse
  - Statement pool management

- **Parameterized queries**
  - Named parameters (`:name`, `@name`, `$name`)
  - Positional parameters (`?`, `?1`, `?2`)
  - Type-safe parameter binding
  - Array parameter binding for IN clauses

- **Query building utilities**
  - Helper functions for common query patterns
  - Query result mapping utilities
  - Optional ORM-like convenience methods

### Advanced SQLite Features

- **SQLite-specific features**
  - Full-text search (FTS) support
  - JSON functions support
  - Window functions
  - Common Table Expressions (CTEs)
  - UPSERT operations (INSERT OR REPLACE, etc.)

- **Schema operations**
  - Schema introspection (tables, columns, indexes)
  - Migration utilities
  - Schema validation
  - Foreign key constraint support

- **Performance features**
  - Index recommendations
  - Query plan analysis
  - WAL mode configuration
  - Journal mode configuration
  - Synchronous mode configuration

### Connection Configuration

- **Database configuration**
  - PRAGMA settings (cache_size, temp_store, etc.)
  - Connection string support
  - Database initialization hooks
  - Custom SQLite extensions (if applicable)

- **Pool configuration**
  - Configurable pool size
  - Connection timeout settings
  - Idle connection management
  - Pool monitoring and metrics

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
  - Maintain and refine aiosqlite drop-in replacement (achieved in Phase 1)
  - Enhanced compatibility features beyond core aiosqlite API
  - Migration guides from other libraries (sqlite3, etc.)
  - Compatibility shims for common patterns and idioms

- **Framework integration**
  - Integration examples with web frameworks
  - ORM integration patterns (SQLAlchemy, etc.)
  - Database migration tool integration

## Phase 3 — Ecosystem

Focus: Advanced features, ecosystem integration, and query optimization.

### Advanced Query Features

- **Query optimization**
  - Query plan analysis and optimization hints
  - Automatic index recommendations
  - Query result caching strategies
  - Lazy query execution patterns

- **Advanced result handling**
  - Streaming query results for large datasets
  - Cursor-based pagination
  - Result set transformation utilities
  - Row-to-object mapping helpers

### Async-Safe Connection Pooling

- **Advanced pooling**
  - Dynamic pool sizing
  - Connection health monitoring
  - Automatic pool scaling
  - Cross-process connection sharing patterns (if applicable)

- **Connection management**
  - Read/write connection separation
  - Replication patterns (read replicas)
  - Connection routing strategies
  - Failover and recovery patterns

### Ecosystem Adapters

- **ORM integration**
  - SQLAlchemy async driver support
  - Custom ORM adapters
  - Query builder integrations
  - Migration framework support

- **Framework integrations**
  - FastAPI database dependencies
  - Django async database backend (if applicable)
  - aiohttp database patterns
  - Background task queue integration

### Integration & Tooling

- **rap-core integration**
  - Shared primitives with other rap packages
  - Common database patterns
  - Unified error handling
  - Performance monitoring hooks

- **Developer tools**
  - Query logging and profiling
  - Database introspection tools
  - Migration generation utilities
  - Testing utilities and fixtures

### Observability & Monitoring

- **Monitoring & metrics**
  - Performance metrics export
  - Query timing and profiling
  - Connection pool metrics
  - Resource usage tracking
  - Slow query detection and reporting

- **Debugging tools**
  - SQL query logging
  - Transaction tracing
  - Connection pool diagnostics
  - Performance profiling utilities

### Advanced Features

- **Database features**
  - Backup and restore utilities
  - Database encryption support (if applicable)
  - Replication patterns
  - Multi-database transaction support

- **Testing & Development**
  - In-memory database support
  - Testing utilities and fixtures
  - Database mocking for tests
  - Migration testing tools

### Documentation & Community

- **Comprehensive documentation**
  - Advanced usage patterns and examples
  - Performance tuning guides
  - Migration documentation from other libraries
  - Best practices and anti-patterns
  - Contributing guidelines

- **Ecosystem presence**
  - PyPI package optimization
  - CI/CD pipeline improvements
  - Community examples and tutorials
  - Blog posts and case studies
  - Conference talks and presentations

## Cross-Package Dependencies

- **Phase 1**: Independent development, minimal dependencies
- **Phase 2**: Potential integration with `rapfiles` for database file operations, `rapcsv` for import/export patterns
- **Phase 3**: Integration with `rap-core` for shared primitives, serve as database foundation for rap ecosystem

## Success Criteria

- **Phase 1**: Connection pooling implemented, transactions supported, stable API, **drop-in replacement for aiosqlite**, passes 100% of aiosqlite test suite, passes Fake Async Detector under all load conditions
- **Phase 2**: Feature-complete for common SQLite use cases, competitive performance benchmarks, excellent documentation, seamless migration from aiosqlite
- **Phase 3**: Industry-leading performance, ecosystem integration, adoption as primary async SQLite library for Python and preferred aiosqlite alternative

## Versioning Strategy

Following semantic versioning:
- `v0.x`: Breaking changes allowed, MVP and Phase 1 development
- `v1.0`: Stable API, Phase 1 complete, production-ready
- `v1.x+`: Phase 2 and 3 features, backwards-compatible additions

