Exceptions
==========

rapsqlite defines the following exception classes, matching the aiosqlite API:

.. autoexception:: rapsqlite.Error
   :show-inheritance:

.. autoexception:: rapsqlite.Warning
   :show-inheritance:

.. autoexception:: rapsqlite.DatabaseError
   :show-inheritance:

.. autoexception:: rapsqlite.OperationalError
   :show-inheritance:

.. autoexception:: rapsqlite.ProgrammingError
   :show-inheritance:

.. autoexception:: rapsqlite.IntegrityError
   :show-inheritance:

Exception Hierarchy
-------------------

All exceptions inherit from ``Error``, which inherits from Python's ``Exception``:

.. code-block:: text

   Exception
   └── Error
       ├── Warning
       ├── DatabaseError
       │   ├── OperationalError
       │   └── ProgrammingError
       └── IntegrityError

Usage
-----

.. code-block:: python

   from rapsqlite import connect, IntegrityError, OperationalError

   async with connect("example.db") as conn:
       try:
           await conn.execute("INSERT INTO users (email) VALUES (?)", ["duplicate@example.com"])
       except IntegrityError:
           print("Duplicate email")
       except OperationalError as e:
           print(f"Database error: {e}")
