.. rapsqlite documentation master file

Welcome to rapsqlite's documentation!
======================================

**True async SQLite — no fake async, no GIL stalls.**

rapsqlite provides true async SQLite operations for Python, backed by Rust,
Tokio, and sqlx. Unlike libraries that wrap blocking database calls in async
syntax, rapsqlite guarantees that all database operations execute **outside the
Python GIL**, ensuring event loops never stall under load.

.. toctree::
   :maxdepth: 2
   :caption: Getting Started

   installation
   quickstart

.. toctree::
   :maxdepth: 2
   :caption: API Reference

   api-reference/index

.. toctree::
   :maxdepth: 2
   :caption: Guides

   guides/advanced-usage
   guides/migration-guide
   guides/compatibility
   guides/performance

.. toctree::
   :maxdepth: 1
   :caption: Reference

   reference/roadmap

Features
--------

* ✅ **True async** SQLite operations (all operations execute outside Python GIL)
* ✅ **Native Rust-backed** execution (Tokio + sqlx)
* ✅ **Zero Python thread pools** (no fake async)
* ✅ **Event-loop-safe** concurrency under load
* ✅ **GIL-independent** database operations
* ✅ **aiosqlite-compatible API** (~95% compatibility, drop-in replacement)
* ✅ **Prepared statement caching** (automatic via sqlx, 2-5x faster for repeated queries)
* ✅ **Connection pooling** (configurable pool size and timeouts)
* ✅ **Row factories** (dict, tuple, callable, and `rapsqlite.Row` class)
* ✅ **Advanced SQLite features** (callbacks, extensions, schema introspection, backup, dump)
* ✅ **Database initialization hooks** (automatic schema setup)

Why ``rap*``?
-------------

Packages prefixed with **``rap``** stand for **Real Async Python**. Unlike many
libraries that merely wrap blocking I/O in ``async`` syntax, ``rap*`` packages
guarantee that all I/O work is executed **outside the Python GIL** using native
runtimes (primarily Rust). This means event loops are never stalled by hidden
thread pools, blocking syscalls, or cooperative yielding tricks. If a ``rap*`` API
is ``async``, it is *structurally non-blocking by design*, not by convention.

See the `rap-manifesto <https://github.com/eddiethedean/rap-manifesto>`_ for
philosophy and guarantees.

Quick Example
-------------

.. code-block:: python

   import asyncio
   from rapsqlite import connect

   async def main():
       async with connect("example.db") as conn:
           await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
           await conn.execute("INSERT INTO test (value) VALUES ('hello')")
           rows = await conn.fetch_all("SELECT * FROM test")
           print(rows)

   asyncio.run(main())

Indices and tables
==================

* :ref:`genindex`
* :ref:`modindex`
* :ref:`search`
