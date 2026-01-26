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

- ✅ **True async** SQLite operations
- ✅ **Native Rust-backed** execution (Tokio + sqlx)
- ✅ **Zero Python thread pools**
- ✅ **Event-loop-safe** concurrency under load
- ✅ **GIL-independent** database operations
- ✅ **Async-safe** SQLite bindings
- ✅ **Verified** by Fake Async Detector
- ✅ **Connection lifecycle management** (async context managers)
- ✅ **Transaction support** (begin, commit, rollback)
- ✅ **Type system improvements** (proper Python types: int, float, str, bytes, None)
- ✅ **Cursor API** (execute, executemany, fetchone, fetchall, fetchmany)
- ✅ **Enhanced error handling** (custom exception classes matching aiosqlite)
- ✅ **aiosqlite-compatible API** (connect function, exception types)

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
        
        # Fetch all rows
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
        print(schema)
        # Output: {
        #   'table_name': 'posts',
        #   'columns': [...],
        #   'indexes': [...],
        #   'foreign_keys': [...]
        # }

asyncio.run(main())
```

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

### `Connection(path: str)`

Create a new async SQLite connection.

**Parameters:**
- `path` (str): Path to the SQLite database file

**Example:**
```python
conn = Connection("example.db")
async with conn:
    await conn.execute("CREATE TABLE test (id INTEGER)")
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
# Output: ['users', 'posts', 'comments']

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
# Output: {'tables': [{'name': 'users'}, {'name': 'posts'}]}
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

For more technical details, see [docs/BACKUP_SQLITE3_CONNECTION_FIX.md](docs/BACKUP_SQLITE3_CONNECTION_FIX.md).

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

## Benchmarks

This package passes the [Fake Async Detector](https://github.com/eddiethedean/rap-bench). Benchmarks are available in the [rap-bench](https://github.com/eddiethedean/rap-bench) repository.

Run the detector yourself:

```bash
pip install rap-bench
rap-bench detect rapsqlite
```

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md) for detailed development plans. Key goals include:

- ✅ Phase 1: Connection lifecycle, transactions, type system, error handling, cursor API (complete)
- ⏳ Phase 2: Prepared statements and parameterized queries
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
- ⏳ Parameterized queries not yet supported (placeholder for Phase 2)
- ⏳ `Cursor.fetchmany()` returns all rows (size-based slicing in Phase 2)
- ⏳ Limited SQL dialect support (basic SQLite features)
- ⏳ Not yet a complete drop-in replacement for `aiosqlite` (work in progress)
- ⏳ Not designed for synchronous use cases
- ⚠️ **Backup to `sqlite3.Connection` not supported**: The `Connection.backup()` method supports backing up to another `rapsqlite.Connection`, but backing up to Python's standard `sqlite3.Connection` is not supported due to SQLite library instance incompatibility. See [Backup Limitations](#backup-limitations) below for details.

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

