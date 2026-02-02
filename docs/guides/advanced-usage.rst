Advanced Usage Guide
====================

This guide covers advanced usage patterns, best practices, performance tuning, and common anti-patterns for ``rapsqlite``.

Table of Contents
-----------------

* :ref:`connection-pooling`
* :ref:`monitoring`
* :ref:`resource-cleanup`
* :ref:`thread-safety`
* :ref:`transaction-patterns`
* :ref:`error-handling-strategies`
* :ref:`performance-tuning`
* :ref:`common-anti-patterns`
* :ref:`best-practices`

.. _connection-pooling:

Connection Pooling
------------------

Understanding Connection Pools
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``rapsqlite`` uses connection pooling internally. The default pool size is 1, but you can configure it:

.. code-block:: python

   from rapsqlite import Connection

   # Create connection with custom pool size
   conn = Connection("example.db")
   conn.pool_size = 5  # Allow up to 5 concurrent connections
   conn.connection_timeout = 30  # 30 second timeout for acquiring connections

When to Increase Pool Size
~~~~~~~~~~~~~~~~~~~~~~~~~~

* **Concurrent operations**: If you have many concurrent database operations, increase pool size
* **Long-running queries**: If queries take a long time, more connections allow better concurrency
* **Mixed read/write workloads**: Separate connections for reads and writes

Pool Size Guidelines
~~~~~~~~~~~~~~~~~~~~

* **Default (1)**: Good for single-threaded async applications or low concurrency
* **Small (2-5)**: Good for moderate concurrency, most web applications
* **Large (10+)**: Only for high-concurrency scenarios, be mindful of SQLite's write serialization

**Note**: SQLite serializes writes, so increasing pool size mainly helps with concurrent reads.

Pool size is fixed at first use
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The connection pool is created lazily when the first database operation runs. Once created,
the pool size cannot be changed. To use a different pool size, set ``conn.pool_size = N``
**before** any database operation (e.g. before ``execute``, ``fetch_all``, or entering a
transaction). If you need to resize after the pool exists, create a new connection with
the desired ``pool_size`` and close the old one.

Idle connection timeout
~~~~~~~~~~~~~~~~~~~~~~~

Set ``idle_timeout`` (seconds) so connections idle in the pool longer than that are closed.
Use ``connect(..., idle_timeout=60)`` or ``conn.idle_timeout = 60`` before the pool is created.
``None`` (default) means no idle timeout. This helps reduce resource use when load drops.

.. _monitoring:

Monitoring
---------

Pool metrics
~~~~~~~~~~~

``Connection.pool_metrics()`` returns a dict with pool usage: ``size`` (total connections), ``num_idle`` (idle), and ``in_use`` (active). Use it to observe pool health in production:

.. code-block:: python

   async with connect("app.db") as conn:
       metrics = await conn.pool_metrics()
       # e.g. {"size": 5, "num_idle": 3, "in_use": 2}
       logger.info("pool %s", metrics)
       # Or expose via a /metrics endpoint for Prometheus, etc.

Metrics export (Prometheus / custom)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use the optional helper **``pool_metrics_gauges(conn)``** to get pool metrics as a dict of
gauge names suitable for Prometheus or a custom metrics endpoint. It returns
``rapsqlite_pool_size``, ``rapsqlite_pool_num_idle``, and ``rapsqlite_pool_in_use``.
Import it from ``rapsqlite`` and call it with your connection:

.. code-block:: python

   from rapsqlite import connect, pool_metrics_gauges

   async with connect("app.db") as conn:
       gauges = await pool_metrics_gauges(conn)
       # e.g. {"rapsqlite_pool_size": 5, "rapsqlite_pool_num_idle": 3, "rapsqlite_pool_in_use": 2}
       # Expose gauges on your /metrics endpoint or feed into your metrics system.

See :ref:`api-connection` for ``pool_metrics()`` and ``pool_health()``.

Health checks
~~~~~~~~~~~~~

Use ``pool_health()`` for liveness/readiness probes: it runs ``SELECT 1`` and returns ``True`` on success, or raises on failure.

