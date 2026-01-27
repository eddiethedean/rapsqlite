# Migration Guide: aiosqlite to rapsqlite

This guide helps you migrate from `aiosqlite` to `rapsqlite` for true async SQLite operations with GIL-independent performance.

## Quick Start

The simplest migration is a one-line change:

```python
# Before (aiosqlite)
import aiosqlite

# After (rapsqlite)
import rapsqlite as aiosqlite
```

For most applications, this is all you need! `rapsqlite` is designed to be a drop-in replacement for `aiosqlite`.

## Why Migrate?

- **True async**: All database operations execute outside the Python GIL
- **Better performance**: No fake async, no event loop stalls
- **Same API**: Drop-in replacement with minimal code changes
- **Verified**: Passes Fake Async Detector benchmarks

## Migration Steps

### 1. Install rapsqlite

```bash
pip install rapsqlite
```

### 2. Update Imports

**Option A: Simple alias (recommended)**
```python
import rapsqlite as aiosqlite
```

**Option B: Direct import**
```python
from rapsqlite import connect, Connection
```

### 3. Verify Compatibility

Run your existing tests. Most code should work without changes.

## API Compatibility

### Core API (100% Compatible)

All core aiosqlite APIs are supported:

- ✅ `connect()` - Connection factory
- ✅ `Connection` - Connection class
- ✅ `Cursor` - Cursor class
- ✅ `execute()`, `executemany()` - Query execution
- ✅ `fetchone()`, `fetchall()`, `fetchmany()` - Result fetching
- ✅ `begin()`, `commit()`, `rollback()` - Transactions
- ✅ `transaction()` - Transaction context manager
- ✅ Exception types: `Error`, `OperationalError`, `ProgrammingError`, `IntegrityError`
- ✅ Parameterized queries (named and positional)
- ✅ Row factories (`dict`, `tuple`, callable)
- ✅ PRAGMA settings
- ✅ Connection string URIs

### Enhanced APIs

rapsqlite includes additional methods not in aiosqlite:

- `fetch_all()` - Fetch all rows (returns list)
- `fetch_one()` - Fetch single row (raises if not found)
- `fetch_optional()` - Fetch single row or None
- `last_insert_rowid()` - Get last insert ID
- `changes()` - Get number of affected rows
- `init_hook` - Database initialization hook (rapsqlite-specific)
- Schema introspection methods (`get_tables()`, `get_table_info()`, etc.)

## Code Examples

### Basic Connection

```python
# Works the same in both libraries
import rapsqlite as aiosqlite

async with aiosqlite.connect("example.db") as db:
    await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
    await db.execute("INSERT INTO test (value) VALUES ('hello')")
    rows = await db.fetch_all("SELECT * FROM test")
    print(rows)  # [[1, 'hello']]
```

### Parameterized Queries

```python
# Named parameters
await db.execute(
    "INSERT INTO users (name, email) VALUES (:name, :email)",
    {"name": "Alice", "email": "alice@example.com"}
)

# Positional parameters
await db.execute(
    "INSERT INTO users (name, email) VALUES (?, ?)",
    ["Bob", "bob@example.com"]
)
```

### Transactions

```python
# Explicit transaction
await db.begin()
try:
    await db.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
    await db.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2")
    await db.commit()
except Exception:
    await db.rollback()

# Transaction context manager
async with db.transaction():
    await db.execute("INSERT INTO test (value) VALUES (1)")
    await db.execute("INSERT INTO test (value) VALUES (2)")
```

### Cursors

```python
cursor = db.cursor()
await cursor.execute("SELECT * FROM test")
row = await cursor.fetchone()
rows = await cursor.fetchmany(10)
all_rows = await cursor.fetchall()
```

### Row Factories

```python
# Dict row factory
db.row_factory = "dict"
rows = await db.fetch_all("SELECT * FROM users")
# rows[0] = {"id": 1, "name": "Alice"}

# Tuple row factory
db.row_factory = "tuple"
rows = await db.fetch_all("SELECT * FROM users")
# rows[0] = (1, "Alice")

# Custom row factory
def my_factory(row):
    return {"id": row[0], "name": row[1].upper()}
db.row_factory = my_factory
```

## Differences and Limitations

For a detailed compatibility analysis based on running the aiosqlite test suite, see [AIOSQLITE_COMPATIBILITY_ANALYSIS.md](AIOSQLITE_COMPATIBILITY_ANALYSIS.md).

### Known Differences

1. **`async with db.execute(...)` Pattern**: aiosqlite supports using `execute()` as an async context manager that returns a cursor. rapsqlite uses explicit cursor creation or `fetch_*()` methods instead.

   **aiosqlite:**
   ```python
   async with db.execute("SELECT * FROM users") as cursor:
       rows = await cursor.fetchall()
   ```
   
   **rapsqlite:**
   ```python
   # Option 1: Use fetch methods (recommended)
   rows = await db.fetch_all("SELECT * FROM users")
   
   # Option 2: Use explicit cursor
   cursor = db.cursor()
   await cursor.execute("SELECT * FROM users")
   rows = await cursor.fetchall()
   ```

