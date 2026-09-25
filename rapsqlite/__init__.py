"""True async SQLite — no fake async, no GIL stalls.

rapsqlite provides true async SQLite operations for Python, backed by Rust,
Tokio, and sqlx. Unlike libraries that wrap blocking database calls in async
syntax, rapsqlite guarantees that all database operations execute outside the
Python GIL, ensuring event loops never stall under load. Supports type adapters
and converters (register_adapter, register_converter) and custom aggregates and
collations (create_aggregate, create_collation) per-connection (sqlite3-style).

Example:
    Basic usage::

        import asyncio
        from rapsqlite import Connection

        async def main():
            async with Connection("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                await conn.execute("INSERT INTO test (value) VALUES ('hello')")
                rows = await conn.fetch_all("SELECT * FROM test")
                print(rows)
                # Output: [[1, 'hello']]

        asyncio.run(main())

    Using the connect() function (aiosqlite-compatible)::

        import asyncio
        from rapsqlite import connect

        async def main():
            async with connect("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                await conn.execute("INSERT INTO test (value) VALUES ('hello')")
                rows = await conn.fetch_all("SELECT * FROM test")
                print(rows)
                # Output: [[1, 'hello']]

        asyncio.run(main())

    Transactions::

        async with Connection("example.db") as conn:
            await conn.begin()
            try:
                await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
                await conn.commit()
            except Exception:
                await conn.rollback()
"""

import builtins as _builtins
import inspect
import importlib
import os
import uuid
from typing import TYPE_CHECKING, Any, Protocol, TypeAlias, cast
from urllib.parse import quote

from rapsqlite._compat import apply_compat
from rapsqlite._connection_state import apply_state
from rapsqlite._metrics import PoolMetrics, PoolMetricsGauges, pool_metrics_gauges
from rapsqlite._prepared import PreparedQuery
from rapsqlite._query_helpers import (
    analyze_query_plan,
    execute_iter,
    in_clause_query,
    paginate,
    rows_to_dicts,
    suggest_indexes,
    timed_fetch_all,
)
from rapsqlite._transaction_helpers import (
    transaction_retry,
    transaction_with_timeout,
)


class StreamChunksIterator(Protocol):
    """Async iterator returned by :func:`execute_iter`."""

    def __aiter__(self) -> "StreamChunksIterator": ...
    async def __anext__(self) -> list[list[Any]]: ...


try:
    # Preferred: import extension from the local module name used when installed.
    import _rapsqlite as _ext
except ImportError:  # pragma: no cover - fallback for editable installs/alt layouts
    try:
        from rapsqlite import _rapsqlite as _ext
    except ImportError:  # pragma: no cover
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        ) from None

# Re-export symbols from the extension module.
Connection = _ext.Connection
Cursor = _ext.Cursor
if TYPE_CHECKING:
    ConnectionT: TypeAlias = _ext.Connection
    CursorT: TypeAlias = _ext.Cursor
else:
    ConnectionT = Connection
    CursorT = Cursor

Error = _ext.Error
Warning = _ext.Warning
DatabaseError = _ext.DatabaseError
OperationalError = _ext.OperationalError
ProgrammingError = _ext.ProgrammingError
IntegrityError = _ext.IntegrityError


def _compat_exception(name: str, base: type[BaseException]) -> type[BaseException]:
    """Return an extension exception, or a compatible fallback for old wheels."""

    exception = getattr(_ext, name, None)
    if exception is None:
        return type(name, (base,), {})
    return cast(type[BaseException], exception)


InterfaceError: type[BaseException] = _compat_exception(
    "InterfaceError", cast(type[BaseException], Error)
)
DataError: type[BaseException] = _compat_exception(
    "DataError", cast(type[BaseException], DatabaseError)
)
InternalError: type[BaseException] = _compat_exception(
    "InternalError", cast(type[BaseException], DatabaseError)
)
NotSupportedError: type[BaseException] = _compat_exception(
    "NotSupportedError", cast(type[BaseException], DatabaseError)
)


ValueError: type[BaseException] = cast(
    type[BaseException], getattr(_ext, "ValueError", _builtins.ValueError)
)

# Export RapRow as Row for aiosqlite compatibility, but fall back to Row if
# running against an older build that does not expose RapRow explicitly.
if TYPE_CHECKING:
    Row: TypeAlias = _ext.RapRow
else:
    try:
        Row = getattr(_ext, "RapRow", None) or _ext.Row
    except AttributeError:
        # If neither RapRow nor Row exists, create a placeholder or raise a helpful error
        raise ImportError(
            "RapRow class not found in _rapsqlite module. "
            "The extension module may need to be rebuilt. "
            f"Available attributes: {[x for x in dir(_ext) if not x.startswith('_')]}"
        ) from None