.. code-block:: python

   try:
       ok = await conn.pool_health()
       assert ok
   except Exception:
       # Database or pool unavailable
       pass

Connection health and recovery
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The underlying pool (sqlx) acquires connections on demand; when a connection is returned to the pool after use, it remains available for reuse. If a connection fails (e.g. database closed or I/O error), the pool can replace it on the next acquire. Use **``pool_health()``** periodically (e.g. in a liveness probe) to detect when the database is unavailable; combine with **``pool_metrics()``** to observe pool usage. For transient errors (e.g. ``SQLITE_BUSY``), retry the operation or use a transaction retry pattern (see :ref:`transaction-patterns`).

Query logging and slow-query detection
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use ``set_trace_callback`` to log every SQL statement executed on the connection. For slow-query detection, record the time before and after the query in your callback (the callback is invoked before the statement runs; you can pair it with a wrapper that measures duration around execute calls, or log and correlate with application metrics).

.. code-block:: python

   import time
   import logging
   logger = logging.getLogger("rapsqlite.queries")

   def query_logger(sql: str):
       logger.info("SQL: %s", sql.strip())

   await conn.set_trace_callback(query_logger)
   # All subsequent executes on this connection will log SQL
   # Set to None to disable: await conn.set_trace_callback(None)

For slow-query detection, measure elapsed time around your own execute calls (e.g. with a small helper or middleware) and log when a threshold is exceeded; the trace callback alone does not provide timing. This gives a clear path to observe queries without implementing a full metrics pipeline.

Slow query threshold (Phase 3.5 — implemented)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use **``Connection.set_slow_query_threshold(threshold_secs, callback=None)``** to automatically detect and report slow queries. When ``fetch_all()`` takes longer than ``threshold_secs``, the optional callback is invoked with ``callback(duration_secs, sql)``. Set ``threshold_secs`` to 0 to disable.

.. code-block:: python

   import logging
   from rapsqlite import connect

   logger = logging.getLogger("app.queries")

   async with connect("app.db") as conn:
       conn.set_slow_query_threshold(1.0, lambda d, s: logger.warning("Slow query (%.2fs): %s", d, s[:100]))
       await conn.execute("CREATE TABLE t (id INT)")
       rows = await conn.fetch_all("SELECT * FROM t")

Query timing (duration and callbacks)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

To record per-query duration (e.g. for metrics or slow-query logging), use **``timed_fetch_all(conn, sql, parameters=None, on_timing=None)``**. It runs ``fetch_all``, measures elapsed time, and optionally calls ``on_timing(duration_secs, sql)``. Import from ``rapsqlite``:

.. code-block:: python

   import logging
   from rapsqlite import connect, timed_fetch_all

   logger = logging.getLogger("app.queries")

   def on_timing(duration_secs: float, sql: str) -> None:
       if duration_secs > 1.0:
           logger.warning("Slow query (%.2fs): %s", duration_secs, sql.strip()[:200])

   async with connect("app.db") as conn:
       rows = await timed_fetch_all(conn, "SELECT * FROM big_table", on_timing=on_timing)

   # Or omit on_timing to get (rows, duration_secs)
   rows, duration = await timed_fetch_all(conn, "SELECT 1")
   logger.info("Query took %.3fs", duration)

.. _resource-cleanup:

Resource Cleanup and Connection Lifetime
-----------------------------------------

Always close connections under async control (Option A): use ``async with connect(...) as conn:`` or explicitly ``await conn.close()``. Do not rely on garbage collection to close connections.

If a ``Connection`` is dropped without calling ``close()`` (e.g. you keep no reference and never use ``async with``), Python's garbage collector may eventually drop the Rust connection object. Pool cleanup then runs outside the async runtime and can fail. To avoid this:

