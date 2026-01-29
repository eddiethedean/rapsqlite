Connection
==========

.. autoclass:: rapsqlite.Connection
   :members:
   :undoc-members:
   :show-inheritance:
   :special-members: __init__, __aenter__, __aexit__

The ``Connection`` class represents an async SQLite database connection.

Example
-------

.. code-block:: python

   from rapsqlite import Connection

   async with Connection("example.db") as conn:
       await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
       rows = await conn.fetch_all("SELECT * FROM test")

Methods
-------

.. automethod:: rapsqlite.Connection.execute
.. automethod:: rapsqlite.Connection.fetch_all
.. automethod:: rapsqlite.Connection.fetch_one
.. automethod:: rapsqlite.Connection.fetch_optional
.. automethod:: rapsqlite.Connection.execute_many
.. automethod:: rapsqlite.Connection.begin
.. automethod:: rapsqlite.Connection.commit
.. automethod:: rapsqlite.Connection.rollback
.. automethod:: rapsqlite.Connection.transaction
.. automethod:: rapsqlite.Connection.iterdump
.. automethod:: rapsqlite.Connection.backup

Properties
----------

.. autoattribute:: rapsqlite.Connection.row_factory
.. autoattribute:: rapsqlite.Connection.text_factory
.. autoattribute:: rapsqlite.Connection.pool_size
.. autoattribute:: rapsqlite.Connection.connection_timeout
.. autoattribute:: rapsqlite.Connection.timeout
.. autoattribute:: rapsqlite.Connection.include_query_in_errors

Callback Methods
-----------------

.. automethod:: rapsqlite.Connection.create_function
.. automethod:: rapsqlite.Connection.set_trace_callback
.. automethod:: rapsqlite.Connection.set_authorizer
.. automethod:: rapsqlite.Connection.set_progress_handler
.. automethod:: rapsqlite.Connection.enable_load_extension
.. automethod:: rapsqlite.Connection.load_extension

Callback Exception Handling
~~~~~~~~~~~~~~~~~~~~~~~~~~~

When using callback methods, exceptions in your Python callbacks are handled as follows:

- **User-defined functions** (`create_function`): Exceptions are converted to SQLite errors, causing the query to fail with an ``OperationalError``
- **Trace callbacks** (`set_trace_callback`): Exceptions are silently ignored to prevent affecting database operations
- **Authorizer callbacks** (`set_authorizer`): Exceptions default to **DENY** (fail-secure) - operations are denied if callback raises
- **Progress handlers** (`set_progress_handler`): Exceptions default to **continue** - operation continues even if callback raises

Best practice: Always handle exceptions within your callback functions to avoid unexpected behavior.
