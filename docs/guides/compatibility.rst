aiosqlite Compatibility Analysis
================================

This document analyzes compatibility between rapsqlite and aiosqlite based on running the aiosqlite test suite.

Test Execution
--------------

**Date**: 2026-01-26 (Updated)  
**rapsqlite Version**: 0.2.0  
**aiosqlite Test Suite**: Latest from https://github.com/omnilib/aiosqlite

**Last Updated**: 2026-01-26 - Major compatibility improvements implemented

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

For more details, see :doc:`migration-guide`.