* **Use ``async with``**: Preferred. The context manager calls ``close()`` when exiting, so the pool is closed under the async runtime.
* **Call ``close()`` explicitly**: If you cannot use a context manager, call ``await conn.close()`` when done.
* **Best-effort ``__del__``**: The Python wrapper schedules ``close()`` on the running event loop when the connection is GC'd, if a loop exists. This is best-effort only (no guarantees about finalizer order or loop lifetime) and does not replace ``async with`` or explicit ``close()``.

.. _thread-safety:

Thread safety
~~~~~~~~~~~~~

``rapsqlite`` is designed for async use; connections are not thread-safe. Do not share a single ``Connection`` instance across threads. Use one connection per asyncio task, or rely on the internal pool (one logical connection, multiple pooled connections used by your async code). For concurrent access from multiple threads, use a separate ``Connection`` per thread or a thread-safe wrapper that hands out connections from a pool per request.

.. _transaction-patterns:

Transaction Patterns
--------------------

Explicit Transactions
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   async with connect("example.db") as conn:
       await conn.begin()
       try:
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
           await conn.commit()
       except Exception:
           await conn.rollback()
           raise

Transaction Context Manager (Recommended)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   async with connect("example.db") as conn:
       async with conn.transaction():
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
           # Automatically commits on success, rolls back on exception

Nested Transactions
~~~~~~~~~~~~~~~~~~~

SQLite doesn't support true nested transactions, but you can use savepoints:

.. code-block:: python

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

You can also use the ``savepoint()`` context manager for the same pattern:

.. code-block:: python

   async with connect("example.db") as conn:
       await conn.begin()
       try:
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           async with conn.savepoint("sp1"):
               await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
           await conn.commit()
       except Exception:
           await conn.rollback()

Use ``async with conn.savepoint():`` without a name to auto-generate one. Savepoints require an active transaction (``begin()`` or ``transaction()``).

Transaction retry (transient errors)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

For workloads that may hit ``SQLITE_BUSY`` or ``SQLITE_LOCKED``, use **``transaction_retry(conn, work, max_retries=5, ...)``**. It runs a transaction with exponential backoff on transient errors. ``work`` must be a callable that returns an awaitable (e.g. an async function); it is invoked once per attempt so each retry runs fresh.

.. code-block:: python

   from rapsqlite import connect, transaction_retry

   async with connect("app.db") as conn:
       async def do_work():
           await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
       await transaction_retry(conn, do_work, max_retries=3)

Transaction timeout
~~~~~~~~~~~~~~~~~~~

Use **``transaction_with_timeout(conn, work, timeout_secs=30)``** to run a transaction
with a maximum duration. Raises ``asyncio.TimeoutError`` if the transaction exceeds
the timeout. Use this to prevent long-running transactions from blocking other work.

.. code-block:: python

   from rapsqlite import connect, transaction_with_timeout

   async with connect("app.db") as conn:
       async def do_work():
           await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
       await transaction_with_timeout(conn, do_work, timeout_secs=5)

.. _error-handling-strategies:

Error Handling Strategies
-------------------------

Handling Specific Errors
~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

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

Retry Logic for Locked Database
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

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

Error Context
~~~~~~~~~~~~~

Always include context in error messages:

.. code-block:: python

   try:
       await conn.execute("INSERT INTO users (name) VALUES (?)", [name])
   except IntegrityError as e:
       raise IntegrityError(f"Failed to insert user '{name}': {e}") from e

Callback Exception Handling
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

When using SQLite callbacks (user-defined functions, trace callbacks, authorizer, progress handler),
exceptions in your Python callbacks are handled automatically:

**User-Defined Functions:**
- Exceptions are converted to SQLite errors
- The query will fail with an ``OperationalError`` containing the Python exception message
- Example: If your function raises ``ValueError("Invalid input")``, the query fails with ``Python function error: ValueError: Invalid input``

**Trace Callbacks:**
- Exceptions are silently ignored to prevent trace callback failures from affecting database operations
- Your trace callback should handle exceptions internally if you need error handling
- Example: Wrap your callback logic in try/except if you need to log errors

**Authorizer Callbacks:**
- Exceptions default to **DENY** (fail-secure behavior) for security
- If your authorizer callback raises an exception, the operation is denied
- This prevents authorization bypass if the callback crashes
- Example: Always handle exceptions in authorizer callbacks to avoid denying legitimate operations

