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

Callback Exception Handling
~~~~~~~~~~~~~~~~~~~~~~~~~~~

When using callback methods, exceptions in your Python callbacks are handled as follows:

- **User-defined functions** (`create_function`): Exceptions are converted to SQLite errors, causing the query to fail with an ``OperationalError``
- **Trace callbacks** (`set_trace_callback`): Exceptions are silently ignored to prevent affecting database operations
- **Authorizer callbacks** (`set_authorizer`): Exceptions default to **DENY** (fail-secure) - operations are denied if callback raises
- **Progress handlers** (`set_progress_handler`): Exceptions default to **continue** - operation continues even if callback raises

Best practice: Always handle exceptions within your callback functions to avoid unexpected behavior.

Pool metrics and health
~~~~~~~~~~~~~~~~~~~~~~~

When using a connection (which uses an internal pool), you can observe pool state and run health checks:

- **``pool_metrics()``** (async): Returns a dict with ``size`` (total connections in pool), ``num_idle`` (idle connections), and ``in_use`` (connections currently in use). Use this to monitor pool usage in production (e.g. log periodically or expose via a metrics endpoint).

- **``pool_health()``** (async): Runs a minimal health check (``SELECT 1``) and returns ``True`` on success; raises on failure. Use for liveness/readiness probes.
