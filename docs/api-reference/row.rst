Row
===

.. autoclass:: rapsqlite.Row
   :members:
   :undoc-members:
   :show-inheritance:
   :special-members: __getitem__, __len__, __contains__, __iter__

The ``Row`` class provides dict-like access to query results, similar to ``aiosqlite.Row``.

Example
-------

.. code-block:: python

   from rapsqlite import connect, Row

   async with connect("example.db") as conn:
       conn.row_factory = Row
       rows = await conn.fetch_all("SELECT id, name FROM users")
       # Access by column name
       print(rows[0]["name"])
       # Access by index
       print(rows[0][0])
       # Get column names
       print(rows[0].keys())
