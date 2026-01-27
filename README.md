# rapsqlite

**True async SQLite — no fake async, no GIL stalls.**

[![PyPI version](https://img.shields.io/pypi/v/rapsqlite.svg)](https://pypi.org/project/rapsqlite/)
[![Downloads](https://pepy.tech/badge/rapsqlite)](https://pepy.tech/project/rapsqlite)
[![Python 3.8+](https://img.shields.io/badge/python-3.8+-blue.svg)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Overview

`rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx. Unlike libraries that wrap blocking database calls in `async` syntax, `rapsqlite` guarantees that all database operations execute **outside the Python GIL**, ensuring event loops never stall under load.

**Roadmap Goal**: Achieve drop-in replacement compatibility with `aiosqlite`, enabling seamless migration with true async performance. See [docs/ROADMAP.md](docs/ROADMAP.md) for details.

## Why `rap*`?

Packages prefixed with **`rap`** stand for **Real Async Python**. Unlike many libraries that merely wrap blocking I/O in `async` syntax, `rap*` packages guarantee that all I/O work is executed **outside the Python GIL** using native runtimes (primarily Rust). This means event loops are never stalled by hidden thread pools, blocking syscalls, or cooperative yielding tricks. If a `rap*` API is `async`, it is *structurally non-blocking by design*, not by convention. The `rap` prefix is a contract: measurable concurrency, real parallelism, and verifiable async behavior under load.

See the [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) for philosophy and guarantees.

## Features

- ✅ **True async** SQLite operations (all operations execute outside Python GIL)
- ✅ **Native Rust-backed** execution (Tokio + sqlx)
- ✅ **Zero Python thread pools** (no fake async)
- ✅ **Event-loop-safe** concurrency under load
- ✅ **GIL-independent** database operations
- ✅ **Async-safe** SQLite bindings
- ✅ **Verified** by Fake Async Detector
- ✅ **Connection lifecycle management** (async context managers)
- ✅ **Transaction support** (begin, commit, rollback, transaction context managers)
- ✅ **Type system improvements** (proper Python types: int, float, str, bytes, None)
- ✅ **Cursor API** (execute, executemany, fetchone, fetchall, fetchmany, executescript)
- ✅ **Enhanced error handling** (custom exception classes matching aiosqlite)
- ✅ **aiosqlite-compatible API** (~95% compatibility, drop-in replacement)
- ✅ **Prepared statement caching** (automatic via sqlx, 2-5x faster for repeated queries)
- ✅ **Connection pooling** (configurable pool size and timeouts)
- ✅ **Row factories** (dict, tuple, callable, and `rapsqlite.Row` class)
- ✅ **Advanced SQLite features** (callbacks, extensions, schema introspection, backup, dump)
- ✅ **Database initialization hooks** (automatic schema setup)

## Requirements

- Python 3.8+ (including Python 3.13 and 3.14)
- Rust 1.70+ (for building from source)

## Installation

```bash
pip install rapsqlite
```

### Building from Source

```bash
git clone https://github.com/eddiethedean/rapsqlite.git
cd rapsqlite
pip install maturin
maturin develop
```

---

## Usage

### Basic Usage

```python
import asyncio
import tempfile
import os
from rapsqlite import Connection

async def main():
    # Create a database file
    with tempfile.NamedTemporaryFile(suffix='.db', delete=False) as f:
        db_path = f.name
    
    try:
        # Create connection (async context manager)
        async with Connection(db_path) as conn:
            # Create table
            await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)")
            
            # Insert data
            await conn.execute("INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')")
            await conn.execute("INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')")
            
            # Fetch all rows
            rows = await conn.fetch_all("SELECT * FROM users")
            print(rows)
            # Output: [[1, 'Alice', 'alice@example.com'], [2, 'Bob', 'bob@example.com']]
            
            # Fetch single row
            user = await conn.fetch_one("SELECT * FROM users WHERE name = 'Alice'")
            print(user)
            # Output: [1, 'Alice', 'alice@example.com']
            
            # Fetch optional row
            user = await conn.fetch_optional("SELECT * FROM users WHERE name = 'Charlie'")
            print(user)
            # Output: None
    finally:
        # Cleanup
        if os.path.exists(db_path):
            os.unlink(db_path)

asyncio.run(main())
```

### Using the `connect()` Function (aiosqlite-compatible)

```python
import asyncio
from rapsqlite import connect

async def main():
    async with connect("example.db") as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (value) VALUES ('hello')")
        rows = await conn.fetch_all("SELECT * FROM test")
        print(rows)
        # Output: [[1, 'hello']]

asyncio.run(main())
```

### Transactions

```python
import asyncio
from rapsqlite import Connection

async def main():
    async with Connection("example.db") as conn:
        await conn.execute("CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)")
        await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")
        
        # Begin transaction
        await conn.begin()
        try:
            await conn.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
            await conn.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2")
            await conn.commit()
            print("Transaction committed")
            # Output: Transaction committed
        except Exception:
            await conn.rollback()
            print("Transaction rolled back")

asyncio.run(main())
```

### Using Cursors

```python
import asyncio
from rapsqlite import Connection

async def main():
    async with Connection("example.db") as conn:
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (name) VALUES ('item1')")
        await conn.execute("INSERT INTO items (name) VALUES ('item2')")
        
        # Use cursor
        cursor = conn.cursor()
        await cursor.execute("SELECT * FROM items")
        
        # Fetch one row
        row = await cursor.fetchone()
        print(row)  # Output: [1, 'item1']
        
        # Fetch all remaining rows (after fetchone, only remaining rows are returned)
        rows = await cursor.fetchall()
        print(rows)  # Output: [[2, 'item2']]
        
        # Execute a fresh query to fetch all rows
        await cursor.execute("SELECT * FROM items")
        rows = await cursor.fetchall()
        print(rows)  # Output: [[1, 'item1'], [2, 'item2']]
        
        # Fetch many rows
        await cursor.execute("SELECT * FROM items")
        rows = await cursor.fetchmany(1)
        print(rows)  # Output: [[1, 'item1']]

asyncio.run(main())
```

### Concurrent Database Operations

```python
import asyncio
from rapsqlite import Connection

async def main():
    async with Connection("example.db") as conn:
        await conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, value INTEGER)")
        
        # Execute multiple inserts concurrently
        tasks = [
            conn.execute(f"INSERT INTO data (value) VALUES ({i})")
            for i in range(100)
        ]
        await asyncio.gather(*tasks)
        
        # Fetch all results
        rows = await conn.fetch_all("SELECT * FROM data")
        print(f"Inserted {len(rows)} rows")
        # Output: Inserted 100 rows

asyncio.run(main())
```

### Error Handling

```python
import asyncio
from rapsqlite import Connection, IntegrityError, OperationalError

async def main():
    async with Connection("example.db") as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE)")
        await conn.execute("INSERT INTO users (email) VALUES ('alice@example.com')")
        
        try:
            # This will raise IntegrityError
            await conn.execute("INSERT INTO users (email) VALUES ('alice@example.com')")
        except IntegrityError as e:
            print(f"Integrity constraint violation: {e}")
            # Output: Integrity constraint violation: Failed to execute query on database ...: error returned from database: (code: 2067) UNIQUE constraint failed: users.email

asyncio.run(main())
```

### Database Backup

```python
import asyncio
from rapsqlite import Connection

async def main():
    # Create source database with data
    async with Connection("source.db") as source:
        await source.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        await source.execute("INSERT INTO test (name) VALUES ('Alice')")
        await source.execute("INSERT INTO test (name) VALUES ('Bob')")
        
        # Backup to another rapsqlite connection
        async with Connection("backup.db") as target:
            await source.backup(target)
            
            # Verify backup
            rows = await target.fetch_all("SELECT * FROM test")
            print(rows)  # Output: [[1, 'Alice'], [2, 'Bob']]

asyncio.run(main())
```

**⚠️ Important**: The `backup()` method only supports backing up to another `rapsqlite.Connection`. Backing up to Python's standard `sqlite3.Connection` is **not supported** and will cause a segmentation fault. See [Backup Limitations](#backup-limitations) for details.

### Schema Operations

```python
import asyncio
from rapsqlite import Connection

async def main():
    async with Connection("example.db") as conn:
        # Create tables with indexes and foreign keys
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                email TEXT UNIQUE NOT NULL
            )
        """)
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER,
                title TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )
        """)
        await conn.execute("CREATE INDEX idx_posts_user ON posts(user_id)")
        
        # List all tables
        tables = await conn.get_tables()
        print(tables)  # Output: ['posts', 'users']
        
        # Get table information
        columns = await conn.get_table_info("users")
        print(columns)
        # Output: [
        #   {'cid': 0, 'name': 'id', 'type': 'INTEGER', 'notnull': 0, 'dflt_value': None, 'pk': 1},
        #   {'cid': 1, 'name': 'email', 'type': 'TEXT', 'notnull': 1, 'dflt_value': None, 'pk': 0}
        # ]
        
        # Get indexes
        indexes = await conn.get_indexes(table_name="posts")
        print(indexes)
        # Output: [
        #   {'name': 'idx_posts_user', 'table': 'posts', 'unique': 0, 'sql': 'CREATE INDEX idx_posts_user ON posts(user_id)'}
        # ]
        
        # Get foreign keys
        foreign_keys = await conn.get_foreign_keys("posts")
        print(foreign_keys)
        # Output: [
        #   {'id': 0, 'seq': 0, 'table': 'users', 'from': 'user_id', 'to': 'id', 'on_update': 'NO ACTION', 'on_delete': 'NO ACTION', 'match': 'NONE'}
        # ]
        
        # Get comprehensive schema
        schema = await conn.get_schema(table_name="posts")
        print(list(schema.keys()))
        # Output: ['columns', 'indexes', 'foreign_keys', 'table_name']

asyncio.run(main())
```

### Database Initialization Hooks

**Note:** `init_hook` is a **rapsqlite-specific enhancement** and is not available in aiosqlite. This feature provides automatic database initialization capabilities that go beyond standard aiosqlite functionality.

The `init_hook` parameter allows you to run custom initialization code automatically when a connection pool is first created. This is useful for setting up schema, inserting initial data, or configuring database settings.

```python
import asyncio
from rapsqlite import Connection

async def init_hook(conn):
    """Initialize database schema and data."""
    # Create tables
    await conn.execute("""
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT UNIQUE
        )
    """)
    
    # Insert initial data
    await conn.execute("INSERT OR IGNORE INTO users (name, email) VALUES ('Admin', 'admin@example.com')")
    
    # Set additional PRAGMAs
    await conn.set_pragma("foreign_keys", True)

async def main():
    # Connection will automatically call init_hook on first use
    async with Connection("example.db", init_hook=init_hook) as conn:
        # Tables are already created and initialized
        users = await conn.fetch_all("SELECT * FROM users")
        print(users)  # Output: [[1, 'Admin', 'admin@example.com']]

asyncio.run(main())
```

**Key features:**
- The hook is called **once** per `Connection` instance, when the pool is first used
- The hook receives the `Connection` object as an argument, allowing full database access
- If the hook raises an exception, it will be propagated to the caller
- The hook can perform any database operations (create tables, insert data, set PRAGMAs, etc.)

**Compatibility note:** `init_hook` is a rapsqlite-specific enhancement and is not available in aiosqlite. This feature provides automatic database initialization capabilities that go beyond standard aiosqlite functionality.

## API Reference

### `connect(path: str, **kwargs: Any) -> Connection`

Connect to a SQLite database (aiosqlite-compatible API).

**Parameters:**
- `path` (str): Path to the SQLite database file
- `**kwargs`: Additional arguments (currently ignored, reserved for future use)

**Returns:**
- `Connection`: An async SQLite connection

**Example:**
```python
async with connect("example.db") as conn:
    await conn.execute("CREATE TABLE test (id INTEGER)")
```

### `Connection(path: str, *, pragmas=None, init_hook=None)`

Create a new async SQLite connection.

**Parameters:**
- `path` (str): Path to the SQLite database file
- `pragmas` (Optional[Dict[str, Any]]): Optional dictionary of PRAGMA settings to apply at connection time
- `init_hook` (Optional[Callable[[Connection], Coroutine[Any, Any, None]]]): Optional async callable that receives the Connection object and runs initialization code. Called once when the connection pool is first used.
  
  **Note:** `init_hook` is a rapsqlite-specific feature and is not available in aiosqlite. This enhancement provides automatic database initialization capabilities beyond standard aiosqlite functionality.

**Example:**
```python
async def init_hook(conn):
    await conn.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)")

