Quick Start
===========

This guide will get you up and running with rapsqlite in just a few minutes.

Basic Connection
----------------

The simplest way to use rapsqlite is with the ``connect()`` function:

.. code-block:: python

   import asyncio
   from rapsqlite import connect

   async def main():
       async with connect("example.db") as conn:
           await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
           await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
           rows = await conn.fetch_all("SELECT * FROM users")
           print(rows)

   asyncio.run(main())

The connection is automatically closed when exiting the ``async with`` block.

Creating Tables
---------------

.. code-block:: python

   async with connect("example.db") as conn:
       await conn.execute("""
           CREATE TABLE users (
               id INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               email TEXT UNIQUE
           )
       """)

Inserting Data
--------------

Use parameterized queries to safely insert data:

.. code-block:: python

   async with connect("example.db") as conn:
       # Single insert
       await conn.execute(
           "INSERT INTO users (name, email) VALUES (?, ?)",
           ["Alice", "alice@example.com"]
       )

       # Multiple inserts (batch)
       users = [
           ["Bob", "bob@example.com"],
           ["Charlie", "charlie@example.com"]
       ]
       await conn.execute_many(
           "INSERT INTO users (name, email) VALUES (?, ?)",
           users
       )

Querying Data
-------------

.. code-block:: python

   async with connect("example.db") as conn:
       # Fetch all rows
       all_users = await conn.fetch_all("SELECT * FROM users")

       # Fetch one row (raises if not found)
       user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [1])

       # Fetch optional row (returns None if not found)
       user = await conn.fetch_optional("SELECT * FROM users WHERE id = ?", [999])

Transactions
------------

Use transactions to ensure data consistency:

.. code-block:: python

   async with connect("example.db") as conn:
       # Transaction context manager (recommended)
       async with conn.transaction():
           await conn.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
           await conn.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 2")
           # Automatically commits on success, rolls back on exception

       # Or explicit transaction control
       await conn.begin()
       try:
           await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
           await conn.commit()
       except Exception:
           await conn.rollback()
           raise

Using Cursors
-------------

.. code-block:: python

   async with connect("example.db") as conn:
       cursor = conn.cursor()
       await cursor.execute("SELECT * FROM users")
       
       # Fetch one row
       row = await cursor.fetchone()
       
       # Fetch many rows
       rows = await cursor.fetchmany(10)
       
       # Fetch all rows
       all_rows = await cursor.fetchall()

       # Async iteration
       await cursor.execute("SELECT * FROM users")
       async for row in cursor:
           print(row)

Row Factories
-------------

Customize how rows are returned:

.. code-block:: python

   async with connect("example.db") as conn:
       # Dict factory
       conn.row_factory = "dict"
       rows = await conn.fetch_all("SELECT * FROM users")
       # rows[0] = {"id": 1, "name": "Alice"}

       # Tuple factory
       conn.row_factory = "tuple"
       rows = await conn.fetch_all("SELECT * FROM users")
       # rows[0] = (1, "Alice")

       # Row class (dict-like access)
       from rapsqlite import Row
       conn.row_factory = Row
       rows = await conn.fetch_all("SELECT * FROM users")
       # rows[0]["name"] or rows[0][0]

Next Steps
----------

* See :doc:`api-reference/index` for complete API documentation
* Check out :doc:`guides/advanced-usage` for advanced patterns
* Read :doc:`guides/migration-guide` if migrating from aiosqlite