**Progress Handlers:**
- Exceptions default to **continue** (don't abort the operation)
- Progress callback failures won't abort long-running operations
- Example: Handle exceptions internally if you need to track progress errors

Best Practice: Always handle exceptions within your callback functions:

.. code-block:: python

   def safe_user_function(x):
       try:
           # Your logic here
           return process_value(x)
       except Exception as e:
           # Log error and return a safe default or re-raise
           logger.error(f"Error in user function: {e}")
           return None  # Or raise if you want query to fail

   await conn.create_function("safe_func", 1, safe_user_function)

Streaming and large result sets
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

To process large SELECT result sets without loading every row into memory:

- **Cursor iteration**: ``async for row in cursor`` after ``await cursor.execute(...)``
  iterates over rows, but the cursor currently loads the full result set when
  ``execute`` runs. Use this when the result size is acceptable in memory.

- **Chunked fetching**: Use ``cursor.fetchmany(size)`` in a loop after
  ``execute``; each call returns up to ``size`` rows from the already-fetched
  result set. For true streaming (fetch from the database in chunks), use
  pagination below.

- **Streaming iterator (memory-efficient)**: Use ``execute_iter(conn, sql, parameters=None, chunk_size=None)`` or ``conn.execute_iter(sql, ...)`` to get an async iterator that yields **chunks** of rows. Uses ``LIMIT``/``OFFSET`` under the hood so memory stays bounded by ``chunk_size`` (default: ``conn.iter_chunk_size``, e.g. 64). One connection is used for the duration of iteration; closing the connection or cancelling the task stops iteration.

  .. code-block:: python

     from rapsqlite import connect, execute_iter

     async with connect("app.db") as conn:
         async for chunk in conn.execute_iter("SELECT * FROM big_table ORDER BY id", chunk_size=1000):
             for row in chunk:
                 process(row)

  Ensure the query has a deterministic order (e.g. ``ORDER BY id``) so pages are consistent.

- **Page-based pagination**: Use ``paginate(conn, sql, parameters=None, page_size=64, offset=0)`` to fetch one page at a time. Returns a list of rows for that page.

  .. code-block:: python

     from rapsqlite import connect, paginate

     async with connect("app.db") as conn:
         offset = 0
         while True:
             rows = await paginate(conn, "SELECT * FROM big_table ORDER BY id", page_size=100, offset=offset)
             if not rows:
                 break
             for row in rows:
                 process(row)
             offset += len(rows)

- **Manual pagination**: You can still run ``fetch_all`` with ``LIMIT``/``OFFSET`` in a loop if you prefer; ``execute_iter`` and ``paginate`` do this for you.

- **Rows to dicts**: Use ``rows_to_dicts(rows, columns)`` to convert list-of-list rows to list-of-dicts using column names from ``cursor.description``:

  .. code-block:: python

     from rapsqlite import connect, rows_to_dicts

     async with connect("app.db") as conn:
         rows = await conn.fetch_all("SELECT id, name FROM users")
         dicts = rows_to_dicts(rows, ["id", "name"])
         # [{"id": 1, "name": "Alice"}, ...]

Query plan analysis
~~~~~~~~~~~~~~~~~~~

Use ``analyze_query_plan(conn, sql, parameters=None)`` to inspect how SQLite will execute a query. Returns a dict with ``rows`` (raw EXPLAIN QUERY PLAN output), ``details`` (list of detail strings), ``uses_index`` (True if index is used), and ``table_scan`` (True if full table scan):

  .. code-block:: python

     from rapsqlite import connect, analyze_query_plan

     async with connect("app.db") as conn:
         analysis = await analyze_query_plan(conn, "SELECT * FROM users WHERE id = ?", [1])
         if analysis["table_scan"] and not analysis["uses_index"]:
             print("Consider adding an index on users(id)")

Use **``suggest_indexes(conn, sql, parameters=None)``** to get index suggestions when the plan shows a full table scan. Returns a list of dicts with ``table``, ``column`` (may be empty), and ``suggestion`` (CREATE INDEX template):

  .. code-block:: python

     from rapsqlite import connect, suggest_indexes

     async with connect("app.db") as conn:
         suggestions = await suggest_indexes(conn, "SELECT * FROM users WHERE email = ?", ["x"])
         for s in suggestions:
             print(s["suggestion"])  # e.g. CREATE INDEX idx_users_<columns> ON users(<columns>) ...

FTS, JSON, and UPSERT
~~~~~~~~~~~~~~~~~~~~~

rapsqlite uses standard SQLite; you can use FTS, JSON, and UPSERT directly:

- **Full-Text Search (FTS)**: Create a virtual table with ``CREATE VIRTUAL TABLE
  ... USING fts5(...)`` and query with ``MATCH``. No extra setup required.

- **JSON**: Use SQLite's JSON1 functions (e.g. ``json_extract``, ``json_object``,
  ``->``, ``->>``) in your SQL. Results are returned as text; parse in Python
  if needed.

