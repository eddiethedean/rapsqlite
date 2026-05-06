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

import inspect
import os
from typing import TYPE_CHECKING, Any, TypeAlias, cast

import builtins as _builtins

from rapsqlite._compat import apply_compat
from rapsqlite._connection_state import apply_state
from rapsqlite._metrics import PoolMetricsGauges, pool_metrics_gauges
from rapsqlite._query_helpers import (
    _StreamChunksIterator,
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


try:
    # Preferred: import extension from the local module name used when installed.
    import _rapsqlite as _ext
except ImportError:  # pragma: no cover - fallback for editable installs/alt layouts
    try:
        from rapsqlite import _rapsqlite as _ext
    except ImportError as exc:  # pragma: no cover
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        ) from exc

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
try:
    InterfaceError = _ext.InterfaceError
except AttributeError:  # pragma: no cover - compatibility with older wheels

    class InterfaceError(Error):  # type: ignore[no-redef,misc,valid-type]
        pass


try:
    DataError = _ext.DataError
except AttributeError:  # pragma: no cover - compatibility with older wheels

    class DataError(DatabaseError):  # type: ignore[no-redef,misc,valid-type]
        pass


try:
    InternalError = _ext.InternalError
except AttributeError:  # pragma: no cover - compatibility with older wheels

    class InternalError(DatabaseError):  # type: ignore[no-redef,misc,valid-type]
        pass


try:
    NotSupportedError = _ext.NotSupportedError
except AttributeError:  # pragma: no cover - compatibility with older wheels

    class NotSupportedError(DatabaseError):  # type: ignore[no-redef,misc,valid-type]
        pass


try:
    ValueError = _ext.ValueError
except AttributeError:  # pragma: no cover - compatibility with older wheels
    # Fall back to the built-in ValueError so callers can still catch it.
    ValueError = _builtins.ValueError

# Export RapRow as Row for aiosqlite compatibility, but fall back to Row if
# running against an older build that does not expose RapRow explicitly.
try:
    Row = getattr(_ext, "RapRow", None) or getattr(_ext, "Row")
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


# Connection.execute_iter (streaming helper) - uses Connection.fetch_all
def _connection_execute_iter(
    self: ConnectionT,
    sql: str,
    parameters: Any | None = None,
    chunk_size: int | None = None,
) -> "_StreamChunksIterator":
    """Return an async iterator that yields rows in chunks (streaming / memory-efficient)."""
    return execute_iter(self, sql, parameters, chunk_size)


Connection.execute_iter = _connection_execute_iter

__version__: str = "0.3.3"
__all__: list[str] = [
    "Connection",
    "ConnectionT",
    "Cursor",
    "CursorT",
    "Row",
    "connect",
    "PoolMetricsGauges",
    "pool_metrics_gauges",
    "execute_iter",
    "paginate",
    "analyze_query_plan",
    "suggest_indexes",
    "in_clause_query",
    "rows_to_dicts",
    "timed_fetch_all",
    "transaction_retry",
    "transaction_with_timeout",
    "Error",
    "Warning",
    "InterfaceError",
    "DatabaseError",
    "DataError",
    "OperationalError",
    "IntegrityError",
    "InternalError",
    "ProgrammingError",
    "NotSupportedError",
    "ValueError",
]


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
        pool_size: Optional max connections in the shared pool for this path.
            Set before first use so the pool is created with this size (e.g. for
            high-concurrency tests). Default None (pool uses internal minimum).
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
    if aiosqlite_compat:
        conn.row_factory = "tuple"
    return cast(ConnectionT, conn)


# Register sqlite+rapsqlite dialect so create_async_engine("sqlite+rapsqlite:///...") works
# without a separate "import rapsqlite.sqlalchemy". (Entry point in pyproject.toml does the
# same at install time; this covers editable installs and runtimes where entry points aren't used.)
try:
    import rapsqlite.sqlalchemy  # noqa: F401
except ImportError:
    pass
