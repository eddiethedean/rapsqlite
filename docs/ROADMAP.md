# rapsqlite Roadmap

This roadmap outlines the development plan for `rapsqlite`, a true async SQLite library for Python built with Rust, Tokio, and sqlx.

## Current Status

**Current Version: v0.2.0** ✅  
**Phase 1: Complete** ✅  
**Phase 2: Complete** ✅  
**Phase 3: In Planning** ⏳

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

**Test Coverage**: 432+ tests passing (6 skipped)  
**API Compatibility**: ~95% with aiosqlite  
**Python Support**: 3.8–3.14  
**Code Quality**: Full mypy type checking and Ruff formatting/linting

---

## Phase 3 — Advanced Features & Ecosystem (v0.3.0 → v1.0.0)

**Goal**: Transform `rapsqlite` into the industry-leading async SQLite library for Python with advanced features, ecosystem integration, and optimizations leading to a stable v1.0.0 release.

**Timeline**: Incremental releases (v0.3.0, v0.4.0, etc.) leading to v1.0.0

### 3.1 Query Optimization & Performance (High Priority)

**Focus**: Advanced query features and performance optimizations

#### Query Optimization
- ⏳ Query plan analysis and optimization hints
- ⏳ Automatic index recommendations
- ⏳ Query result caching strategies
- ⏳ Lazy query execution patterns
- ⏳ EXPLAIN QUERY PLAN integration

#### Result Handling
- ⏳ Streaming query results for large datasets
- ⏳ Cursor-based pagination utilities
- ⏳ Result set transformation utilities
- ⏳ Row-to-object mapping helpers
- ⏳ Efficient memory usage patterns for large result sets

#### SQLite-Specific Features
- ⏳ Full-text search (FTS) support
- ⏳ JSON functions support (JSON1 extension)
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
- ⏳ Connection health monitoring and automatic recovery
- ⏳ Idle connection management (timeout and cleanup)
- ⏳ Pool monitoring and metrics (connection count, wait times, etc.)
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
- ⏳ Nested transaction handling (savepoints)
- ⏳ Transaction isolation level configuration
- ⏳ Deadlock detection and automatic retry
- ⏳ Transaction timeout handling
- ⏳ Long-running transaction monitoring

#### Transaction Utilities
- ⏳ Savepoint context managers (`async with db.savepoint():`)
- ⏳ Transaction retry decorators/utilities
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
- ⏳ SQLAlchemy async driver support
- ⏳ Tortoise ORM async SQLite backend
- ⏳ Peewee async SQLite support
- ⏳ Custom ORM adapters and patterns
- ⏳ Query builder integrations

#### Web Framework Integration
- ⏳ FastAPI database dependencies and patterns
- ⏳ Django async database backend (if applicable)
- ⏳ aiohttp database patterns and middleware
- ⏳ Starlette async database integration
- ⏳ Quart async database support
- ⏳ Sanic async database patterns

#### Migration Tools
- ⏳ Alembic integration patterns
- ⏳ Migration generation utilities
- ⏳ Schema migration testing tools

**Success Criteria**:
- SQLAlchemy async driver works seamlessly
- FastAPI integration examples and patterns documented
- Alembic migrations work with rapsqlite
- At least 3 major frameworks have integration examples

---

### 3.5 Observability & Monitoring (Medium Priority)

**Focus**: Production monitoring and debugging capabilities

#### Monitoring & Metrics
- ⏳ Performance metrics export (Prometheus, StatsD, etc.)
- ⏳ Query timing and profiling
- ⏳ Connection pool metrics
- ⏳ Resource usage tracking
- ⏳ Slow query detection and reporting

#### Debugging Tools
- ⏳ SQL query logging (configurable levels)
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
- ⏳ Performance tuning guides
- ⏳ Migration documentation from other libraries
- ⏳ Best practices and anti-patterns
- ⏳ Contributing guidelines
- ⏳ Thread-safety documentation

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
- ⏳ Pass 100% of aiosqlite test suite as drop-in replacement validation
- ⏳ Stress testing and performance regression tests
- ⏳ Cross-platform testing (Linux, macOS, Windows)

#### Compatibility Testing
- ⏳ Continuous compatibility testing with aiosqlite
- ⏳ Python version compatibility matrix (3.8–3.14+)
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
- ⏳ `Connection.execute_fetchall(sql, parameters=None)` - Helper to execute query and fetch all rows
- ⏳ `Connection.execute_insert(sql, parameters=None)` - Helper to insert and get last_insert_rowid
- These convenience methods improve API compatibility and reduce boilerplate code

