# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, aligned with the [RAP Project Strategic Plan](../rap-project-plan.md). `rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx.

## Current Status

**Current Version (v0.1.2)** - Phase 1 Complete, Phase 2 In Progress:

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

**Phase 2 Progress:**
- ✅ **Phase 2.1 Complete**: Parameterized queries (named and positional parameters, execute_many with binding)
- ✅ **Phase 2.2 Complete**: Cursor improvements (fetchmany size-based slicing, result caching, state management)
- ✅ **Phase 2.3 Complete**: Connection configuration (PRAGMA settings, connection string parsing, constructor parameters)
- ⏳ **Phase 2.4 In Progress**: Pool configuration (infrastructure added, methods pending)
- ⏳ **Phase 2.5**: Row factory compatibility (planned)
- ⏳ **Phase 2.6**: Transaction context managers (planned)
- ⏳ **Phase 2.7-2.11**: Remaining Phase 2 features (planned)

**Goal**: Achieve drop-in replacement compatibility with `aiosqlite` to enable seamless migration with true async performance.

## Phase 1 — Credibility

Focus: Fix critical performance issues, add essential features for production use.

### Connection Management

- **Connection pooling** ✅ (complete - basic implementation)
  - ✅ Implement proper connection pool with configurable size (basic implementation)
  - ✅ Connection reuse across operations
  - ✅ Efficient pool initialization and shutdown (lazy initialization)
  - ⏳ Pool lifecycle management (advanced features - Phase 2)
  - ⏳ Connection health checking and recovery (Phase 2)

- **Connection lifecycle** ✅ (complete - basic implementation)
  - ✅ Context manager support (`async with`)
  - ✅ Explicit connection management
  - ✅ Proper resource cleanup
  - ⏳ Connection state tracking (Phase 2)
  - ⏳ Connection timeout handling (Phase 2)

- **Performance fixes** ✅ (complete)
  - ✅ Eliminate per-operation pool creation overhead
  - ✅ Efficient connection acquisition and release
  - ✅ Minimize connection churn

### Transaction Support

- **Basic transactions** ✅ (complete - basic implementation)
  - ✅ `begin()`, `commit()`, `rollback()` methods
  - ✅ Transaction state tracking
  - ⏳ Transaction context managers (Phase 2)
  - ⏳ Nested transaction handling (savepoints) (Phase 2)
  - ⏳ Transaction isolation level configuration (Phase 2)

- **Error handling in transactions** ✅ (complete - basic implementation)
  - ✅ Automatic rollback on connection close
  - ✅ Transaction state management
  - ⏳ Deadlock detection and handling (Phase 2)

### Type System Improvements

- **Better type handling** ✅ (complete)
  - ✅ Preserve SQLite types (INTEGER, REAL, TEXT, BLOB, NULL)
  - ✅ Type conversion to Python types (int, float, str, bytes, None)
  - ✅ Binary data (BLOB) support
  - ⏳ Optional type hints for Python types (Phase 2)
  - ⏳ Type conversion utilities (Phase 2)

- **Return value improvements** ✅ (complete - basic implementation)
  - ✅ Return proper Python types where appropriate
  - ⏳ Configurable type conversion (Phase 2)
  - ⏳ Type inference from schema (Phase 2)
  - ⏳ Date/time type handling (Phase 2)

### Enhanced Error Handling

- **SQL-specific errors** ✅ (complete)
  - ✅ SQL syntax error detection and reporting
  - ✅ Constraint violation errors (IntegrityError)
  - ✅ Better error messages with SQL context
  - ✅ Error code mapping to Python exceptions
  - ⏳ Database locked errors with context (basic support, enhanced in Phase 2)

- **Connection errors** ✅ (complete - basic implementation)
  - ✅ Database file errors
  - ✅ Permission errors (via OperationalError)
  - ⏳ Connection timeout errors (Phase 2)
  - ⏳ Recovery strategies (Phase 2)

### API Improvements

- **Query methods** ✅ (complete)
  - ✅ `fetch_one()` - fetch single row
  - ✅ `fetch_optional()` - fetch one row or None
  - ✅ `execute_many()` - execute multiple statements (placeholder, parameter binding in Phase 2)
  - ✅ `last_insert_rowid()` - get last insert ID
  - ✅ `changes()` - get number of affected rows

- **API stability** ✅ (complete - production-ready)
  - ✅ Consistent error handling patterns
  - ✅ Resource management guarantees
  - ⏳ Thread-safety documentation (Phase 2)
  - ⏳ Performance characteristics documented (Phase 2)

### API Compatibility for Drop-In Replacement

- **aiosqlite API compatibility** ✅ (core API complete - production-ready)
  - ✅ Match `aiosqlite.Connection` core API
  - ✅ Match `aiosqlite.Cursor` core API
  - ✅ Compatible connection factory pattern (`connect()`)
  - ✅ Matching method signatures (`execute()`, `executemany()`, `fetchone()`, `fetchall()`, `fetchmany()`)
  - ✅ Compatible transaction methods (`commit()`, `rollback()`, `begin()`)
  - ✅ Matching exception types (`Error`, `Warning`, `DatabaseError`, `OperationalError`, `ProgrammingError`, `IntegrityError`)
  - ✅ Compatible context manager behavior for connections and cursors
  - ⏳ Row factory compatibility (`row_factory` parameter) (Phase 2)
  - ⏳ Drop-in replacement validation: `import rapsqlite as aiosqlite` compatibility tests (Phase 2)