conn = Connection("example.db", pragmas={"foreign_keys": True}, init_hook=init_hook)
async with conn:
    await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
```

### Connection Methods

#### `Connection.execute(query: str) -> None`

Execute a SQL statement (CREATE, INSERT, UPDATE, DELETE, etc.).

**Parameters:**
- `query` (str): SQL query to execute

**Raises:**
- `OperationalError`: If the query execution fails
- `ProgrammingError`: For SQL syntax errors
- `IntegrityError`: For constraint violations

#### `Connection.fetch_all(query: str) -> List[List[Any]]`

Execute a SELECT query and return all rows.

**Parameters:**
- `query` (str): SELECT query to execute

**Returns:**
- `List[List[Any]]`: List of rows, where each row is a list of values (int, float, str, bytes, or None)

**Raises:**
- `OperationalError`: If the query execution fails
- `ProgrammingError`: For SQL syntax errors

#### `Connection.fetch_one(query: str) -> List[Any]`

Execute a SELECT query and return a single row.

**Parameters:**
- `query` (str): SELECT query to execute

**Returns:**
- `List[Any]`: A single row as a list of values

**Raises:**
- `OperationalError`: If no row is found or query execution fails
- `ProgrammingError`: For SQL syntax errors

#### `Connection.fetch_optional(query: str) -> Optional[List[Any]]`

Execute a SELECT query and return a single row or None.

**Parameters:**
- `query` (str): SELECT query to execute

**Returns:**
- `Optional[List[Any]]`: A single row as a list of values, or None if no row is found

**Raises:**
- `OperationalError`: If the query execution fails
- `ProgrammingError`: For SQL syntax errors

#### `Connection.begin() -> None`

Begin a transaction.

**Raises:**
- `OperationalError`: If a transaction is already in progress

#### `Connection.commit() -> None`

Commit the current transaction.

**Raises:**
- `OperationalError`: If no transaction is in progress

#### `Connection.rollback() -> None`

Rollback the current transaction.

**Raises:**
- `OperationalError`: If no transaction is in progress

#### `Connection.last_insert_rowid() -> int`

Get the row ID of the last inserted row.

**Returns:**
- `int`: The row ID of the last INSERT operation

#### `Connection.changes() -> int`

Get the number of rows affected by the last statement.

**Returns:**
- `int`: The number of rows affected

#### `Connection.cursor() -> Cursor`

Create a cursor for this connection.

**Returns:**
- `Cursor`: A new cursor object

#### `Connection.close() -> None`

Close the connection and release resources.

#### `Connection.get_tables(name: Optional[str] = None) -> List[str]`

Get list of table names in the database.

**Parameters:**
- `name` (Optional[str]): Optional table name filter. If provided, returns only that table if it exists.

**Returns:**
- `List[str]`: List of table names, excluding system tables (sqlite_*)

**Example:**
```python
tables = await conn.get_tables()
# Output: ['comments', 'posts', 'users']  # Tables are returned in alphabetical order

