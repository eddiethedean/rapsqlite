# True Async DBAPI Specification (for SQLAlchemy Compatibility)

## Purpose

This document defines a **True Async DBAPI** interface suitable for
integration with SQLAlchemy's asyncio extension. "True async" means **no
thread offloading** and **no greenlets** --- all I/O must be awaitable.

This is an advanced, low-level contract.

------------------------------------------------------------------------

## Required Python Compatibility

-   Python ≥ 3.10
-   Async/await native
-   No blocking calls in the event loop

------------------------------------------------------------------------

## DBAPI Module-Level Contract

### Required Attributes

``` python
apilevel = "2.0"
threadsafety = 0
paramstyle = "qmark" | "named" | "format" | "pyformat"
```

-   `threadsafety = 0` is strongly recommended
-   SQLite-style drivers usually use `qmark`

------------------------------------------------------------------------

### Required Exceptions

All must inherit from `Exception`:

``` python
class Error(Exception): ...
class InterfaceError(Error): ...
class DatabaseError(Error): ...
class DataError(DatabaseError): ...
class OperationalError(DatabaseError): ...
class IntegrityError(DatabaseError): ...
class InternalError(DatabaseError): ...
class ProgrammingError(DatabaseError): ...
class NotSupportedError(DatabaseError): ...
```

SQLAlchemy relies heavily on correct exception mapping.

------------------------------------------------------------------------

## Connection Interface

### Creation

``` python
async def connect(*args, **kwargs) -> AsyncConnection
```

### Required Methods

``` python
class AsyncConnection:
    async def cursor(self) -> AsyncCursor
    async def execute(self, sql: str, params=None) -> AsyncCursor
    async def executemany(self, sql: str, seq_of_params)
    async def commit(self)
    async def rollback(self)
    async def close(self)
```

### Optional but Strongly Recommended

``` python
async def __aenter__(self)
async def __aexit__(self, exc_type, exc, tb)
```

------------------------------------------------------------------------

### Transaction Semantics

-   Autocommit MUST be disabled by default
-   BEGIN should be explicit or implicit on first statement
-   Nested transactions require SAVEPOINT support

------------------------------------------------------------------------

## Cursor Interface

### Required Methods

``` python
class AsyncCursor:
    async def execute(self, sql: str, params=None)
    async def executemany(self, sql: str, seq_of_params)
    async def fetchone(self)
    async def fetchmany(self, size: int = None)
    async def fetchall(self)
    async def close(self)
```

### Required Attributes

``` python
cursor.description
cursor.rowcount
cursor.lastrowid
cursor.arraysize
```

------------------------------------------------------------------------

## Async Iteration Support (Highly Recommended)

``` python
async for row in cursor:
    ...
```

Must be equivalent to repeated `fetchone()`.

------------------------------------------------------------------------

## Type Handling

### Parameter Binding

-   Must support positional and/or named parameters
-   Type adaptation must be deterministic
-   NULL must map to Python None

### Result Decoding

-   Native Python types preferred
-   No lazy decoding allowed

------------------------------------------------------------------------

## Cancellation Semantics

-   Cancellation must:
    -   Abort the underlying query
    -   Leave connection in a valid state
-   Silent cancellation is NOT acceptable

------------------------------------------------------------------------

## Concurrency Rules

-   One operation per connection at a time
-   Concurrent cursor usage must raise `ProgrammingError`
-   Connection pooling is external (SQLAlchemy handles it)

------------------------------------------------------------------------

## SQLAlchemy Async Integration Hooks

### Required Dialect Flags

``` python
is_async = True
supports_server_side_cursors = False
supports_statement_cache = True
```

### Execution Model

SQLAlchemy expects: - Awaitable `execute()` - Deterministic
transactional boundaries - Immediate exception propagation

------------------------------------------------------------------------

## Event Loop Safety

-   No blocking calls
-   No background threads
-   All I/O must be awaitable

Violations WILL deadlock SQLAlchemy.

------------------------------------------------------------------------

## Minimal Driver Checklist

-   [x] Async connect()
-   [x] Async cursor
-   [x] Full exception hierarchy
-   [x] Transaction support
-   [x] Cancellation handling (interrupt on `CancelledError`, re-raise; connection remains usable)
-   [x] SQLAlchemy dialect registration (`sqlite+rapsqlite`; `create_async_engine`; engine creation)
-   [x] Deterministic cleanup

Implemented by rapsqlite: see `rapsqlite.dbapi`, `rapsqlite.sqlalchemy`, and `tests/test_dbapi.py`.

------------------------------------------------------------------------

## Final Warning

SQLite is **not a true async database**. Achieving this requires: -
Custom VFS - Native async I/O - Or C extensions

Most "async SQLite" drivers are async facades.

Proceed only if you fully accept this constraint.

------------------------------------------------------------------------

## References

-   PEP 249 (DBAPI 2.0)
-   SQLAlchemy asyncio internals
-   asyncpg driver architecture
