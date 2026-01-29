Migration Guide: aiosqlite to rapsqlite
========================================

This guide helps you migrate from ``aiosqlite`` to ``rapsqlite`` for true async SQLite operations with GIL-independent performance.

Quick Start
-----------

The simplest migration is a one-line change:

.. code-block:: python

   # Before (aiosqlite)
   import aiosqlite

   # After (rapsqlite)
   import rapsqlite as aiosqlite

For most applications, this is all you need! ``rapsqlite`` is designed to be a drop-in replacement for ``aiosqlite``.

Why Migrate?
-------------

* **True async**: All database operations execute outside the Python GIL
* **Better performance**: No fake async, no event loop stalls
* **Same API**: Drop-in replacement with minimal code changes
* **Verified**: Passes Fake Async Detector benchmarks

Migration Steps
---------------

1. Install rapsqlite
~~~~~~~~~~~~~~~~~~~~

.. code-block:: bash

   pip install rapsqlite

2. Update Imports
~~~~~~~~~~~~~~~~~

**Option A: Simple alias (recommended)**

.. code-block:: python

   import rapsqlite as aiosqlite

**Option B: Direct import**

.. code-block:: python

   from rapsqlite import connect, Connection

3. Verify Compatibility
~~~~~~~~~~~~~~~~~~~~~~~

Run your existing tests. Most code should work without changes.

API Compatibility
-----------------

Core API (100% Compatible)
~~~~~~~~~~~~~~~~~~~~~~~~~~~

All core aiosqlite APIs are supported:

* ✅ ``connect()`` - Connection factory
* ✅ ``Connection`` - Connection class
* ✅ ``Cursor`` - Cursor class
* ✅ ``execute()``, ``executemany()`` - Query execution
* ✅ ``fetchone()``, ``fetchall()``, ``fetchmany()`` - Result fetching
* ✅ ``begin()``, ``commit()``, ``rollback()`` - Transactions
* ✅ ``transaction()`` - Transaction context manager
* ✅ Exception types: ``Error``, ``OperationalError``, ``ProgrammingError``, ``IntegrityError``
* ✅ Parameterized queries (named and positional)
* ✅ Row factories (``dict``, ``tuple``, callable)
* ✅ PRAGMA settings
* ✅ Connection string URIs

Enhanced APIs
~~~~~~~~~~~~~

rapsqlite includes additional methods not in aiosqlite:

* ``fetch_all()`` - Fetch all rows (returns list)
* ``fetch_one()`` - Fetch single row (raises if not found)
* ``fetch_optional()`` - Fetch single row or None
* ``last_insert_rowid()`` - Get last insert ID
* ``changes()`` - Get number of affected rows
* ``init_hook`` - Database initialization hook (rapsqlite-specific)
* Schema introspection methods (``get_tables()``, ``get_table_info()``, etc.)

Code Examples
-------------

Basic Connection
~~~~~~~~~~~~~~~~

.. code-block:: python

   # Works the same in both libraries
   import rapsqlite as aiosqlite

   async with aiosqlite.connect("example.db") as db:
       await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
       await db.execute("INSERT INTO test (value) VALUES ('hello')")
       rows = await db.fetch_all("SELECT * FROM test")
       print(rows)  # [[1, 'hello']]

Parameterized Queries
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

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

Transactions
~~~~~~~~~~~~

.. code-block:: python

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

Differences and Limitations
---------------------------

For a detailed compatibility analysis based on running the aiosqlite test suite, see :doc:`compatibility`.

Known Differences
~~~~~~~~~~~~~~~~~

1. **Connection Properties**: ``total_changes`` and ``in_transaction`` are async methods (not properties) in rapsqlite, but functionally equivalent:

   .. code-block:: python

      # aiosqlite
      changes = db.total_changes
      in_tx = db.in_transaction

      # rapsqlite
      changes = await db.total_changes()
      in_tx = await db.in_transaction()

2. **``iterdump()`` Return Type**: rapsqlite supports both async iteration and await-to-list:

   .. code-block:: python

      # aiosqlite and rapsqlite (async iterator)
      async for line in db.iterdump():
          ...

      # rapsqlite (backwards compatible)
      lines = await db.iterdump()  # Returns List[str]

3. **``init_hook`` parameter**: This is a rapsqlite-specific enhancement for automatic database initialization. It's not available in aiosqlite.

Advanced Features
-----------------

Database Initialization Hooks
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

rapsqlite supports automatic database initialization:

.. code-block:: python

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

Schema Introspection
~~~~~~~~~~~~~~~~~~~~

rapsqlite provides comprehensive schema introspection:

.. code-block:: python

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

Troubleshooting
---------------

Import Errors
~~~~~~~~~~~~~

**Problem**: ``ImportError: Could not import _rapsqlite``

**Solution**: Make sure rapsqlite is built. If installing from source:

.. code-block:: bash

   pip install maturin
   maturin develop

Performance Issues
~~~~~~~~~~~~~~~~~~

**Problem**: Queries seem slower than expected

**Solution**:

* Ensure you're using parameterized queries (not string formatting)
* Use connection pooling for concurrent operations
* Consider using ``execute_many()`` for batch inserts

Transaction Issues
~~~~~~~~~~~~~~~~~~

**Problem**: "Transaction connection not available" error

**Solution**: Make sure you call ``begin()`` before using transaction methods, or use the ``transaction()`` context manager.

Testing Your Migration
------------------------

1. **Run existing tests**: Your aiosqlite tests should work with minimal changes
2. **Use compatibility tests**: See ``tests/test_dropin_replacement.py`` for examples
3. **Verify performance**: Use benchmarks to ensure performance meets expectations

Example: Complete Migration
---------------------------

Here's a complete example of migrating an application:

**Before (aiosqlite):**

.. code-block:: python

   import aiosqlite

   async def main():
       async with aiosqlite.connect("app.db") as db:
           await db.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)")
           await db.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           async with db.execute("SELECT * FROM users") as cursor:
               rows = await cursor.fetchall()
               print(rows)

   asyncio.run(main())

**After (rapsqlite):**

.. code-block:: python

   import rapsqlite as aiosqlite

   async def main():
       async with aiosqlite.connect("app.db") as db:
           await db.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)")
           await db.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           # Option 1: Use async with pattern (same as aiosqlite)
           async with db.execute("SELECT * FROM users") as cursor:
               rows = await cursor.fetchall()
               print(rows)
           # Option 2: Use fetch_all (rapsqlite enhancement)
           rows = await db.fetch_all("SELECT * FROM users")
           print(rows)

   asyncio.run(main())

The migration is complete! Your code now uses true async SQLite operations.