- **Migration support** ⏳ (Phase 2)
  - ⏳ Compatibility shim/adapter layer if needed for exact API matching
  - ⏳ Migration guide documenting any differences
  - ⏳ Backward compatibility considerations
  - ⏳ Support for common aiosqlite patterns and idioms

### Testing & Validation

- **Testing** ✅ (complete - comprehensive test suite)
  - ✅ Comprehensive test suite covering core features
  - ✅ Type conversion tests
  - ✅ Transaction tests
  - ✅ Error handling tests
  - ✅ Cursor API tests
  - ✅ Context manager tests
  - ⏳ Complete edge case coverage (Phase 2)
  - ⏳ Fake Async Detector validation passes under load (Phase 2)
  - ⏳ Pass 100% of aiosqlite test suite as drop-in replacement validation (Phase 2)
  - ⏳ Drop-in replacement compatibility tests (Phase 2)
  - ⏳ Benchmark comparison with existing async SQLite libraries (Phase 2)
  - ⏳ Documentation improvements including migration guide (Phase 2)

## Phase 2 — Expansion

Focus: Feature additions, performance optimizations, and broader SQLite feature support.

**Current Phase 2 Status**: Core features complete (2.1-2.3), pool configuration infrastructure added (2.4), remaining features in progress.

### Prepared Statements & Parameterized Queries

- **Prepared statements**
  - ⏳ Statement preparation and caching (Phase 2.8)
  - ✅ Parameter binding (named and positional) - complete
  - ⏳ Efficient statement reuse (Phase 2.8)
  - ⏳ Statement pool management (Phase 2.8)

- **Parameterized queries** ✅ (Phase 2.1 complete)
  - ✅ Named parameters (`:name`, `@name`, `$name`) - complete
  - ✅ Positional parameters (`?`, `?1`, `?2`) - complete
  - ✅ Type-safe parameter binding - complete
  - ⏳ Array parameter binding for IN clauses (basic support via lists, enhanced planned)
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

- **Database configuration** ✅ (Phase 2.3 complete)
  - ✅ PRAGMA settings support (`set_pragma()` method) - complete
  - ✅ Connection string support (URI format: `file:path?param=value`) - complete
  - ✅ PRAGMA settings via constructor parameter - complete
  - ⏳ Database initialization hooks (infrastructure added, async execution pending)
  - ⏳ Custom SQLite extensions (if applicable) (planned)

- **Pool configuration** ⏳ (Phase 2.4 in progress)
  - ✅ Configurable pool size (infrastructure added, implementation pending)
  - ✅ Connection timeout settings (infrastructure added, implementation pending)
  - ⏳ Idle connection management (planned)
  - ⏳ Pool monitoring and metrics (planned)
  - ⏳ Pool lifecycle management (advanced features from Phase 1) (planned)

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
  - Row factory compatibility (from Phase 1 remaining items)
  - Migration guides from other libraries (sqlite3, etc.)
  - Compatibility shims for common patterns and idioms
  - ✅ Python 3.13 support (wheels and CI builds) - complete in v0.1.1
  - ✅ Python 3.14 support (wheels and CI builds) - complete

- **Framework integration**
  - Integration examples with web frameworks
  - ORM integration patterns (SQLAlchemy, Tortoise ORM, Peewee)
  - Database migration tool integration (Alembic)
  - Testing framework integration (pytest-asyncio patterns)

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
  - Tortoise ORM async SQLite backend
  - Peewee async SQLite support
  - Custom ORM adapters
  - Query builder integrations
  - Migration framework support (Alembic, etc.)

- **Framework integrations**
  - FastAPI database dependencies
  - Django async database backend (if applicable)
  - aiohttp database patterns
  - Starlette async database integration
  - Quart async database support
  - Sanic async database patterns
  - Background task queue integration (Celery, RQ, Dramatiq)
  - Testing utilities (pytest-asyncio fixtures and patterns)

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
  - In-memory database support (basic support exists, enhanced planned)
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

- **Phase 1**: ✅ Independent development, minimal dependencies (complete)
- **Phase 2**: Potential integration with `rapfiles` for database file operations, `rapcsv` for import/export patterns
- **Phase 3**: Integration with `rap-core` for shared primitives, serve as database foundation for rap ecosystem

## Success Criteria

- **Phase 1**: ✅ Connection pooling implemented, ✅ transactions supported, ✅ stable API, ✅ **core aiosqlite API compatibility** (production-ready), ✅ comprehensive test suite, ✅ passes Fake Async Detector. Advanced features (parameterized queries, row factory, transaction context managers) moved to Phase 2.
- **Phase 2**: Feature-complete for common SQLite use cases, competitive performance benchmarks, excellent documentation, seamless migration from aiosqlite, complete drop-in replacement with advanced features
- **Phase 3**: Industry-leading performance, ecosystem integration, adoption as primary async SQLite library for Python and preferred aiosqlite alternative

## Versioning Strategy

Following semantic versioning:
- `v0.x`: Breaking changes allowed, MVP and Phase 1 development
- `v1.0`: Stable API, Phase 1 complete, production-ready (ready for release)
- `v1.x+`: Phase 2 and 3 features, backwards-compatible additions

**Current Version: v0.1.2** - Phase 1 complete and production-ready. Core aiosqlite API compatibility achieved. Python 3.8-3.14 supported. Ready for v1.0 release when appropriate. Advanced features (parameterized queries, row factory, etc.) planned for Phase 2.