# Apply aiosqlite compat patches, then connection state cache (order matters).
apply_compat(Connection, Cursor, operational_error=OperationalError)
apply_state(Connection)


async def _connection_fetch_blob(
    self: ConnectionT,
    query: str,
    parameters: Any | None = None,
) -> bytes | None:
    """Fetch one BLOB value without constructing a general row."""

    # Check SQLite's runtime storage class before text_factory/converters can
    # turn a TEXT value into Python bytes.
    value = await cast(Any, self).fetch_scalar(query, parameters, _require_blob=True)
    if value is None:
        return None
    if not isinstance(value, bytes):
        raise TypeError("fetch_blob() requires a BLOB or NULL result")
    return value


def _connection_prepare(
    self: ConnectionT,
    query: str,
    *,
    raw: bool = False,
    blob: bool = False,
) -> PreparedQuery:
    """Create a reusable connection-bound query object."""

    return PreparedQuery(self, query, raw=raw, blob=blob)


Connection.fetch_blob = _connection_fetch_blob
Connection.prepare = _connection_prepare


# Connection.execute_iter (streaming helper) - uses Connection.fetch_all
def _connection_execute_iter(
    self: ConnectionT,
    sql: str,
    parameters: Any | None = None,
    chunk_size: int | None = None,
) -> StreamChunksIterator:
    """Return an async iterator that yields rows in chunks (streaming / memory-efficient)."""
    return execute_iter(self, sql, parameters, chunk_size)


Connection.execute_iter = _connection_execute_iter

__version__: str = "0.4.0"
__all__: list[str] = [
    "Connection",
    "ConnectionT",
    "Cursor",
    "CursorT",
    "DataError",
    "DatabaseError",
    "Error",
    "IntegrityError",
    "InterfaceError",
    "InternalError",
    "NotSupportedError",
    "OperationalError",
    "PoolMetrics",
    "PoolMetricsGauges",
    "PreparedQuery",
    "ProgrammingError",
    "Row",
    "ValueError",
    "Warning",
    "analyze_query_plan",
    "connect",
    "connect_memory",
    "execute_iter",
    "in_clause_query",
    "paginate",
    "pool_metrics_gauges",
    "rows_to_dicts",
    "suggest_indexes",
    "timed_fetch_all",
    "transaction_retry",
    "transaction_with_timeout",
]


def connect_memory(
    *,
    name: object | None = None,
    pragmas: Any = None,
    timeout: float = 5.0,
    iter_chunk_size: int = 64,
    idle_timeout: int | None = None,
    pool_size: int | None = None,
    session_affinity: bool = False,
) -> ConnectionT:
    """Create an isolated or explicitly shared in-memory SQLite database.

    An unnamed database gets a unique process-local identity. A non-empty name
    shares one database among live ``connect_memory(name=...)`` connections
    with that same name. SQLite closes the database after the last connection
    using that identity is closed or discarded.
    """
    if name is None:
        identity = uuid.uuid4().hex
    elif not isinstance(name, str) or not name:
        raise ValueError("name must be a non-empty string or None")
    else:
        identity = quote(name, safe="")
    uri = f"file:rapsqlite-memory-{identity}?mode=memory&cache=shared"
    return connect(
        uri,
        pragmas=pragmas,
        timeout=timeout,
        iter_chunk_size=iter_chunk_size,
        idle_timeout=idle_timeout,
        pool_size=pool_size,
        session_affinity=session_affinity,
    )


