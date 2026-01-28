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
