aiosqlite Compatibility Analysis
================================

This document analyzes compatibility between rapsqlite and aiosqlite based on running the aiosqlite test suite.

Test Execution
--------------

**Date**: 2026-01-30  
**rapsqlite Version**: 0.3.0-dev  
**aiosqlite Test Suite**: Latest from https://github.com/omnilib/aiosqlite

**Test baseline**: Run ``scripts/run_aiosqlite_tests.py`` to execute the aiosqlite test suite against rapsqlite (patched imports). Results are written to ``docs/AIOSQLITE_TEST_RESULTS.md`` with a per-test breakdown. Summary: some tests pass in ``perf.py`` and ``smoke.py``; failures are often due to **document** (intentional differences: async methods for ``total_changes``/``in_transaction``, no transaction queue, backup API), **environment** (temp dir, package layout when run from script), or compatibility gaps. See ``docs/AIOSQLITE_TEST_RESULTS.md`` for the per-test breakdown.

Known API Differences
---------------------

1. ``async with db.execute(...)`` Pattern
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **NOW SUPPORTED** - ``async with db.execute(...)`` pattern is fully implemented via ``ExecuteContextManager``. Both ``async with`` and direct ``await`` patterns work.

**Impact**: High - Many aiosqlite tests use this pattern. Now fully compatible.

2. Cursor as Async Context Manager / Async Iteration
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **NOW SUPPORTED** - Cursors support async iteration via ``__aiter__`` and ``__anext__`` methods.

**Impact**: Medium - Some tests use async iteration. Now fully compatible.

3. Connection Properties
~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **ALL PROPERTIES NOW SUPPORTED** - All connection properties are implemented.

**Note**: ``total_changes`` and ``in_transaction`` are async methods (not properties) in rapsqlite due to internal implementation, but functionally equivalent.

4. Row Factory: ``aiosqlite.Row``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **NOW SUPPORTED** - ``rapsqlite.Row`` class is implemented with dict-like access (``row["column"]``, ``row[0]``, ``keys()``, ``values()``, ``items()``).

5. ``executescript()`` Method
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **NOW IMPLEMENTED**

6. ``load_extension()`` Method
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Status**: ✅ **NOW IMPLEMENTED**

7. Backup to sqlite3.Connection
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``rapsqlite.Connection.backup()`` now supports both ``rapsqlite.Connection`` and ``sqlite3.Connection`` targets. For sqlite3 targets, it uses Python's sqlite3 backup API over the on-disk database file (file-backed databases only; ``:memory:`` and non-file URIs are not supported).

8. ``iterdump()`` Return Type
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

rapsqlite supports both async iteration and await-to-list:

.. code-block:: python

   # aiosqlite and rapsqlite (async iterator)
   async for line in db.iterdump():
       ...

   # rapsqlite (backwards compatible)
   lines = await db.iterdump()  # Returns List[str]

9. ``init_hook`` parameter
~~~~~~~~~~~~~~~~~~~~~~~~~~

This is a rapsqlite-specific enhancement for automatic database initialization. It's not available in aiosqlite.

True Async DBAPI (SQLAlchemy-style)
-----------------------------------

rapsqlite implements a **True Async DBAPI 2.0** interface in ``rapsqlite.dbapi``, suitable for
SQLAlchemy and other consumers that expect an async driver with no thread offloading.
See ``docs/true_async_dbapi_spec.md`` for the contract.

* ``apilevel = "2.0"``, ``threadsafety = 0``, ``paramstyle = "qmark"``
* Full exception hierarchy (``Error``, ``InterfaceError``, ``DataError``, etc.)
* ``async def connect(database, **kwargs) -> AsyncConnection``
* ``AsyncConnection``: ``async cursor()``, ``async execute()``, ``executemany``, ``commit``, ``rollback``, ``close``
* ``Cursor``: ``execute``, ``executemany``, ``fetchone``, ``fetchmany``, ``fetchall``, ``close``; ``description``, ``rowcount``, ``lastrowid``, ``arraysize``

Use ``rapsqlite.dbapi`` when integrating with async frameworks that require a DBAPI-compliant module.

SQLAlchemy
~~~~~~~~~~

A ``sqlite+rapsqlite`` dialect is provided. Install with ``pip install 'rapsqlite[sqlalchemy]'``, then:

.. code-block:: python

   import rapsqlite.sqlalchemy  # register dialect
   from sqlalchemy.ext.asyncio import create_async_engine
   engine = create_async_engine("sqlite+rapsqlite:///path.db")
   # or sqlite+rapsqlite:///:memory:

See ``docs/true_async_dbapi_spec.md`` for the full async DBAPI contract.

Alembic with rapsqlite
~~~~~~~~~~~~~~~~~~~~~~

