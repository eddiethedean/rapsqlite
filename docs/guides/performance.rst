Performance Guide
=================

This guide covers performance tuning, optimization strategies, and best practices for getting the best performance from rapsqlite.

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

Single connection vs larger pool
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* **Single connection (pool_size=1, default)**: Use when you have one logical worker (e.g. one async task doing DB work at a time), or when concurrency is low. Session-connection reuse means one connection handles many sequential queries efficiently.
* **Larger pool (2–5 or more)**: Use when many concurrent coroutines perform database operations at the same time (e.g. many concurrent HTTP requests each hitting the DB). Increase ``pool_size`` before the first operation; it cannot be changed after the pool is created.

Prepared Statement Caching
---------------------------

``rapsqlite`` automatically benefits from prepared statement caching provided by sqlx (the underlying database library). sqlx caches prepared statements per connection, meaning:

* **Automatic caching**: Prepared statements are cached automatically - no configuration needed
* **Per-connection cache**: Each connection in the pool maintains its own cache
* **Query normalization**: rapsqlite normalizes queries (removes extra whitespace) to maximize cache hits
* **Performance benefit**: Repeated queries with the same structure reuse prepared statements

To maximize cache hits:

.. code-block:: python

   # Good: Same query structure, different parameters
   for user_id in user_ids:
       await conn.fetch_all("SELECT * FROM users WHERE id = ?", [user_id])
   # sqlx caches the prepared statement after first execution

   # Also good: Query normalization handles whitespace differences
   await conn.execute("INSERT INTO test (value) VALUES (?)", ["a"])
   await conn.execute("INSERT  INTO  test  (value)  VALUES  (?)", ["b"])
   # Both queries are normalized and benefit from the same prepared statement

Best practices:

* Use consistent query formatting (normalization happens automatically)
* Reuse the same query strings with different parameters
* Keep connections alive for repeated queries (connection pooling helps)
* Use parameterized queries (required for prepared statements)
* Avoid dynamic query building when possible (reduces cache hits)

Performance impact:

Prepared statement caching provides significant performance benefits for repeated queries:

* First execution: Statement is prepared and cached
* Subsequent executions: Reuses cached prepared statement (much faster)
* Typical improvement: 2-5x faster for repeated queries vs. unique queries

PRAGMA Optimization
-------------------

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

Batch Operations
----------------

Use ``execute_many()`` for batch inserts:

.. code-block:: python

   # Good: Single transaction, prepared statement reuse
   params = [[f"user_{i}"] for i in range(1000)]
   await conn.execute_many("INSERT INTO users (name) VALUES (?)", params)

   # Bad: Many individual transactions
   for i in range(1000):
       await conn.execute("INSERT INTO users (name) VALUES (?)", [f"user_{i}"])

Connection Reuse
----------------

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

Use Appropriate Fetch Methods
------------------------------

* ``fetch_all()``: When you need all rows
* ``fetch_one()``: When you expect exactly one row
* ``fetch_optional()``: When you might have zero or one row
* ``Cursor.fetchmany()``: When processing large result sets in chunks

Low-latency scalar and BLOB lookups
-----------------------------------

Cache-style code that needs one value should use ``fetch_scalar()`` instead of
``fetch_one()``. The scalar path requires exactly one result column and avoids
building a general row or applying a row factory. ``fetch_blob()`` adds a
runtime contract that the result is a BLOB or ``NULL``.

.. code-block:: python

   value = await conn.fetch_scalar(
       "SELECT value FROM cache WHERE key = ? AND expires_at > ?",
       [key, time.time()],
   )
   payload = await conn.fetch_blob(
       "SELECT value FROM cache WHERE key = ? AND expires_at > ?",
       [key, time.time()],
   )

For a deliberately narrow, opt-in fast path, ``raw_fetch_scalar()`` executes
one prepared SQLite C statement and returns a scalar without SQLx row decoding.
It supports SQLite scalar values and BLOBs, but is unavailable when callbacks
are configured and does not apply row factories, text factories, or converters.
It is intended for trusted, parameterized cache lookups, not as a replacement
for the general query API. A raw operation registers its active SQLite handle
so ``await conn.interrupt()`` can stop a long-running raw statement; ordinary
task cancellation does not magically interrupt synchronous SQLite execution.