2. **Connection Properties**: Some aiosqlite connection properties are not implemented:
   - `db.total_changes` - Not implemented
   - `db.in_transaction` - Not implemented
   - `db.text_factory` - Not implemented

3. **`aiosqlite.Row` Class**: rapsqlite doesn't have a `Row` class, but supports dict row factory:
   ```python
   # aiosqlite
   db.row_factory = aiosqlite.Row
   
   # rapsqlite
   db.row_factory = "dict"  # Provides similar functionality
   ```

4. **`executescript()` Method**: Not implemented in rapsqlite. Use separate execute calls or transactions instead.

5. **Backup to sqlite3.Connection**: The `backup()` method only supports backing up to another `rapsqlite.Connection`. Backing up to Python's standard `sqlite3.Connection` is not supported due to SQLite library instance incompatibility.

   **Workaround**: Use `rapsqlite.Connection` for both source and target:
   ```python
   async with Connection("source.db") as source:
       async with Connection("target.db") as target:
           await source.backup(target)
   ```

2. **init_hook parameter**: This is a rapsqlite-specific enhancement for automatic database initialization. It's not available in aiosqlite.

### Performance Characteristics

- **Connection pooling**: rapsqlite uses connection pooling internally. The default pool size is 1, but can be configured.
- **Prepared statements**: sqlx (the underlying library) caches prepared statements per connection automatically.
- **True async**: All operations execute outside the GIL, providing better concurrency under load.

## Advanced Features

### Database Initialization Hooks

rapsqlite supports automatic database initialization:

```python
async def init_hook(conn):
    """Initialize database schema and data."""
    await conn.execute("""
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT UNIQUE
        )
    """)
    await conn.set_pragma("foreign_keys", True)

async with Connection("example.db", init_hook=init_hook) as conn:
    # Tables are already created and initialized
    users = await conn.fetch_all("SELECT * FROM users")
```

### Schema Introspection

rapsqlite provides comprehensive schema introspection:

```python
# Get all tables
tables = await conn.get_tables()

# Get table information
columns = await conn.get_table_info("users")

# Get indexes
indexes = await conn.get_indexes("users")

# Get foreign keys
foreign_keys = await conn.get_foreign_keys("posts")

# Get comprehensive schema
schema = await conn.get_schema("users")
```

### Connection Configuration

```python
# PRAGMA settings at connection time
async with connect("example.db", pragmas={
    "journal_mode": "WAL",
    "synchronous": "NORMAL"
}) as conn:
    # PRAGMAs are automatically applied

# Pool configuration
conn = Connection("example.db")
conn.pool_size = 5
conn.connection_timeout = 30
```

## Troubleshooting

### Import Errors

**Problem**: `ImportError: Could not import _rapsqlite`

**Solution**: Make sure rapsqlite is built. If installing from source:
```bash
pip install maturin
maturin develop
```

### Performance Issues

**Problem**: Queries seem slower than expected

**Solution**: 
- Ensure you're using parameterized queries (not string formatting)
- Use connection pooling for concurrent operations
- Consider using `execute_many()` for batch inserts

### Transaction Issues

**Problem**: "Transaction connection not available" error

**Solution**: Make sure you call `begin()` before using transaction methods, or use the `transaction()` context manager.

### Backup Issues

**Problem**: Segmentation fault when backing up to sqlite3.Connection

**Solution**: Use `rapsqlite.Connection` for both source and target connections. See [Backup Limitations](#known-differences) above.

## Testing Your Migration

1. **Run existing tests**: Your aiosqlite tests should work with minimal changes
2. **Use compatibility tests**: See `tests/test_dropin_replacement.py` for examples
3. **Verify performance**: Use benchmarks to ensure performance meets expectations

## Getting Help

- **Documentation**: See [README.md](../README.md) for full API documentation
- **Issues**: Report issues on [GitHub](https://github.com/eddiethedean/rapsqlite/issues)
- **Roadmap**: See [docs/ROADMAP.md](ROADMAP.md) for planned features

## Example: Complete Migration

Here's a complete example of migrating an application:

**Before (aiosqlite):**
```python
import aiosqlite

async def main():
    async with aiosqlite.connect("app.db") as db:
        await db.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)")
        await db.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        async with db.execute("SELECT * FROM users") as cursor:
            rows = await cursor.fetchall()
            print(rows)

asyncio.run(main())
```

**After (rapsqlite):**
```python
import rapsqlite as aiosqlite

async def main():
    async with aiosqlite.connect("app.db") as db:
        await db.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)")
        await db.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        # Option 1: Use fetch_all (rapsqlite enhancement)
        rows = await db.fetch_all("SELECT * FROM users")
        print(rows)
        # Option 2: Use cursor (same as aiosqlite)
        cursor = db.cursor()
        await cursor.execute("SELECT * FROM users")
        rows = await cursor.fetchall()
        print(rows)

asyncio.run(main())
```

The migration is complete! Your code now uses true async SQLite operations.
