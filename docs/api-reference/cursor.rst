Cursor
======

.. autoclass:: rapsqlite.Cursor
   :members:
   :undoc-members:
   :show-inheritance:
   :special-members: __aiter__, __anext__

The ``Cursor`` class provides a way to execute queries and fetch results.

Example
-------

.. code-block:: python

   from rapsqlite import connect

   async with connect("example.db") as conn:
       cursor = conn.cursor()
       await cursor.execute("SELECT * FROM users")
       rows = await cursor.fetchall()

Methods
-------

.. automethod:: rapsqlite.Cursor.execute
.. automethod:: rapsqlite.Cursor.executemany
.. automethod:: rapsqlite.Cursor.executescript
.. automethod:: rapsqlite.Cursor.fetchone
.. automethod:: rapsqlite.Cursor.fetchmany
.. automethod:: rapsqlite.Cursor.fetchall