def connect(
    path: str | os.PathLike[str],
    *,
    pragmas: Any = None,
    timeout: float = 5.0,
    iter_chunk_size: int = 64,
    idle_timeout: int | None = None,
    loop: Any = None,
    aiosqlite_compat: bool = False,
    pool_size: int | None = None,
    session_affinity: bool = False,
    **kwargs: Any,
) -> ConnectionT:
    """Connect to a SQLite database.

    This function matches the aiosqlite.connect() API for compatibility,
    allowing seamless migration from aiosqlite to rapsqlite.

    Args:
        path: Path to the SQLite database file. Can be ":memory:" for an
            in-memory database, or a file path. Can also be a URI format:
            "file:path?param=value". The path is validated for security
            (non-empty, no null bytes).
        pragmas: Optional dictionary of PRAGMA settings to apply on connection.
            These are applied when the connection pool is first created.
            Example: {"journal_mode": "WAL", "synchronous": "NORMAL",
            "foreign_keys": True}. See SQLite PRAGMA documentation for
            available settings.
        timeout: How long to wait (in seconds) when the database is locked by
            another process/thread before raising an error. Default: 5.0 seconds.
            This sets SQLite's busy_timeout PRAGMA. Set to 0.0 to disable timeout.
            This matches aiosqlite and sqlite3's timeout parameter.
        iter_chunk_size: Chunk size for iteration (e.g. fetchmany). Default 64.
            Stored for use with cursor iteration; aiosqlite-compatible.
        idle_timeout: Optional seconds. When set, connections idle in the pool
            longer than this are closed. None (default) means no idle timeout.
        loop: Deprecated. Event loop (ignored). Accept-only for aiosqlite
            compatibility.
        aiosqlite_compat: If True, set default row_factory to tuple so that
            fetch_all, fetchone, cursor fetchall/fetchone return tuples (like
            aiosqlite/sqlite3). Use for drop-in ``import rapsqlite as aiosqlite``
            without changing code that expects tuple rows. Default False (rows
            are lists).
        pool_size: Optional maximum connections in the shared pool for this path.
            An explicit value is honored (0 is normalized to 1). If another live
            Connection already created this shared pool, its existing maximum is
            used. Default None selects the internal shared-pool default (25).
        session_affinity: Retain one physical SQLite session between operations
            on this Connection. This can reduce repeated pool acquisition for
            low-latency workloads but consumes one pool slot while the Connection
            is idle. Default False.
        **kwargs: Additional arguments (currently ignored, reserved for future use)

    Returns:
        Connection: An async SQLite connection object that can be used as an
            async context manager. The connection uses lazy initialization -
            the actual database connection pool is created on first use.

    Example:
        With timeout (aiosqlite compatibility)::

            async with connect("example.db", timeout=10.0) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    Raises:
        ValueError: If the database path is invalid (empty or contains null bytes)
        OperationalError: If the database connection cannot be established
            (e.g., permission denied, disk full, etc.)

        Example:
        Basic usage::

            async with connect("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
                await conn.execute("INSERT INTO test DEFAULT VALUES")
                rows = await conn.fetch_all("SELECT * FROM test")
                # rows = [[1]]

        In-memory database::

            async with connect(":memory:") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER)")
                # Database exists only for the duration of the connection

        With PRAGMA settings::

            async with connect("example.db", pragmas={
                "journal_mode": "WAL",
                "synchronous": "NORMAL",
                "foreign_keys": True
            }) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

        URI format::

            async with connect("file:example.db?mode=rwc") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER)")

    Note:
        The connection object supports async context manager protocol. It's
        recommended to use ``async with`` to ensure proper resource cleanup.
        All database operations execute outside the Python GIL, providing true
        async performance.

    See Also:
        :class:`Connection`: For more advanced connection options including
        initialization hooks.
    """
    # Accept pathlib.Path / os.PathLike for aiosqlite compatibility (e.g. aiosqlite smoke tests)
    path_str = os.fspath(path) if not isinstance(path, str) else path

    # Prefer signature-based filtering of kwargs over brittle TypeError message parsing.
    # Older wheels or non-standard builds may not expose a full signature; in that case
    # we fall back to passing only the core arguments and let TypeError surface.
    supports_iter_chunk = False
    supports_loop_param = False
    try:
        sig = inspect.signature(Connection)
    except (TypeError, ValueError):
        sig = None
    if sig is not None:
        supports_iter_chunk = "iter_chunk_size" in sig.parameters
        supports_loop_param = "loop_param" in sig.parameters

    conn_kwargs: dict[str, Any] = {
        "pragmas": pragmas,
        "timeout": timeout,
    }
    if supports_iter_chunk:
        conn_kwargs["iter_chunk_size"] = iter_chunk_size
    if supports_loop_param:
        conn_kwargs["loop_param"] = loop

    try:
        conn = Connection(path_str, **conn_kwargs)
    except TypeError as e:
        # Fallback for older wheels where signature inspection is not reliable.
        err = str(e)
        if (
            "iter_chunk_size" in err
            or "loop_param" in err
            or "unexpected keyword argument" in err
        ):
            conn = Connection(path_str, pragmas=pragmas, timeout=timeout)
        else:
            raise
    if idle_timeout is not None:
        conn.idle_timeout = idle_timeout
    if pool_size is not None:
        conn.pool_size = pool_size
    conn.session_affinity = session_affinity
    if aiosqlite_compat:
        conn.row_factory = "tuple"
    return conn


# Register sqlite+rapsqlite dialect so create_async_engine("sqlite+rapsqlite:///...") works
# without a separate "import rapsqlite.sqlalchemy". (Entry point in pyproject.toml does the
# same at install time; this covers editable installs and runtimes where entry points aren't used.)
try:
    importlib.import_module("rapsqlite.sqlalchemy")
except ImportError:
    pass
