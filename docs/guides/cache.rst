Process-local cache API
=======================

``SQLiteCache`` provides a compact TTL-aware BLOB cache on top of an existing
rapsqlite connection. Cache reads and writes use native rapsqlite operations:
keys and values are bound in Rust, TTL timestamps are computed there, and reads
return BLOBs without building general result rows. Writes use an atomic SQLite
upsert. Values are bytes; serialize JSON or other application objects before
storing them.

Basic usage
-----------

The cache uses the supplied connection but does not own or close it. Schema
creation is lazy, or can be requested eagerly with ``initialize()``:

.. code-block:: python

   from rapsqlite import SQLiteCache, connect_memory

   async with connect_memory(name="api-cache", session_affinity=True) as conn:
       cache = SQLiteCache(conn)
       await cache.initialize()

       await cache.set("user:42", b'{"name":"Ada"}', ttl=60)
       value = await cache.get("user:42")
       if value is None:
           ...  # cache miss or expired entry

       deleted = await cache.delete("user:42")

For the lowest request-path overhead, initialize the cache before starting a
transaction. If first initialization occurs inside a transaction, it is treated
as provisional and retried on later operations until initialization succeeds
outside a transaction, so a rollback cannot leave the cache pointing at a
missing table.

TTL and expiration cleanup
--------------------------

``ttl`` is expressed in seconds using the system wall clock. ``None`` means no
expiration; zero expires immediately. Negative, non-finite, and boolean TTLs
are rejected. ``get()`` returns ``None`` for absent and expired keys, but does
not delete expired rows on the read path. Call ``cleanup_expired()`` from a
maintenance task to remove them; it deletes at most 1,000 rows by default and
returns the number removed. Pass a larger positive ``limit`` to clean more in
one call.

Keys must be strings and values must be bytes. The optional ``table_name``
selects the cache table and must be a simple SQLite identifier. Multiple cache
objects using the same database and table name share entries.

FastAPI lifespan example
------------------------

Keep the connection and cache alive for the application process, rather than
constructing either on each request:

.. code-block:: python

   from contextlib import asynccontextmanager
   from fastapi import FastAPI, Request
   from rapsqlite import SQLiteCache, connect_memory

   @asynccontextmanager
   async def lifespan(app: FastAPI):
       async with connect_memory(name="my-api-cache", session_affinity=True) as conn:
           app.state.cache = SQLiteCache(conn)
           await app.state.cache.initialize()
           yield

   app = FastAPI(lifespan=lifespan)

   async def get_cached_value(request: Request, key: str) -> bytes | None:
       return await request.app.state.cache.get(key)

An in-memory database is shared only by live connections with the same name
inside the same process. Use a file-backed database or a separate shared cache
service when entries must be shared across worker processes or hosts. Cache
operations from multiple ``SQLiteCache`` objects sharing one logical connection
are serialized by a native connection-level lock. That lock does not cover
ordinary ``Connection`` calls; callers should coordinate concurrent database
work that mixes those calls with cache operations. This API does not add a
multiplexed read mode or Redis-style multi-command pipeline.

The API is intended for process-local cache use, not as a universal Redis
replacement. Benchmark it on deployment hardware with the payload and
concurrency profile that matter to your application.
