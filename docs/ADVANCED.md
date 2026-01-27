# Advanced Usage Guide

This guide covers advanced usage patterns, best practices, performance tuning, and common anti-patterns for `rapsqlite`.

## Table of Contents

- [Connection Pooling](#connection-pooling)
- [Transaction Patterns](#transaction-patterns)
- [Error Handling Strategies](#error-handling-strategies)
- [Performance Tuning](#performance-tuning)
- [Common Anti-Patterns](#common-anti-patterns)
- [Best Practices](#best-practices)

## Connection Pooling

### Understanding Connection Pools

`rapsqlite` uses connection pooling internally. The default pool size is 1, but you can configure it:

```python
from rapsqlite import Connection

# Create connection with custom pool size
conn = Connection("example.db")
conn.pool_size = 5  # Allow up to 5 concurrent connections
conn.connection_timeout = 30  # 30 second timeout for acquiring connections
```

### When to Increase Pool Size

- **Concurrent operations**: If you have many concurrent database operations, increase pool size
- **Long-running queries**: If queries take a long time, more connections allow better concurrency
- **Mixed read/write workloads**: Separate connections for reads and writes

### Pool Size Guidelines

- **Default (1)**: Good for single-threaded async applications or low concurrency
- **Small (2-5)**: Good for moderate concurrency, most web applications
- **Large (10+)**: Only for high-concurrency scenarios, be mindful of SQLite's write serialization

**Note**: SQLite serializes writes, so increasing pool size mainly helps with concurrent reads.

## Transaction Patterns

### Explicit Transactions

```python
async with connect("example.db") as conn:
    await conn.begin()
    try:
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
        await conn.commit()
    except Exception:
        await conn.rollback()
        raise
```

### Transaction Context Manager (Recommended)

```python
async with connect("example.db") as conn:
    async with conn.transaction():
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
        # Automatically commits on success, rolls back on exception
```

### Nested Transactions

SQLite doesn't support true nested transactions, but you can use savepoints:

```python
async with connect("example.db") as conn:
    await conn.begin()
    try:
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        
        # Use savepoint for nested transaction-like behavior
        await conn.execute("SAVEPOINT sp1")
        try:
            await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
            await conn.execute("RELEASE SAVEPOINT sp1")
        except Exception:
            await conn.execute("ROLLBACK TO SAVEPOINT sp1")
            raise
        
        await conn.commit()
    except Exception:
        await conn.rollback()
```

## Error Handling Strategies

### Handling Specific Errors

```python
from rapsqlite import IntegrityError, OperationalError, ProgrammingError

async with connect("example.db") as conn:
    try:
        await conn.execute("INSERT INTO users (email) VALUES (?)", ["duplicate@example.com"])
    except IntegrityError as e:
        # Handle constraint violation
        print(f"Integrity error: {e}")
    except OperationalError as e:
        # Handle operational errors (database locked, etc.)
        print(f"Operational error: {e}")
    except ProgrammingError as e:
        # Handle SQL syntax errors
        print(f"Programming error: {e}")
```

### Retry Logic for Locked Database

```python
import asyncio
from rapsqlite import OperationalError

async def execute_with_retry(conn, query, params, max_retries=3):
    for attempt in range(max_retries):
        try:
            await conn.execute(query, params)
            return
        except OperationalError as e:
            if "database is locked" in str(e).lower() and attempt < max_retries - 1:
                await asyncio.sleep(0.1 * (attempt + 1))  # Exponential backoff
                continue
            raise
```

### Error Context

Always include context in error messages:

```python
try:
    await conn.execute("INSERT INTO users (name) VALUES (?)", [name])
except IntegrityError as e:
    raise IntegrityError(f"Failed to insert user '{name}': {e}") from e
```

## Performance Tuning

### Use Parameterized Queries

**Good:**
```python
await conn.execute("INSERT INTO users (name) VALUES (?)", [name])
```

**Bad:**
```python
await conn.execute(f"INSERT INTO users (name) VALUES ('{name}')")  # SQL injection risk, no caching
```

### Batch Operations

Use `execute_many()` for batch inserts:

```python
# Good: Single transaction, prepared statement reuse
params = [[f"user_{i}"] for i in range(1000)]
await conn.execute_many("INSERT INTO users (name) VALUES (?)", params)
```

```python
# Bad: Many individual transactions
for i in range(1000):
    await conn.execute("INSERT INTO users (name) VALUES (?)", [f"user_{i}"])
```

### Connection Reuse

Reuse connections when possible:

```python
# Good: Reuse connection
async with connect("example.db") as conn:
    for i in range(100):
        await conn.execute("INSERT INTO test (value) VALUES (?)", [i])
```

```python
# Bad: Create new connection for each operation
for i in range(100):
    async with connect("example.db") as conn:
        await conn.execute("INSERT INTO test (value) VALUES (?)", [i])
```

### PRAGMA Optimization

Configure SQLite for your workload:

```python
# For read-heavy workloads
async with connect("example.db", pragmas={
    "journal_mode": "WAL",  # Write-Ahead Logging
    "synchronous": "NORMAL",  # Balance safety and performance
    "cache_size": "-64000",  # 64MB cache (negative = KB)
}) as conn:
    # Your operations
    pass

# For write-heavy workloads
async with connect("example.db", pragmas={
    "journal_mode": "WAL",
    "synchronous": "FULL",  # Maximum safety
    "wal_autocheckpoint": "1000",  # Checkpoint every 1000 pages
}) as conn:
    # Your operations
    pass
```

### Prepared Statement Caching

`rapsqlite` automatically caches prepared statements. To maximize cache hits:

- Use consistent query formatting (normalization happens automatically)
- Reuse the same query strings with different parameters
- Keep connections alive for repeated queries

## Common Anti-Patterns

### ❌ Not Using Transactions for Multiple Operations

```python
# Bad: Each operation is a separate transaction
await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")
await conn.execute("INSERT INTO accounts (balance) VALUES (2000)")
# If second fails, first is already committed
```

```python
# Good: Use transaction
async with conn.transaction():
    await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")
    await conn.execute("INSERT INTO accounts (balance) VALUES (2000)")
```

### ❌ String Formatting Instead of Parameters

```python
# Bad: SQL injection risk, no prepared statement caching
await conn.execute(f"SELECT * FROM users WHERE name = '{name}'")
```

```python
# Good: Parameterized query
await conn.execute("SELECT * FROM users WHERE name = ?", [name])
```

### ❌ Fetching All Rows When Only One Needed

```python
# Bad: Fetches all rows, then takes first
rows = await conn.fetch_all("SELECT * FROM users WHERE id = ?", [user_id])
user = rows[0] if rows else None
```

```python
# Good: Fetch only what you need
user = await conn.fetch_optional("SELECT * FROM users WHERE id = ?", [user_id])
```

### ❌ Not Handling Errors in Transactions

```python
# Bad: Exception leaves transaction open
async with conn.transaction():
    await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
    # If exception occurs, transaction might not rollback properly
```

```python
# Good: Transaction context manager handles rollback automatically
async with conn.transaction():
    await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
    # Automatically rolls back on exception
```

### ❌ Creating Too Many Connections

```python
# Bad: Creates new connection for each operation
async def get_user(user_id):
    async with connect("example.db") as conn:
        return await conn.fetch_one("SELECT * FROM users WHERE id = ?", [user_id])
```

```python
# Good: Reuse connection or use connection pool
async with connect("example.db") as conn:
    user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [user_id])
```

## Best Practices

### 1. Always Use Context Managers

```python
# Good
async with connect("example.db") as conn:
    await conn.execute("CREATE TABLE test (id INTEGER)")
```

### 2. Use Transactions for Related Operations

```python
async with conn.transaction():
    await conn.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
    await conn.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2")
```

### 3. Handle Errors Appropriately

```python
try:
    await conn.execute("INSERT INTO users (email) VALUES (?)", [email])
except IntegrityError:
    # Handle duplicate email
    pass
```

### 4. Use Appropriate Fetch Methods

- `fetch_all()`: When you need all rows
- `fetch_one()`: When you expect exactly one row
- `fetch_optional()`: When you might have zero or one row
- `Cursor.fetchmany()`: When processing large result sets in chunks

### 5. Configure PRAGMAs for Your Workload

```python
# Read-heavy
pragmas = {"journal_mode": "WAL", "synchronous": "NORMAL"}

# Write-heavy
pragmas = {"journal_mode": "WAL", "synchronous": "FULL"}

# Development
pragmas = {"journal_mode": "MEMORY", "synchronous": "OFF"}
```

### 6. Use Database Initialization Hooks

```python
async def init_db(conn):
    await conn.execute("""
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        )
    """)
    await conn.set_pragma("foreign_keys", True)

async with Connection("example.db", init_hook=init_db) as conn:
    # Database is initialized automatically
    pass
```

### 7. Monitor Connection Pool Usage

```python
# Adjust pool size based on your workload
conn = Connection("example.db")
conn.pool_size = 5  # For moderate concurrency
conn.connection_timeout = 30  # 30 second timeout
```

### 8. Use Schema Introspection

```python
# Check if table exists before creating
tables = await conn.get_tables()
if "users" not in tables:
    await conn.execute("CREATE TABLE users ...")

# Get table structure
columns = await conn.get_table_info("users")
```

## Performance Monitoring

### Query Timing

```python
import time

start = time.perf_counter()
rows = await conn.fetch_all("SELECT * FROM users")
elapsed = time.perf_counter() - start
print(f"Query took {elapsed * 1000:.2f}ms")
```

### Connection Pool Metrics

Monitor connection pool usage by tracking:
- Number of concurrent operations
- Connection acquisition timeouts
- Pool size vs. actual usage

## Troubleshooting Performance Issues

### Slow Queries

1. **Check if using parameterized queries**: String formatting prevents prepared statement caching
2. **Verify indexes**: Use `get_indexes()` to check table indexes
3. **Analyze query plans**: Use `EXPLAIN QUERY PLAN` to understand query execution
4. **Check PRAGMA settings**: Ensure appropriate settings for your workload

### High Memory Usage

1. **Reduce pool size**: Smaller pool uses less memory
2. **Use fetchmany()**: Process large result sets in chunks
3. **Clear query cache**: Connection close clears prepared statement cache

### Database Locked Errors

1. **Increase timeout**: Set `connection_timeout` higher
2. **Use WAL mode**: `journal_mode = "WAL"` allows concurrent reads
3. **Reduce transaction duration**: Keep transactions short
4. **Implement retry logic**: See error handling section above

## Further Reading

- [SQLite Performance Tuning](https://www.sqlite.org/performance.html)
- [SQLite PRAGMA Documentation](https://www.sqlite.org/pragma.html)
- [rapsqlite README](../README.md) - Full API documentation
- [Migration Guide](MIGRATION.md) - Migrating from aiosqlite
