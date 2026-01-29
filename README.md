# rapsqlite

**True async SQLite — no fake async, no GIL stalls.**

[![PyPI version](https://img.shields.io/pypi/v/rapsqlite.svg)](https://pypi.org/project/rapsqlite/)
[![Downloads](https://pepy.tech/badge/rapsqlite)](https://pepy.tech/project/rapsqlite)
[![Python 3.8+](https://img.shields.io/badge/python-3.8+-blue.svg)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Documentation](https://readthedocs.org/projects/rapsqlite/badge/?version=latest)](https://rapsqlite.readthedocs.io/en/latest/)

## Overview

`rapsqlite` provides true async SQLite operations for Python, backed by Rust, Tokio, and sqlx. Unlike libraries that wrap blocking database calls in `async` syntax, `rapsqlite` guarantees that all database operations execute **outside the Python GIL**, ensuring event loops never stall under load.

📚 **[Full Documentation](https://rapsqlite.readthedocs.io/en/latest/)** | **Roadmap Goal**: Achieve drop-in replacement compatibility with `aiosqlite`, enabling seamless migration with true async performance. See [docs/ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/master/docs/ROADMAP.md) for details.

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
- Python development headers (included with most Python installations)

## Installation

```bash
pip install rapsqlite
```

### Building from Source

**Prerequisites:**
- Python 3.8+ with development headers installed
- Rust 1.70+ and Cargo

**Installation:**
```bash
git clone https://github.com/eddiethedean/rapsqlite.git
cd rapsqlite
pip install maturin
maturin develop
```

**Note**: Python development headers are required for building. They're typically included with Python installations, but on some Linux distributions you may need to install `python3-dev` or `python3-devel` package. Use `maturin develop` instead of `cargo build` for development, as maturin automatically handles Python library linking.

## Documentation

📖 **Full documentation is available at [rapsqlite.readthedocs.io](https://rapsqlite.readthedocs.io/en/latest/)**

The documentation includes:
- [Quickstart Guide](https://rapsqlite.readthedocs.io/en/latest/quickstart.html) - Get started in minutes
- [API Reference](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html) - Complete API documentation
- [Migration Guide](https://rapsqlite.readthedocs.io/en/latest/guides/migration-guide.html) - Migrating from aiosqlite
- [Performance Guide](https://rapsqlite.readthedocs.io/en/latest/guides/performance.html) - Optimization tips
- [Advanced Usage](https://rapsqlite.readthedocs.io/en/latest/guides/advanced-usage.html) - Advanced features and patterns

---

## Quick Start

See the [Quickstart Guide](https://rapsqlite.readthedocs.io/en/latest/quickstart.html) for getting started with `rapsqlite`.

For complete API documentation, see the [API Reference](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html).

## API Reference

Complete API documentation is available at [rapsqlite.readthedocs.io](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html):

- [Connection API](https://rapsqlite.readthedocs.io/en/latest/api-reference/connection.html)
- [Cursor API](https://rapsqlite.readthedocs.io/en/latest/api-reference/cursor.html)
- [Exception Classes](https://rapsqlite.readthedocs.io/en/latest/api-reference/exceptions.html)
- [Row Factory](https://rapsqlite.readthedocs.io/en/latest/api-reference/row.html)

### Backup Support

The `Connection.backup()` method supports backing up to both `rapsqlite.Connection` and Python's standard `sqlite3.Connection` targets. For `sqlite3.Connection` targets, the backup uses Python's sqlite3 backup API on the on-disk database file (file-backed databases only; `:memory:` and non-file URIs are not supported).

For more details, see the [Backup documentation](https://rapsqlite.readthedocs.io/en/latest/api-reference/connection.html#rapsqlite.Connection.backup) in the API reference.

## Performance

This package passes the [Fake Async Detector](https://github.com/eddiethedean/rap-bench). For detailed performance benchmarks and optimization tips, see the [Performance Guide](https://rapsqlite.readthedocs.io/en/latest/guides/performance.html).

**Key advantages:**
- **True async**: All operations execute outside the Python GIL
- **Prepared statement caching**: Automatic query optimization via sqlx (2-5x faster for repeated queries)
- **Better throughput**: Superior performance under concurrent load due to GIL independence
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

- **`iterdump()`**: `rapsqlite` supports both async iteration (aiosqlite-style) and await-to-list:

  ```python
  # aiosqlite and rapsqlite (async iterator)
  lines = [line async for line in db.iterdump()]

  # rapsqlite
  lines = await db.iterdump()
  dump_sql = "\n".join(lines)
  ```

- **`backup()` targets**: `rapsqlite` supports backups to both `rapsqlite.Connection` and `sqlite3.Connection` targets. For `sqlite3.Connection` targets, only file-backed databases are supported (not `:memory:` or non-file URIs).

**See the [Migration Guide](https://rapsqlite.readthedocs.io/en/latest/guides/migration-guide.html) for a complete migration guide** with:
- Step-by-step migration instructions
- Code examples for common patterns
- API differences and limitations
- Troubleshooting guide
- Performance considerations

**Compatibility Analysis**: See the [Compatibility Guide](https://rapsqlite.readthedocs.io/en/latest/guides/compatibility.html) for detailed analysis based on running the aiosqlite test suite. Overall compatibility: **~95%** for core use cases (updated 2026-01-26). All high-priority compatibility features implemented including `total_changes()`, `in_transaction()`, `executescript()`, `load_extension()`, `text_factory`, `Row` class, and async iteration on cursors.

## Roadmap

See [docs/ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/master/docs/ROADMAP.md) for detailed development plans. Key goals include:

- ✅ Phase 1: Connection lifecycle, transactions, type system, error handling, cursor API (complete)
- ✅ Phase 2: Parameterized queries, cursor improvements, connection/pool configuration, row factory, transaction context managers, advanced callbacks, database dump/backup, schema introspection, database initialization hooks, prepared statement caching (complete)
- ⏳ Phase 3: Advanced SQLite features and ecosystem integration

## Related Projects

- [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) - Philosophy and guarantees
- [rap-bench](https://github.com/eddiethedean/rap-bench) - Fake Async Detector CLI
- [rapfiles](https://github.com/eddiethedean/rapfiles) - True async filesystem I/O
- [rapcsv](https://github.com/eddiethedean/rapcsv) - Streaming async CSV

## Changelog

See [CHANGELOG.md](https://github.com/eddiethedean/rapsqlite/blob/master/CHANGELOG.md) for detailed release notes and version history.

## Limitations (v0.2.0)

**Current limitations:**
- ⏳ Not designed for synchronous use cases
- ⚠️ **Backup to `sqlite3.Connection`**: The `Connection.backup()` method supports backing up to `sqlite3.Connection` targets, but only for file-backed databases (`:memory:` and non-file URIs are not supported). See [Backup Support](#backup-support) above for details.

**Phase 2 (v0.2.0) status:**
- ✅ Phase 2 is complete. See [CHANGELOG.md](https://github.com/eddiethedean/rapsqlite/blob/master/CHANGELOG.md) for the full list of features and compatibility notes.

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

**Roadmap**: See [docs/ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/master/docs/ROADMAP.md) for planned improvements. Our goal is to achieve drop-in replacement compatibility with `aiosqlite` while providing true async performance with GIL-independent database operations.

## Contributing

Contributions are welcome! Please see our [contributing guidelines](https://github.com/eddiethedean/rapsqlite/blob/master/CONTRIBUTING.md).

## License

MIT