- **UPSERT**: Use ``INSERT ... ON CONFLICT (...) DO UPDATE SET ...`` or ``DO
  NOTHING``. Works with rapsqlite like any other DML.

Connection Lifecycle and Cleanup
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Always use context managers** for proper cleanup:

.. code-block:: python

   # Good: Automatic cleanup
   async with connect("example.db") as conn:
       await conn.execute("SELECT 1")
   # Connection automatically closed, transactions rolled back

   # Bad: Manual cleanup required
   conn = connect("example.db")
   try:
       await conn.execute("SELECT 1")
   finally:
       await conn.close()  # Must remember to close

**Transaction cleanup:**
- Active transactions are automatically rolled back when connection is closed
- Use transaction context managers for automatic commit/rollback
- Example: ``async with conn.transaction():`` ensures cleanup even on exceptions

Pool Exhaustion Troubleshooting
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

If you see "Failed to acquire connection from pool" errors:

1. **Increase pool_size**: More connections allow more concurrent operations
2. **Increase connection_timeout**: Give more time for connections to become available
3. **Check for long-running transactions**: Transactions hold connections until commit/rollback
4. **Ensure proper cleanup**: Use context managers to release connections promptly

Error messages include current pool configuration and suggestions:

.. code-block:: python

   try:
       await conn.execute("SELECT 1")
   except OperationalError as e:
       # Error message includes:
       # - Current pool_size
       # - Current connection_timeout
       # - Suggestions for resolution
       print(e)

.. _performance-tuning:

Performance Tuning
------------------

For a full performance tuning guide (pool sizing, prepared statements, PRAGMAs, benchmarks), see :doc:`performance`.

Use Parameterized Queries
~~~~~~~~~~~~~~~~~~~~~~~~~~

**Good:**

.. code-block:: python

   await conn.execute("INSERT INTO users (name) VALUES (?)", [name])

**Bad:**

.. code-block:: python

   await conn.execute(f"INSERT INTO users (name) VALUES ('{name}')")  # SQL injection risk, no caching

IN clause expansion
~~~~~~~~~~~~~~~~~~~

Use **``in_clause_query(sql, values)``** when you have ``WHERE id IN (?)`` and a list of IDs. It returns ``(processed_sql, params)`` to pass to ``fetch_all``:

.. code-block:: python

   from rapsqlite import connect, in_clause_query

   ids = [1, 2, 3]
   sql, params = in_clause_query("SELECT * FROM users WHERE id IN (?)", ids)
   rows = await conn.fetch_all(sql, params)
   # Equivalent to: SELECT * FROM users WHERE id IN (?, ?, ?) with params [1, 2, 3]

Alternatively, build the placeholders manually: ``", ".join("?" * len(ids))``.

Batch Operations
~~~~~~~~~~~~~~~~

Use ``execute_many()`` for batch inserts:

