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

Further Reading
---------------

* `SQLite Performance Tuning <https://www.sqlite.org/performance.html>`_
* `SQLite PRAGMA Documentation <https://www.sqlite.org/pragma.html>`_
* :doc:`advanced-usage` - Advanced usage patterns and best practices