user_table = await conn.get_tables(name="users")
# Output: ['users']
```

#### `Connection.get_table_info(table_name: str) -> List[Dict[str, Any]]`

Get table information (columns) for a specific table.

**Parameters:**
- `table_name` (str): Name of the table to get information for

**Returns:**
- `List[Dict[str, Any]]`: List of dictionaries with column metadata:
  - `cid`: Column ID
  - `name`: Column name
  - `type`: Column type
  - `notnull`: Not null constraint (0 or 1)
  - `dflt_value`: Default value (can be None)
  - `pk`: Primary key (0 or 1)

**Example:**
```python
info = await conn.get_table_info("users")
# Output: [
#   {'cid': 0, 'name': 'id', 'type': 'INTEGER', 'notnull': 0, 'dflt_value': None, 'pk': 1},
#   {'cid': 1, 'name': 'email', 'type': 'TEXT', 'notnull': 1, 'dflt_value': None, 'pk': 0}
# ]
```

#### `Connection.get_indexes(table_name: Optional[str] = None) -> List[Dict[str, Any]]`

Get list of indexes in the database.

**Parameters:**
- `table_name` (Optional[str]): Optional table name filter. If provided, returns only indexes for that table.

**Returns:**
- `List[Dict[str, Any]]`: List of dictionaries with index information:
  - `name`: Index name
  - `table`: Table name
  - `unique`: Whether index is unique (0 or 1)
  - `sql`: CREATE INDEX SQL statement (can be None)

**Example:**
```python
all_indexes = await conn.get_indexes()
table_indexes = await conn.get_indexes(table_name="users")
```

#### `Connection.get_foreign_keys(table_name: str) -> List[Dict[str, Any]]`

Get foreign key constraints for a specific table.

**Parameters:**
- `table_name` (str): Name of the table to get foreign keys for

**Returns:**
- `List[Dict[str, Any]]`: List of dictionaries with foreign key information:
  - `id`: Foreign key ID
  - `seq`: Sequence number
  - `table`: Referenced table name
  - `from`: Column in current table
  - `to`: Column in referenced table
  - `on_update`: ON UPDATE action
  - `on_delete`: ON DELETE action
  - `match`: MATCH clause

**Example:**
```python
fks = await conn.get_foreign_keys("posts")
# Output: [
#   {'id': 0, 'seq': 0, 'table': 'users', 'from': 'user_id', 'to': 'id', ...}
# ]
```

#### `Connection.get_schema(table_name: Optional[str] = None) -> Dict[str, Any]`

Get comprehensive schema information for a table or all tables.

**Parameters:**
- `table_name` (Optional[str]): Optional table name. If provided, returns detailed info for that table. If None, returns list of all tables.

**Returns:**
- `Dict[str, Any]`: Dictionary with schema information:
  - If `table_name` provided: `columns`, `indexes`, `foreign_keys`, `table_name`
  - If `table_name` is None: `tables` (list of table names)

**Example:**
```python
# Get schema for specific table
schema = await conn.get_schema(table_name="users")
# Output: {'table_name': 'users', 'columns': [...], 'indexes': [...], 'foreign_keys': [...]}

