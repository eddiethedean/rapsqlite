# rapsqlite

**True async SQLite — no fake async, no GIL stalls.**

[![PyPI version](https://img.shields.io/pypi/v/rapsqlite.svg)](https://pypi.org/project/rapsqlite/)
[![Downloads](https://pepy.tech/badge/rapsqlite)](https://pepy.tech/project/rapsqlite)
[![Python 3.8+](https://img.shields.io/badge/python-3.8+-blue.svg)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Overview

`rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx. Unlike libraries that wrap blocking database calls in `async` syntax, `rapsqlite` guarantees that all database operations execute **outside the Python GIL**, ensuring event loops never stall under load.

**Roadmap Goal**: Achieve drop-in replacement compatibility with `aiosqlite`, enabling seamless migration with true async performance. See [ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/main/ROADMAP.md) for details.

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

## Requirements

- Python 3.8+
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

```python
import asyncio
import tempfile
import os
from rapsqlite import Connection

async def main():
    # Create a database file (ensure it exists first)
    with tempfile.NamedTemporaryFile(suffix='.db', delete=False) as f:
        db_path = f.name
    
    try:
        # Create connection
        conn = Connection(db_path)
        
        # Create table
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)")
        
        # Insert data
        await conn.execute("INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')")
        await conn.execute("INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')")
        
        # Fetch all rows
        rows = await conn.fetch_all("SELECT * FROM users")
        print(rows)
        # Output: [['1', 'Alice', 'alice@example.com'], ['2', 'Bob', 'bob@example.com']]
        
        # Query specific rows
        alice_rows = await conn.fetch_all("SELECT * FROM users WHERE name = 'Alice'")
        print(alice_rows)
        # Output: [['1', 'Alice', 'alice@example.com']]
    finally:
        # Cleanup
        if os.path.exists(db_path):
            os.unlink(db_path)

asyncio.run(main())
```

### Concurrent Database Operations

```python
import asyncio
import tempfile
import os
from rapsqlite import Connection

async def main():
    with tempfile.NamedTemporaryFile(suffix='.db', delete=False) as f:
        db_path = f.name
    
    try:
        conn = Connection(db_path)
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
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)

asyncio.run(main())
```

## API Reference

### `Connection(path: str)`

Create a new async SQLite connection.

**Parameters:**
- `path` (str): Path to the SQLite database file

**Example:**
```python
conn = Connection("example.db")
```

### `Connection.execute(query: str) -> None`

Execute a SQL statement (CREATE, INSERT, UPDATE, DELETE, etc.).

**Parameters:**
- `query` (str): SQL query to execute

**Raises:**
- `IOError`: If the query execution fails

### `Connection.fetch_all(query: str) -> List[List[str]]`

Execute a SELECT query and return all rows.

**Parameters:**
- `query` (str): SELECT query to execute

**Returns:**
- `List[List[str]]`: List of rows, where each row is a list of string values

**Raises:**
- `IOError`: If the query execution fails

## Benchmarks

This package passes the [Fake Async Detector](https://github.com/eddiethedean/rap-bench). Benchmarks are available in the [rap-bench](https://github.com/eddiethedean/rap-bench) repository.

Run the detector yourself:

```bash
pip install rap-bench
rap-bench detect rapsqlite
```

## Roadmap

See [ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/main/ROADMAP.md) for detailed development plans. Key goals include:
- Drop-in replacement for `aiosqlite` (Phase 1)
- Connection pooling and transaction support
- Prepared statements and parameterized queries
- Comprehensive SQLite feature support
- Advanced query optimization and ecosystem integration

## Related Projects

- [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) - Philosophy and guarantees
- [rap-bench](https://github.com/eddiethedean/rap-bench) - Fake Async Detector CLI
- [rapfiles](https://github.com/eddiethedean/rapfiles) - True async filesystem I/O
- [rapcsv](https://github.com/eddiethedean/rapcsv) - Streaming async CSV

## Limitations (v0.0.2)

**Current limitations:**
- No transaction support
- No prepared statements or parameterized queries
- Limited SQL dialect support
- All values returned as strings (limited type support)
- Not yet a drop-in replacement for `aiosqlite` (goal for Phase 1)
- Not designed for synchronous use cases

**Recent improvements (v0.0.2):**
- ✅ Security fixes: Upgraded dependencies (pyo3 0.27, pyo3-async-runtimes 0.27, sqlx 0.8)
- ✅ Connection pooling: Connection now reuses connection pool across operations (major performance improvement)
- ✅ Input validation: Added path validation (non-empty, no null bytes)
- ✅ Improved error handling: Enhanced error messages with database path and query context
- ✅ Type stubs: Added `.pyi` type stubs for better IDE support and type checking

**Roadmap**: See [ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/main/ROADMAP.md) for planned improvements. Our goal is to achieve drop-in replacement compatibility with `aiosqlite` while providing true async performance with GIL-independent database operations.

## Contributing

Contributions are welcome! Please see our [contributing guidelines](https://github.com/eddiethedean/rapsqlite/blob/main/CONTRIBUTING.md) (coming soon).

## License

MIT

## Changelog

See [CHANGELOG.md](https://github.com/eddiethedean/rapsqlite/blob/main/CHANGELOG.md) (coming soon) for version history.