.. code-block:: python

   value = await conn.raw_fetch_scalar(
       "SELECT value FROM cache WHERE key = ?", [key], blob=True
   )

``prepare()`` creates a reusable connection-bound operation object that keeps
the SQL and mode selection out of a hot loop. Normal prepared operations still
use SQLx's per-connection statement cache; raw operations retain the narrow
raw contract and re-prepare through the current physical handle.

.. code-block:: python

   lookup = conn.prepare(
       "SELECT value FROM cache WHERE key = ?", raw=True, blob=True
   )
   value = await lookup.fetch_blob([key])

Session affinity
-----------------

``connect(..., session_affinity=True)`` retains one physical SQLite connection
between non-transaction operations on that logical connection. This can reduce
pool acquisition and handle-lock churn for sequential cache loops, but the
retained connection consumes pool capacity and reduces sharing under
concurrency. It is opt-in and should be benchmarked with the intended pool
size. Transactions always take priority and release the retained session while
they are active.

Diagnostics and benchmark discipline
------------------------------------

Query-usage analytics are disabled by default. Set
``conn.query_usage_tracking = True`` only while diagnosing workload shape; the
normal path then avoids SQL normalization and the analytics mutex. This
analytics counter is separate from SQLx's internal prepared-statement cache.

Use ``benchmarks/phase5_hotpath.py`` to compare the actual client APIs. Report
sequential latency (including p50/p95/p99), concurrent throughput, event-loop
delay, pool/session settings, payload size, expiration behavior, and errors
separately. Do not infer per-request latency by dividing a published server
throughput number by operations per second, and do not compare unmatched raw
SQLite timings with async client calls.

Performance Monitoring
----------------------

Query Timing
~~~~~~~~~~~~

.. code-block:: python

   import time

   start = time.perf_counter()
   rows = await conn.fetch_all("SELECT * FROM users")
   elapsed = time.perf_counter() - start
   print(f"Query took {elapsed * 1000:.2f}ms")

Connection Pool Metrics
~~~~~~~~~~~~~~~~~~~~~~~

Monitor connection pool usage by tracking:

* Number of concurrent operations
* Connection acquisition timeouts
* Pool size vs. actual usage

Troubleshooting Performance Issues
----------------------------------

Slow Queries
~~~~~~~~~~~~

1. **Check if using parameterized queries**: String formatting prevents prepared statement caching
2. **Verify indexes**: Use ``get_indexes()`` to check table indexes
3. **Analyze query plans**: Use ``EXPLAIN QUERY PLAN`` to understand query execution
4. **Check PRAGMA settings**: Ensure appropriate settings for your workload

High Memory Usage
~~~~~~~~~~~~~~~~~

1. **Reduce pool size**: Smaller pool uses less memory
2. **Use fetchmany()**: Process large result sets in chunks
3. **Clear query cache**: Connection close clears prepared statement cache

Database Locked Errors
~~~~~~~~~~~~~~~~~~~~~~

1. **Increase timeout**: Set ``connection_timeout`` higher
2. **Use WAL mode**: ``journal_mode = "WAL"`` allows concurrent reads
3. **Reduce transaction duration**: Keep transactions short
4. **Implement retry logic**: See error handling section in :doc:`advanced-usage`

Measuring performance and regression testing
---------------------------------------------

* **Query timing**: Use ``timed_fetch_all(conn, sql, parameters, on_timing=...)`` from ``rapsqlite`` to record query duration, or wrap calls with ``time.perf_counter()``. See :doc:`advanced-usage` for monitoring and trace callbacks.
* **Regression tests**: The test suite includes performance regression tests in ``tests/test_performance.py``. Run with ``pytest tests/ -m performance`` to validate performance characteristics after changes.

Further Reading
---------------

* `SQLite Performance Tuning <https://www.sqlite.org/performance.html>`_
* `SQLite PRAGMA Documentation <https://www.sqlite.org/pragma.html>`_
* :doc:`advanced-usage` - Advanced usage patterns and best practices
* :doc:`migration-guide` - Migrating from aiosqlite (performance impact of row format, pool, and PRAGMAs)