.. code-block:: python

   # Good: Single transaction, prepared statement reuse
   params = [[f"user_{i}"] for i in range(1000)]
   await conn.execute_many("INSERT INTO users (name) VALUES (?)", params)

   # Bad: Many individual transactions
   for i in range(1000):
       await conn.execute("INSERT INTO users (name) VALUES (?)", [f"user_{i}"])

Connection Reuse
~~~~~~~~~~~~~~~~

Reuse connections when possible:

.. code-block:: python

   # Good: Reuse connection
   async with connect("example.db") as conn:
       for i in range(100):
           await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

   # Bad: Create new connection for each operation
   for i in range(100):
       async with connect("example.db") as conn:
           await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

PRAGMA Optimization
~~~~~~~~~~~~~~~~~~~~

Configure SQLite for your workload:

.. code-block:: python

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

Prepared Statement Caching
~~~~~~~~~~~~~~~~~~~~~~~~~~

``rapsqlite`` automatically benefits from prepared statement caching provided by sqlx (the underlying database library). sqlx caches prepared statements per connection, meaning:

* **Automatic caching**: Prepared statements are cached automatically - no configuration needed
* **Per-connection cache**: Each connection in the pool maintains its own cache
* **Query normalization**: rapsqlite normalizes queries (removes extra whitespace) to maximize cache hits
* **Performance benefit**: Repeated queries with the same structure reuse prepared statements

**To maximize cache hits:**

.. code-block:: python

   # Good: Same query structure, different parameters
   for user_id in user_ids:
       await conn.fetch_all("SELECT * FROM users WHERE id = ?", [user_id])
   # sqlx caches the prepared statement after first execution

   # Also good: Query normalization handles whitespace differences
   await conn.execute("INSERT INTO test (value) VALUES (?)", ["a"])
   await conn.execute("INSERT  INTO  test  (value)  VALUES  (?)", ["b"])
   # Both queries are normalized and benefit from the same prepared statement

**Best practices:**

* Use consistent query formatting (normalization happens automatically)
* Reuse the same query strings with different parameters
* Keep connections alive for repeated queries (connection pooling helps)
* Use parameterized queries (required for prepared statements)
* Avoid dynamic query building when possible (reduces cache hits)

**Performance impact:**

Prepared statement caching provides significant performance benefits for repeated queries:

* First execution: Statement is prepared and cached
* Subsequent executions: Reuses cached prepared statement (much faster)
* Typical improvement: 2-5x faster for repeated queries vs. unique queries

.. _common-anti-patterns:

Common Anti-Patterns
--------------------

❌ Not Using Transactions for Multiple Operations
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Each operation is a separate transaction
   await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")
   await conn.execute("INSERT INTO accounts (balance) VALUES (2000)")
   # If second fails, first is already committed

.. code-block:: python

   # Good: Use transaction
   async with conn.transaction():
       await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")
       await conn.execute("INSERT INTO accounts (balance) VALUES (2000)")

❌ String Formatting Instead of Parameters
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: SQL injection risk, no prepared statement caching
   await conn.execute(f"SELECT * FROM users WHERE name = '{name}'")

.. code-block:: python

   # Good: Parameterized query
   await conn.execute("SELECT * FROM users WHERE name = ?", [name])

❌ Fetching All Rows When Only One Needed
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Fetches all rows, then takes first
   rows = await conn.fetch_all("SELECT * FROM users WHERE id = ?", [user_id])
   user = rows[0] if rows else None

.. code-block:: python

   # Good: Fetch only what you need
   user = await conn.fetch_optional("SELECT * FROM users WHERE id = ?", [user_id])

❌ Not Handling Errors in Transactions
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Exception leaves transaction open
   async with conn.transaction():
       await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
       await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
       # If exception occurs, transaction might not rollback properly

.. code-block:: python

   # Good: Transaction context manager handles rollback automatically
   async with conn.transaction():
       await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
       await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
       # Automatically rolls back on exception

❌ Creating Too Many Connections
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Creates new connection for each operation
   async def get_user(user_id):
       async with connect("example.db") as conn:
           return await conn.fetch_one("SELECT * FROM users WHERE id = ?", [user_id])

