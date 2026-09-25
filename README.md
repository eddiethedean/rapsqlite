# rapsqlite

**True async SQLite — no fake async, no GIL stalls.**

[![PyPI version](https://img.shields.io/pypi/v/rapsqlite.svg)](https://pypi.org/project/rapsqlite/)
[![Downloads](https://pepy.tech/badge/rapsqlite)](https://pepy.tech/project/rapsqlite)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Documentation](https://readthedocs.org/projects/rapsqlite/badge/?version=latest)](https://rapsqlite.readthedocs.io/en/latest/)

## Overview

`rapsqlite` provides **true async** SQLite for Python, backed by Rust, Tokio, and sqlx. Database operations run **outside the Python GIL**, so the event loop never stalls. Use it as a drop-in replacement for `aiosqlite` with better concurrency and no thread pools.

📚 **[Full Documentation](https://rapsqlite.readthedocs.io/en/latest/)** · [Quickstart](https://rapsqlite.readthedocs.io/en/latest/quickstart.html) · [API Reference](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html) · [ROADMAP](https://github.com/eddiethedean/rapsqlite/blob/master/docs/ROADMAP.md)

## Why `rap*`?

Packages prefixed with **`rap`** stand for **Real Async Python**. Unlike many libraries that merely wrap blocking I/O in `async` syntax, `rap*` packages guarantee that all I/O work is executed **outside the Python GIL** using native runtimes (primarily Rust). This means event loops are never stalled by hidden thread pools, blocking syscalls, or cooperative yielding tricks. If a `rap*` API is `async`, it is *structurally non-blocking by design*, not by convention. The `rap` prefix is a contract: measurable concurrency, real parallelism, and verifiable async behavior under load.

See the [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) for philosophy and guarantees.

## Top Features

- ⚡ **True async** — All SQLite I/O runs outside the Python GIL (Rust + Tokio + sqlx)
- 🚫 **No fake async** — Zero thread pools; event-loop-safe concurrency
- 🔄 **aiosqlite-compatible** — ~95% API parity, drop-in replacement
- 🏊 **Connection pooling** — Configurable size and timeouts
- 🚀 **Prepared statement caching** — Automatic (2–5x faster repeated queries)
- 🐍 **SQLAlchemy 2.0+** — `sqlite+rapsqlite` dialect for async Core and ORM
- 📦 **Alembic** — Full support for async migrations (`alembic init -t async`)

See the [documentation](https://rapsqlite.readthedocs.io/en/latest/) for the full feature list (transactions, cursors, row factories, backup, callbacks, type adapters, and more).

## Requirements

- Python 3.10+ (including Python 3.13 and 3.14)
- Rust 1.70+ (for building from source)
- Python development headers (included with most Python installations)

## Installation

```bash
pip install rapsqlite
```

To verify: run the [installation example](https://rapsqlite.readthedocs.io/en/latest/installation.html#verifying-installation) in the docs (it prints `[[1]]`). For **building from source**, see [Installation](https://rapsqlite.readthedocs.io/en/latest/installation.html#building-from-source).

## Documentation

📖 **[rapsqlite.readthedocs.io](https://rapsqlite.readthedocs.io/en/latest/)** – Quickstart, API reference, migration from aiosqlite, performance, and advanced usage. Code examples are tested and show real output.

---

## Quick Start

```python
import asyncio
from rapsqlite import connect


async def main():
    async with connect("example.db") as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
        rows = await conn.fetch_all("SELECT * FROM users")
        print(rows)


asyncio.run(main())
```

**Output:** `[[1, 'Alice']]`

### SQLAlchemy & Alembic (0.4.0)

Use the `sqlite+rapsqlite` dialect with SQLAlchemy 2.0+ for true async ORM and Core. **Alembic migrations** are fully supported with the async template (`alembic init -t async`).

```python
import asyncio
from sqlalchemy import text
from sqlalchemy.ext.asyncio import create_async_engine


async def main():
    engine = create_async_engine("sqlite+rapsqlite:///app.db")
    async with engine.connect() as conn:
        result = await conn.execute(text("SELECT 1"))
        print(result.scalar())  # 1
    await engine.dispose()


asyncio.run(main())
```

Install with `pip install rapsqlite[sqlalchemy]` (or `pip install rapsqlite sqlalchemy`). For **Alembic**, use `pip install rapsqlite[sqlalchemy] alembic`. See the [Compatibility Guide](https://rapsqlite.readthedocs.io/en/latest/guides/compatibility.html#alembic-with-rapsqlite) for AsyncSession, ORM, and step-by-step Alembic setup.

For more (transactions, cursors, row factories), see the [Quickstart Guide](https://rapsqlite.readthedocs.io/en/latest/quickstart.html) and [API Reference](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html). Code examples in the docs are tested and show real output.

## API Reference

Complete API documentation is at [rapsqlite.readthedocs.io](https://rapsqlite.readthedocs.io/en/latest/api-reference/index.html):

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

For process-local cache lookups, 0.5 adds `fetch_scalar()`/`fetch_blob()`, an
opt-in `raw_fetch_scalar()` path, reusable `conn.prepare(...)` operations, and
opt-in `session_affinity=True`. These APIs are measured separately from the
general row API; use the [Phase 0.5 benchmark](https://github.com/eddiethedean/rapsqlite/blob/main/benchmarks/phase5_hotpath.py)
and do not treat unmatched raw SQLite or published Redis figures as a direct
speed claim.

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

- **`total_changes` / `in_transaction`**: both `aiosqlite` and `rapsqlite` expose these as **properties** (same API):

  ```python
  # aiosqlite and rapsqlite
  changes = db.total_changes
  in_tx = db.in_transaction
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

See [docs/ROADMAP.md](https://github.com/eddiethedean/rapsqlite/blob/master/docs/ROADMAP.md) for full details.

- ✅ **0.1–0.3** – Async core, aiosqlite compatibility, callbacks, pooling, True Async DBAPI, SQLAlchemy/Alembic integration, and advanced SQLite features
- ✅ **0.4** – Post-`v0.3.3` compatibility, security, CI, SQLAlchemy 2.1 support, and release stabilization (`v0.4.0`)
- ✅ **0.5** – Implementation complete: measured low-latency execution, hot-path reductions, scalar/BLOB and prepared-query paths, opt-in session affinity, and an opt-in raw path; release validation remains
- 📋 **0.6** – Cache-specific APIs, bulk operations, and concurrent workloads
- 📋 **0.7–0.9** – Pooling, observability, reliability, ecosystem tooling, and stabilization toward 1.0

## Related Projects

- [rap-manifesto](https://github.com/eddiethedean/rap-manifesto) - Philosophy and guarantees
- [rap-bench](https://github.com/eddiethedean/rap-bench) - Fake Async Detector CLI
- [rapfiles](https://github.com/eddiethedean/rapfiles) - True async filesystem I/O
- [rapcsv](https://github.com/eddiethedean/rapcsv) - Streaming async CSV

## Changelog

See [CHANGELOG.md](https://github.com/eddiethedean/rapsqlite/blob/master/CHANGELOG.md) for detailed release notes and version history.

## Limitations

- **Async only** – Not designed for synchronous use; use `sqlite3` for sync code.
- **Backup to `sqlite3.Connection`** – Supported for file-backed databases only (not `:memory:` or non-file URIs). See [Backup Support](#backup-support) above.

Release history and full feature list: [CHANGELOG.md](https://github.com/eddiethedean/rapsqlite/blob/master/CHANGELOG.md).

## Contributing

Contributions are welcome! Please see our [contributing guidelines](https://github.com/eddiethedean/rapsqlite/blob/master/CONTRIBUTING.md).

## License

MIT