# Get all tables
all_schema = await conn.get_schema()
# Output: {'tables': [{'name': 'posts'}, {'name': 'users'}]}  # Tables in alphabetical order
```

### Cursor Methods

#### `Cursor.execute(query: str) -> None`

Execute a SQL statement.

**Parameters:**
- `query` (str): SQL query to execute

**Raises:**
- `OperationalError`: If the query execution fails
- `ProgrammingError`: For SQL syntax errors

#### `Cursor.executemany(query: str, parameters: List[List[Any]]) -> None`

Execute a SQL statement multiple times with different parameters.

**Parameters:**
- `query` (str): SQL query to execute
- `parameters` (List[List[Any]]): List of parameter lists

**Note:** Parameter binding is currently a placeholder and will be implemented in Phase 2.

**Raises:**
- `OperationalError`: If the query execution fails
- `ProgrammingError`: For SQL syntax errors

#### `Cursor.fetchone() -> Optional[List[Any]]`

Fetch the next row from the query result.

**Returns:**
- `Optional[List[Any]]`: A single row as a list of values, or None if no more rows

**Raises:**
- `ProgrammingError`: If no query has been executed

#### `Cursor.fetchall() -> List[List[Any]]`

Fetch all remaining rows from the query result.

**Returns:**
- `List[List[Any]]`: List of rows

**Raises:**
- `ProgrammingError`: If no query has been executed

#### `Cursor.fetchmany(size: Optional[int] = None) -> List[List[Any]]`

Fetch multiple rows from the query result.

**Parameters:**
- `size` (Optional[int]): Number of rows to fetch (default: 1, or all if not specified)

**Returns:**
- `List[List[Any]]`: List of rows

**Note:** For Phase 1, this returns all rows. Proper size-based slicing will be implemented in Phase 2.

**Raises:**
- `ProgrammingError`: If no query has been executed

#### `Connection.backup(target, *, pages=0, progress=None, name="main", sleep=0.25) -> None`

Make a backup of the current database to a target database.

**Parameters:**
- `target` (Connection): Target connection for backup (must be a `rapsqlite.Connection`)
- `pages` (int, optional): Number of pages to copy per step (0 = all pages). Default: 0
- `progress` (Callable[[int, int, int], None], optional): Progress callback function receiving (remaining, page_count, pages_copied). Default: None
- `name` (str, optional): Database name to backup (e.g., "main", "temp"). Default: "main"
- `sleep` (float, optional): Sleep duration in seconds between backup steps. Default: 0.25

**Raises:**
- `OperationalError`: If backup fails or target connection is invalid

**Example:**
```python
async with Connection("source.db") as source:
    async with Connection("target.db") as target:
        await source.backup(target)