#### Connection Control Methods
- ⏳ `Connection.interrupt()` - Interrupt pending queries (async method)
- ⏳ `Connection.stop()` - Stop background thread (for API compatibility, though less relevant for rapsqlite's architecture)
- Query interruption is important for long-running operations and timeout handling

#### Connection Properties
- ⏳ `Connection.isolation_level` - Property to get/set transaction isolation level
- Currently missing but present in aiosqlite API

#### Connection Await Support
- ⏳ `Connection.__await__()` - Support for `await conn` pattern (aiosqlite supports this)
- Enables direct await on connection objects for compatibility

#### Cursor Properties (All Missing)
- ⏳ `Cursor.arraysize` - Default size for fetchmany() (int, default 1, read-write property)
- ⏳ `Cursor.connection` - Reference to parent Connection object (read-only property)
- ⏳ `Cursor.description` - Column metadata tuple (read-only property, reflects last executed query)
- ⏳ `Cursor.lastrowid` - Last inserted row ID (read-only property, reflects last executed query)
- ⏳ `Cursor.rowcount` - Number of rows affected by last operation (read-only property)
- ⏳ `Cursor.row_factory` - Row factory for this cursor (getter/setter property)
- These properties are essential for aiosqlite compatibility and provide important query metadata

#### Cursor Methods
- ⏳ `Cursor.close()` - Explicit cursor cleanup (async method)
- Provides explicit resource management for cursors

**Success Criteria**:
- All aiosqlite Connection helper methods implemented and tested
- All aiosqlite Cursor properties implemented and tested
- Connection interrupt functionality works correctly
- Cursor properties accurately reflect query state
- 100% aiosqlite API compatibility achieved (up from ~95%)
- All new methods/properties have comprehensive test coverage

---

### 3.10 Type System Enhancements (Medium Priority)

**Focus**: Enhanced type conversion and adapter support for custom types

#### create_function() Enhancement
- ⏳ `Connection.create_function(name, num_params, func, deterministic=False)` - Add `deterministic` parameter support
- SQLite 3.8.3+ optimization flag that allows SQLite to perform additional optimizations
- Should raise `NotSupportedError` if used with older SQLite versions

#### connect() Parameters
- ⏳ `connect(iter_chunk_size=64)` - Parameter for controlling iteration chunk size
- ⏳ `connect(loop=None)` - Event loop parameter (deprecated in aiosqlite but still exists for compatibility)
- These parameters improve compatibility with existing aiosqlite code

#### Module-Level Type Registration Functions
- ⏳ `rapsqlite.register_adapter(type, adapter)` - Register Python-to-SQLite type adapter
- ⏳ `rapsqlite.register_converter(typename, converter)` - Register SQLite-to-Python type converter
- These are sqlite3 compatibility features for custom type handling
- Requires careful implementation to work with sqlx's type system
- Enables custom date/time, UUID, and other type conversions

#### Enhanced Type Conversion Utilities
- ⏳ Date/time type handling utilities
- ⏳ UUID type support
- ⏳ Decimal type support
- ⏳ Custom type adapter examples and documentation

**Success Criteria**:
- `deterministic` parameter works correctly with create_function()
- `iter_chunk_size` and `loop` parameters accepted (even if not fully utilized)
- `register_adapter` and `register_converter` functions work with sqlx
- Custom type conversions work seamlessly
- Type conversion utilities documented with examples
- All type system features have test coverage

---

## Versioning Strategy

Following semantic versioning:

- **v0.1.x**: Phase 1 (MVP and core features) ✅ Complete
- **v0.2.x**: Phase 2 (feature-complete drop-in replacement) ✅ Complete (v0.2.0 released)
- **v0.3.x+**: Phase 3 (advanced features, ecosystem integration) ⏳ In Progress
- **v1.0.0**: Stable API release after Phase 3 completion, production-ready ⏳ Planned

**Current Version: v0.2.0** — Phase 1 and Phase 2 complete. Phase 3 will lead to v1.0.0 release.

---

## Success Criteria for v1.0.0

### Must Have (Blocking v1.0.0)
- ✅ Phase 1 and Phase 2 complete (achieved in v0.2.0)
- ⏳ Phase 3.1 (Query Optimization) — High priority features complete
- ⏳ Phase 3.2 (Advanced Pooling) — High priority features complete
- ⏳ Phase 3.4 (ORM Integration) — At least SQLAlchemy and FastAPI integration complete
- ⏳ Phase 3.8 (Testing) — 100% aiosqlite test suite passes
- ⏳ Phase 3.9 (API Completeness) — All high-priority API gaps filled, 100% aiosqlite compatibility

### Should Have (Target for v1.0.0)
- ⏳ Phase 3.3 (Advanced Transactions) — Core features complete
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

*Last Updated: 2026-01-28*