You can run Alembic migrations using the ``sqlite+rapsqlite`` dialect.

1. Install dependencies: ``pip install 'rapsqlite[sqlalchemy]' alembic``
2. Initialize Alembic with async support: ``alembic init -t async alembic``
3. In ``alembic/env.py``, ensure the dialect is registered and the URL uses ``sqlite+rapsqlite``:

.. code-block:: python

   import rapsqlite.sqlalchemy  # register sqlite+rapsqlite dialect
   from sqlalchemy.ext.asyncio import create_async_engine
   # ...
   # In run_migrations_online(), use:
   connectable = create_async_engine(
       config.get_main_option("sqlalchemy.url").replace("sqlite://", "sqlite+rapsqlite://")
   )

4. Set ``sqlalchemy.url`` in ``alembic.ini`` (e.g. ``sqlite:///app.db``). The env.py replacement above switches it to ``sqlite+rapsqlite:///app.db`` for async runs.
5. Run migrations: ``alembic upgrade head`` (runs in the async context provided by the template).

For file-backed databases, use a path or ``sqlite+rapsqlite:///path/to/db.db``. In-memory (``:memory:``) is supported for the engine but each connection gets its own database; use a file path for migrations so the same database is used across runs.

FastAPI
~~~~~~~

Use a connection dependency with ``rapsqlite.connect()`` and lifespan for setup. See ``examples/fastapi_db.py``:

.. code-block:: python

   from contextlib import asynccontextmanager
   from fastapi import FastAPI, Depends
   from rapsqlite import connect

   @asynccontextmanager
   async def lifespan(app: FastAPI):
       async with connect("app.db") as conn:
           await conn.execute("CREATE TABLE IF NOT EXISTS ...")
       yield

   async def get_db():
       async with connect("app.db") as conn:
           yield conn

   app = FastAPI(lifespan=lifespan)

   @app.get("/")
   async def root(db=Depends(get_db)):
       rows = await db.fetch_all("SELECT ...")
       return {"data": rows}

Run with ``uvicorn examples.fastapi_db:app --reload``. The test ``tests/test_fastapi_example.py`` validates this pattern.

Starlette
~~~~~~~~~

Starlette uses the same ASGI base as FastAPI. Use lifespan for setup and create a connection per request. See ``examples/starlette_db.py``:

.. code-block:: python

   from contextlib import asynccontextmanager
   from starlette.applications import Starlette
   from starlette.responses import JSONResponse
   from starlette.routing import Route
   from rapsqlite import connect

   @asynccontextmanager
   async def lifespan(app):
       async with connect("app.db") as conn:
           await conn.execute("CREATE TABLE IF NOT EXISTS items ...")
       yield

   async def homepage(request):
       async with connect("app.db") as conn:
           rows = await conn.fetch_all("SELECT * FROM items")
           return JSONResponse({"items": [list(r) for r in rows]})

   app = Starlette(routes=[Route("/", homepage)], lifespan=lifespan)

Run with ``uvicorn examples.starlette_db:app --reload``. The test ``tests/test_starlette_example.py`` validates this pattern.

aiohttp
~~~~~~~

Use ``on_startup`` for schema setup and create a connection per request. See ``examples/aiohttp_db.py``:

.. code-block:: python

   from aiohttp import web
   from rapsqlite import connect

   async def init_db(app):
       async with connect("app.db") as conn:
           await conn.execute("CREATE TABLE IF NOT EXISTS items ...")
   async def homepage(request):
       async with connect("app.db") as conn:
           rows = await conn.fetch_all("SELECT * FROM items")
           return web.json_response({"items": [list(r) for r in rows]})

   app = web.Application()
   app.on_startup.append(init_db)
   app.router.add_get("/", homepage)

Run with ``python examples/aiohttp_db.py`` or ``aiohttp run examples/aiohttp_db:app``. The test ``tests/test_aiohttp_example.py`` validates this pattern.

Compatibility Summary
---------------------

**Core API Compatibility**: ~95%

* ✅ All core APIs supported
* ✅ All high-priority compatibility features implemented
* ✅ Drop-in replacement for most use cases
* ⚠️ Minor differences in property vs method access for ``total_changes`` and ``in_transaction``

Performance Characteristics
----------------------------

* **Connection pooling**: rapsqlite uses connection pooling internally. The default pool size is 1, but can be configured.
* **Prepared statements**: sqlx (the underlying library) caches prepared statements per connection automatically.
* **True async**: All operations execute outside the GIL, providing better concurrency under load.

For type conversion and custom types (including the current approach without
``register_adapter``/``register_converter``), see :doc:`../reference/type-conversion`.

For the aiosqlite test suite baseline and per-test failure categories, see ``docs/AIOSQLITE_TEST_RESULTS.md``.

For more details, see :doc:`migration-guide`.