.. code-block:: python

   # Good: Reuse connection or use connection pool
   async with connect("example.db") as conn:
       user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [user_id])

❌ Abandoning Connections Without Closing
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Connection left open; GC cleanup is best-effort and can cause issues
   conn = await connect("example.db").__await__()
   await conn.execute("INSERT INTO test VALUES (1)")
   # conn never closed

.. code-block:: python

   # Good: Always use async with or explicitly close
   async with connect("example.db") as conn:
       await conn.execute("INSERT INTO test VALUES (1)")
   # conn closed on exit

❌ Blocking the Event Loop
~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Bad: Blocking call inside async code stalls the event loop
   async def bad():
       async with connect("example.db") as conn:
           time.sleep(1)  # Blocks entire event loop
           await conn.fetch_all("SELECT * FROM test")

.. code-block:: python

   # Good: Use async sleep and keep I/O in rapsqlite (already non-blocking)
   async def good():
       await asyncio.sleep(1)
       async with connect("example.db") as conn:
           await conn.fetch_all("SELECT * FROM test")

.. _best-practices:

Best Practices
--------------

1. Always Use Context Managers for Connections
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use ``async with connect(...)`` or ``async with Connection(...)`` so connections are always closed. Avoid holding a connection reference without ensuring ``close()`` is called (e.g. on error paths or when storing in globals).

.. code-block:: python

   # Good
   async with connect("example.db") as conn:
       await conn.execute("CREATE TABLE test (id INTEGER)")

2. Use Transactions for Related Operations and Keep Them Short
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Wrap related reads/writes in a single transaction. Keep transaction boundaries tight so locks are held briefly; avoid long-running work (e.g. network calls) inside a transaction.

.. code-block:: python

   async with conn.transaction():
       await conn.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
       await conn.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2")

3. Handle Errors Appropriately
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   try:
       await conn.execute("INSERT INTO users (email) VALUES (?)", [email])
   except IntegrityError:
       # Handle duplicate email
       pass

4. Use Appropriate Fetch Methods
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* ``fetch_all()``: When you need all rows
* ``fetch_one()``: When you expect exactly one row
* ``fetch_optional()``: When you might have zero or one row
* ``Cursor.fetchmany()``: When processing large result sets in chunks

5. Configure PRAGMAs for Your Workload
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Read-heavy
   pragmas = {"journal_mode": "WAL", "synchronous": "NORMAL"}

   # Write-heavy
   pragmas = {"journal_mode": "WAL", "synchronous": "FULL"}

   # Development
   pragmas = {"journal_mode": "MEMORY", "synchronous": "OFF"}

6. Use Database Initialization Hooks
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

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

7. Monitor Connection Pool Usage
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Adjust pool size based on your workload
   conn = Connection("example.db")
   conn.pool_size = 5  # For moderate concurrency
   conn.connection_timeout = 30  # 30 second timeout

8. Use Schema Introspection
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Check if table exists before creating
   tables = await conn.get_tables()
   if "users" not in tables:
       await conn.execute("CREATE TABLE users ...")

   # Get table structure
   columns = await conn.get_table_info("users")

9. Prefer Parameterized Queries and Choose the Right Fetch Pattern
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* **Parameterized queries**: Always use ``?`` placeholders and pass parameters as a list/tuple; never string-format SQL (SQL injection risk, no prepared-statement caching).
* **Large result sets**: Use ``execute_iter`` for streaming (memory-efficient, async iterator) or ``paginate`` for page-based access (manual offset). Use ``fetch_all`` only when the result set fits in memory.
* **Pool sizing**: Set ``pool_size`` before first use; default 1 is fine for most apps; increase for concurrent reads.

Further Reading
---------------

* :doc:`performance` - Performance tuning guide
* :doc:`migration-guide` - Migrating from aiosqlite
* `SQLite Performance Tuning <https://www.sqlite.org/performance.html>`_
* `SQLite PRAGMA Documentation <https://www.sqlite.org/pragma.html>`_
