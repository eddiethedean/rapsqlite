Advanced Usage Guide
====================

This guide covers advanced usage patterns, best practices, performance tuning, and common anti-patterns for ``rapsqlite``.

Table of Contents
-----------------

* :ref:`connection-pooling`
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

Use Parameterized Queries
~~~~~~~~~~~~~~~~~~~~~~~~~~

**Good:**

.. code-block:: python

   await conn.execute("INSERT INTO users (name) VALUES (?)", [name])

**Bad:**

.. code-block:: python

   await conn.execute(f"INSERT INTO users (name) VALUES ('{name}')")  # SQL injection risk, no caching

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

.. _best-practices:

Best Practices
--------------

1. Always Use Context Managers
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: python

   # Good
   async with connect("example.db") as conn:
       await conn.execute("CREATE TABLE test (id INTEGER)")

2. Use Transactions for Related Operations
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

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

Further Reading
---------------

* :doc:`performance` - Performance tuning guide
* :doc:`migration-guide` - Migrating from aiosqlite
* `SQLite Performance Tuning <https://www.sqlite.org/performance.html>`_
* `SQLite PRAGMA Documentation <https://www.sqlite.org/pragma.html>`_