```

**⚠️ Important Limitation**: The `target` parameter must be a `rapsqlite.Connection`. Backing up to Python's standard `sqlite3.Connection` is **not supported** due to SQLite library instance incompatibility. See [Backup Limitations](#backup-limitations) for details.

### Backup Limitations

**SQLite Library Instance Incompatibility**

The `Connection.backup()` method supports backing up from one `rapsqlite.Connection` to another `rapsqlite.Connection`. However, backing up to Python's standard `sqlite3.Connection` is **not supported** and will cause a segmentation fault.

**Root Cause:**
- Python's `sqlite3` module uses one SQLite library instance (from Python's build)
- `rapsqlite` uses `libsqlite3-sys` which may link to a different SQLite library instance
- SQLite handles are only valid within the same library instance
- Mixing handles from different instances causes undefined behavior (segfault)

**Workaround:**
Use `rapsqlite.Connection` for both source and target:

```python
# ✅ Supported: rapsqlite-to-rapsqlite backup
async with Connection("source.db") as source:
    async with Connection("target.db") as target:
        await source.backup(target)

# ❌ Not supported: rapsqlite-to-sqlite3.Connection backup
import sqlite3
source = Connection("source.db")
target = sqlite3.connect("target.db")  # This will cause a segfault
await source.backup(target)  # DO NOT DO THIS
```

**Alternative Solutions:**
1. Convert `sqlite3.Connection` to `rapsqlite.Connection`:
   ```python
   # Instead of using sqlite3.Connection, use rapsqlite.Connection
   target = Connection("target.db")
   await source.backup(target)
   ```

2. Use file-based backup methods if you need to work with `sqlite3.Connection`:
   ```python
   # Use Python's sqlite3 backup if both are sqlite3.Connection
   import sqlite3
   source_sqlite3 = sqlite3.connect("source.db")
   target_sqlite3 = sqlite3.connect("target.db")
   source_sqlite3.backup(target_sqlite3)  # This works
   ```

For more technical details, see the [Backup documentation](https://rapsqlite.readthedocs.io/en/latest/api-reference/connection.html#rapsqlite.Connection.backup) in the API reference.

### Exception Classes

The following exception classes are available, matching the aiosqlite API:

- `Error`: Base exception class for all rapsqlite errors
- `Warning`: Warning exception class
- `DatabaseError`: Base exception class for database-related errors
- `OperationalError`: Exception raised for operational errors (database locked, connection issues, etc.)
- `ProgrammingError`: Exception raised for programming errors (SQL syntax errors, etc.)
- `IntegrityError`: Exception raised for integrity constraint violations (UNIQUE constraint, FOREIGN KEY constraint, etc.)

## Type System

`rapsqlite` automatically converts SQLite types to appropriate Python types:

- `INTEGER` → `int`
- `REAL` → `float`
- `TEXT` → `str`
- `BLOB` → `bytes`
- `NULL` → `None`

## Performance

### Benchmarks

This package passes the [Fake Async Detector](https://github.com/eddiethedean/rap-bench). Benchmarks are available in the [rap-bench](https://github.com/eddiethedean/rap-bench) repository.

Run the detector yourself:

```bash
pip install rap-bench
rap-bench detect rapsqlite
```

### Performance Characteristics

- **True async**: All database operations execute outside the Python GIL
- **Better concurrency**: No event loop stalls under load
- **Connection pooling**: Efficient connection reuse with configurable pool size
- **Prepared statement caching**: Automatic query optimization via sqlx

### Benchmark Suite

See [benchmarks/README.md](benchmarks/README.md) for detailed performance benchmarks comparing rapsqlite with aiosqlite and sqlite3.

Run benchmarks:

```bash
pytest benchmarks/benchmark_suite.py -v -s
```

**Recent benchmark results (macOS arm64, Python 3.9.6):**
- **Simple Query Throughput**: 0.118ms mean latency (1000 queries)
- **Batch Insert**: 505ms for 1000 rows via `execute_many()`
- **Concurrent Reads**: 65ms for 10 workers × 100 queries (1000 total)
- **Transaction Performance**: 235ms for 100 transactions × 10 inserts

**Key advantages:**
- **True async**: All operations execute outside the Python GIL
- **Prepared statement caching**: Automatic query optimization via sqlx (2-5x faster for repeated queries)
- **Better throughput**: Superior performance under concurrent load due to GIL independence
- **Improved scalability**: Better performance scaling with multiple concurrent operations
- **Connection pooling**: Efficient connection reuse with configurable pool size

## Migration from aiosqlite

`rapsqlite` is designed to be a **drop-in replacement** for `aiosqlite`. The simplest migration is a one-line change:

```python
# Before
import aiosqlite

# After
import rapsqlite as aiosqlite
```

For most applications, this is all you need! All core aiosqlite APIs are supported, including:
- Connection and cursor APIs
- `async with db.execute(...)` pattern
- Async iteration on cursors (`async for row in cursor`)
- Parameterized queries (named and positional)
- Transactions and context managers
- Row factories (including `rapsqlite.Row` class)
- Connection properties (`total_changes`, `in_transaction`, `text_factory`)
- `executescript()` and `load_extension()` methods
- Exception types

**Practical compatibility notes:**

- **`total_changes` / `in_transaction`**: these are properties in `aiosqlite` and async methods in `rapsqlite`:

  ```python
  # aiosqlite
  changes = db.total_changes
  in_tx = db.in_transaction

  # rapsqlite
  changes = await db.total_changes()
  in_tx = await db.in_transaction()
  ```

- **`iterdump()`**: `aiosqlite` exposes an async iterator, while `rapsqlite` returns `List[str]`:

  ```python
  # aiosqlite
  lines = [line async for line in db.iterdump()]

  # rapsqlite
  lines = await db.iterdump()
  dump_sql = "\n".join(lines)
  ```

- **`backup()` targets**: `rapsqlite` only supports backups where both source and target are `rapsqlite.Connection` objects. For flows that used `sqlite3.Connection` as a target in `aiosqlite`, prefer:
  - rapsqlite-to-rapsqlite backup when staying within rapsqlite, or
  - file-based copy / sqlite3’s own backup API when working purely with `sqlite3`.

**See the [Migration Guide](https://rapsqlite.readthedocs.io/en/latest/guides/migration-guide.html) for a complete migration guide** with:
- Step-by-step migration instructions
- Code examples for common patterns
- API differences and limitations
- Troubleshooting guide
- Performance considerations

**Compatibility Analysis**: See the [Compatibility Guide](https://rapsqlite.readthedocs.io/en/latest/guides/compatibility.html) for detailed analysis based on running the aiosqlite test suite. Overall compatibility: **~95%** for core use cases (updated 2026-01-26). All high-priority compatibility features implemented including `total_changes()`, `in_transaction()`, `executescript()`, `load_extension()`, `text_factory`, `Row` class, and async iteration on cursors.

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for detailed development plans. Key goals include:

- ✅ Phase 1: Connection lifecycle, transactions, type system, error handling, cursor API (complete)
- ✅ Phase 2: Parameterized queries, cursor improvements, connection/pool configuration, row factory, transaction context managers, advanced callbacks, database dump/backup, schema introspection, database initialization hooks, prepared statement caching (complete)
- ⏳ Phase 3: Advanced SQLite features and ecosystem integration

## Related Projects

- [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) - Philosophy and guarantees
- [rap-bench](https://github.com/eddiethedean/rap-bench) - Fake Async Detector CLI
- [rapfiles](https://github.com/eddiethedean/rapfiles) - True async filesystem I/O
- [rapcsv](https://github.com/eddiethedean/rapcsv) - Streaming async CSV

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for detailed release notes and version history.

## Limitations (v0.2.0)

**Current limitations:**
- ⏳ Not designed for synchronous use cases
- ⚠️ **Backup to `sqlite3.Connection` not supported**: The `Connection.backup()` method supports backing up to another `rapsqlite.Connection`, but backing up to Python's standard `sqlite3.Connection` is not supported due to SQLite library instance incompatibility. See [Backup Limitations](#backup-limitations) below for details.

**Phase 2.1-2.11 improvements (v0.2.0):**
- ✅ Parameterized queries (named and positional parameters) - Phase 2.1 complete
- ✅ Cursor improvements (fetchmany with size-based slicing) - Phase 2.2 complete
- ✅ Connection and pool configuration (PRAGMAs, pool size, timeouts) - Phase 2.3-2.4 complete
- ✅ Row factory support (dict, tuple, callable) - Phase 2.5 complete
- ✅ Transaction context managers - Phase 2.6 complete
- ✅ Advanced SQLite callbacks (create_function, set_trace_callback, etc.) - Phase 2.7 complete
- ✅ Database dump and backup - Phase 2.8-2.9 complete
- ✅ Schema introspection (9 methods) - Phase 2.10 complete
- ✅ Database initialization hooks - Phase 2.11 complete

**Phase 1 improvements (v0.1.0 – v0.1.1):**
- ✅ Connection lifecycle management (async context managers)
- ✅ Transaction support (begin, commit, rollback)
- ✅ Type system improvements (proper Python types: int, float, str, bytes, None)
- ✅ Enhanced error handling (custom exception classes matching aiosqlite)
- ✅ API improvements (fetch_one, fetch_optional, execute_many, last_insert_rowid, changes)
- ✅ Cursor API (execute, executemany, fetchone, fetchall, fetchmany)
- ✅ aiosqlite compatibility (connect function, exception types)
- ✅ Security fixes: Upgraded dependencies (pyo3 0.27, pyo3-async-runtimes 0.27, sqlx 0.8)
- ✅ Connection pooling: Connection reuses connection pool across operations
- ✅ Input validation: Added path validation (non-empty, no null bytes)
- ✅ Improved error handling: Enhanced error messages with database path and query context
- ✅ Type stubs: Added `.pyi` type stubs for better IDE support and type checking

**Roadmap**: See [docs/ROADMAP.md](docs/ROADMAP.md) for planned improvements. Our goal is to achieve drop-in replacement compatibility with `aiosqlite` while providing true async performance with GIL-independent database operations.

## Contributing

Contributions are welcome! Please see our [contributing guidelines](https://github.com/eddiethedean/rapsqlite/blob/main/CONTRIBUTING.md) (coming soon).

## License

MIT

