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

import asyncio
import os
import re
import time
from collections.abc import Callable
from typing import TYPE_CHECKING, Any, TypedDict, TypeAlias, cast

import builtins as _builtins

from rapsqlite._compat import apply_compat
from rapsqlite._connection_state import apply_state


class PoolMetricsGauges(TypedDict):
    """Return type for pool_metrics_gauges(); gauge names for Prometheus/custom metrics."""

    rapsqlite_pool_size: int
    rapsqlite_pool_num_idle: int
    rapsqlite_pool_in_use: int


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
else:
    ConnectionT = Connection

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
    return _StreamChunksIterator(self, sql, parameters, chunk_size)


Connection.execute_iter = _connection_execute_iter

__version__: str = "0.3.0"
__all__: list[str] = [
    "Connection",
    "Cursor",
    "Row",
    "connect",
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
    try:
        conn = Connection(
            path_str,
            pragmas=pragmas,
            timeout=timeout,
            iter_chunk_size=iter_chunk_size,
            loop_param=loop,
        )
    except TypeError as e:
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
    if aiosqlite_compat:
        conn.row_factory = "tuple"
    return cast(ConnectionT, conn)


async def pool_metrics_gauges(conn: ConnectionT) -> PoolMetricsGauges:
    """Return pool metrics as a dict of gauge names to values for Prometheus or custom metrics.

    Calls ``conn.pool_metrics()`` and maps the result to gauge-style keys:
    ``rapsqlite_pool_size``, ``rapsqlite_pool_num_idle``, ``rapsqlite_pool_in_use``.
    Use this to expose pool state on a /metrics endpoint or feed into a metrics system.

    Example:
        async with connect("app.db") as conn:
            gauges = await pool_metrics_gauges(conn)
            # e.g. {"rapsqlite_pool_size": 5, "rapsqlite_pool_num_idle": 3, "rapsqlite_pool_in_use": 2}
            for name, value in gauges.items():
                # Expose as Prometheus gauge, or log, etc.
                pass
    """
    m = await conn.pool_metrics()
    return {
        "rapsqlite_pool_size": m.get("size", 0),
        "rapsqlite_pool_num_idle": m.get("num_idle", 0),
        "rapsqlite_pool_in_use": m.get("in_use", 0),
    }


async def timed_fetch_all(
    conn: ConnectionT,
    sql: str,
    parameters: Any | None = None,
    on_timing: Callable[[float, str], None] | None = None,
) -> list[list[Any]] | tuple[list[list[Any]], float]:
    """Run fetch_all and record duration; optionally call on_timing(duration_secs, sql).

    If on_timing is None, returns (rows, duration_secs). If on_timing is provided,
    calls on_timing(duration_secs, sql) and returns rows only.
    """
    t0 = time.perf_counter()
    rows = await conn.fetch_all(sql, parameters)
    duration = time.perf_counter() - t0
    if on_timing is not None:
        on_timing(duration, sql)
        return cast(list[list[Any]], rows)
    return cast(tuple[list[list[Any]], float], (rows, duration))


async def transaction_retry(
    conn: ConnectionT,
    work: Any,
    max_retries: int = 5,
    initial_delay: float = 0.01,
    max_delay: float = 1.0,
) -> Any:
    """Run a transaction with retry on transient errors (e.g. SQLITE_BUSY, SQLITE_LOCKED).

    ``work`` is a callable that returns an awaitable (e.g. an async function); it is
    invoked once per attempt so each retry runs fresh. Retries with exponential backoff.
    Example:
        async with connect("app.db") as conn:
            async def do_work():
                await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
            await transaction_retry(conn, do_work, max_retries=3)
    """
    last_err: Exception | None = None
    delay = initial_delay
    for attempt in range(max_retries):
        try:
            await conn.begin()
            try:
                coro = work() if callable(work) else work
                result = await coro
                await conn.commit()
                return result
            except Exception as e:
                await conn.rollback()
                last_err = e
                msg = str(e).lower()
                if "busy" in msg or "locked" in msg:
                    if attempt < max_retries - 1:
                        await asyncio.sleep(min(delay, max_delay))
                        delay = min(delay * 2, max_delay)
                        continue
                raise
        except Exception as e:
            last_err = e
            raise
    # If max_retries is 0, we never enter the loop, so raise the last error or return None
    if last_err is not None:
        raise last_err
    raise RuntimeError("transaction_retry: max_retries must be at least 1")


async def transaction_with_timeout(
    conn: ConnectionT,
    work: Any,
    timeout_secs: float = 30.0,
) -> Any:
    """Run a transaction with a timeout (Phase 3.3).

    Wraps the transaction body in asyncio.wait_for. Raises asyncio.TimeoutError
    if the transaction (including work) exceeds timeout_secs.

    Args:
        conn: Database connection
        work: Callable that returns an awaitable (e.g. async def do_work(): ...)
        timeout_secs: Maximum seconds for the transaction (default 30)

    Example:
        async with connect("app.db") as conn:
            async def do_work():
                await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
            await transaction_with_timeout(conn, do_work, timeout_secs=5)
    """

    async def _run() -> Any:
        async with conn.transaction():
            coro = work() if callable(work) else work
            return await coro

    return await asyncio.wait_for(_run(), timeout=timeout_secs)


def execute_iter(
    conn: ConnectionT,
    sql: str,
    parameters: Any | None = None,
    chunk_size: int | None = None,
) -> "_StreamChunksIterator":
    """Return an async iterator that yields rows in chunks (streaming / memory-efficient).

    Uses LIMIT/OFFSET under the hood so memory stays bounded by chunk_size.
    Single connection is used for the duration of iteration; closing the
    connection or cancelling the task stops iteration.

    Example:
        async with connect("app.db") as conn:
            async for chunk in execute_iter(conn, "SELECT * FROM big", chunk_size=500):
                for row in chunk:
                    process(row)
    """
    return _StreamChunksIterator(conn, sql, parameters, chunk_size)


async def paginate(
    conn: ConnectionT,
    sql: str,
    parameters: Any | None = None,
    page_size: int = 64,
    offset: int = 0,
) -> list[list[Any]]:
    """Fetch one page of rows from a SELECT query.

    Uses LIMIT/OFFSET under the hood. For multiple pages, call with
    incrementing offset: paginate(conn, sql, params, 100, 0), then
    paginate(conn, sql, params, 100, 100), etc.

    Args:
        conn: Database connection
        sql: SELECT query (no LIMIT/OFFSET; this adds them)
        parameters: Optional query parameters
        page_size: Number of rows per page
        offset: Row offset for this page

    Returns:
        List of rows for this page (empty if past end)
    """
    sql_clean = sql.strip().rstrip(";")
    wrapped = f"SELECT * FROM ({sql_clean}) LIMIT ? OFFSET ?"
    params = list(parameters) if parameters is not None else []
    rows = await conn.fetch_all(wrapped, params + [page_size, offset])
    return cast(list[list[Any]], rows)


async def analyze_query_plan(
    conn: ConnectionT,
    sql: str,
    parameters: Any | None = None,
) -> dict[str, Any]:
    """Run EXPLAIN QUERY PLAN and return structured analysis (Phase 3.1).

    Returns a dict with:
        - rows: Raw EXPLAIN QUERY PLAN result rows
        - details: List of detail strings (4th column)
        - uses_index: True if plan uses an index
        - table_scan: True if plan does a full table scan

    Example:
        analysis = await analyze_query_plan(conn, "SELECT * FROM t WHERE id = ?", [1])
        if analysis["table_scan"] and not analysis["uses_index"]:
            print("Consider adding an index")
    """
    rows = await conn.explain_query_plan(sql, parameters)
    details: list[str] = []
    for row in rows:
        if isinstance(row, (list, tuple)) and len(row) >= 4:
            details.append(str(row[3]))
        elif isinstance(row, dict) and "detail" in row:
            details.append(str(row["detail"]))
        else:
            details.append(str(row))
    detail_str = " ".join(details).upper()
    return {
        "rows": rows,
        "details": details,
        "uses_index": "USING INDEX" in detail_str or "INDEX" in detail_str,
        "table_scan": "SCAN TABLE" in detail_str or "TABLE SCAN" in detail_str,
    }


async def suggest_indexes(
    conn: ConnectionT,
    sql: str,
    parameters: Any | None = None,
) -> list[dict[str, Any]]:
    """Suggest indexes when query plan indicates a full table scan (Phase 3.1).

    Calls analyze_query_plan and, if table_scan without uses_index, parses
    the plan to extract table names and returns index suggestions.

    Returns a list of dicts, e.g.:
        [{"table": "users", "column": "", "suggestion": "CREATE INDEX idx_users_<col> ON users(<col>)"}]

    Example:
        suggestions = await suggest_indexes(conn, "SELECT * FROM users WHERE email = ?", ["x"])
        for s in suggestions:
            print(s["suggestion"])
    """
    analysis = await analyze_query_plan(conn, sql, parameters)
    if not analysis.get("table_scan") or analysis.get("uses_index"):
        return []

    suggestions: list[dict[str, Any]] = []
    seen_tables: set[str] = set()

    for detail in analysis.get("details", []):
        detail_upper = str(detail).upper()
        # SCAN TABLE tablename or SCAN TABLE tablename AS alias
        match = re.search(r"SCAN\s+TABLE\s+(\w+)", detail_upper, re.IGNORECASE)
        if match:
            table = match.group(1)
            if table not in seen_tables:
                seen_tables.add(table)
                suggestions.append(
                    {
                        "table": table,
                        "column": "",
                        "suggestion": (
                            f"CREATE INDEX idx_{table}_<columns> ON {table}(<columns>) "
                            "-- add columns used in WHERE, ORDER BY, or JOIN"
                        ),
                    }
                )

    return suggestions


def in_clause_query(
    sql: str, values: list[Any] | tuple[Any, ...]
) -> tuple[str, list[Any]]:
    """Expand IN (?) to IN (?,?,...) for use with fetch_all (Phase 3.7).

    Use when your SQL has a single IN (?) clause and you want to pass a list of values.

    Args:
        sql: Query containing exactly one ``IN (?)`` placeholder
        values: List or tuple of values for the IN clause

    Returns:
        (processed_sql, flattened_params) to pass to ``fetch_all``

    Example:
        sql, params = in_clause_query("SELECT * FROM users WHERE id IN (?)", [1, 2, 3])
        rows = await conn.fetch_all(sql, params)
        # Equivalent to: SELECT * FROM users WHERE id IN (?, ?, ?) with params [1, 2, 3]

    Raises:
        ValueError: If values is empty (IN () is invalid in SQLite)
    """
    if len(values) == 0:
        raise ValueError(
            "in_clause_query requires at least one value; IN () is invalid in SQLite"
        )
    placeholders = ",".join("?" * len(values))
    new_sql = re.sub(
        r"\bIN\s*\(\s*\?\s*\)",
        f"IN ({placeholders})",
        sql,
        count=1,
        flags=re.IGNORECASE,
    )
    if new_sql == sql:
        raise ValueError(
            "in_clause_query: sql must contain 'IN (?)' placeholder; found no match"
        )
    return (new_sql, list(values))


def rows_to_dicts(
    rows: list[Any],
    columns: list[str] | tuple[str, ...] | None = None,
) -> list[dict[str, Any]]:
    """Convert rows (list of list/tuple) to list of dicts using column names (Phase 3.1).

    Use when you have rows from fetch_all and want dicts keyed by column name.
    Requires columns to be provided (e.g. from cursor.description).

    Args:
        rows: List of rows, each row a list or tuple of values
        columns: Column names in order (e.g. from desc[0] for desc in cursor.description)

    Returns:
        List of dicts, each mapping column name to value

    Example:
        rows = await conn.fetch_all("SELECT id, name FROM users")
        cols = ["id", "name"]
        dicts = rows_to_dicts(rows, cols)
        # [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]
    """
    if columns is None or len(columns) == 0:
        return []
    col_list = list(columns)
    result: list[dict[str, Any]] = []
    for row in rows:
        if hasattr(row, "keys") and callable(getattr(row, "keys")):
            result.append(dict(row))
        else:
            row_iter = row if isinstance(row, (list, tuple)) else list(row)
            result.append(dict(zip(col_list, row_iter)))
    return result


class _StreamChunksIterator:
    """Async iterator yielding chunks of rows from a SELECT (LIMIT/OFFSET under the hood)."""

    def __init__(
        self,
        conn: ConnectionT,
        sql: str,
        parameters: Any | None = None,
        chunk_size: int | None = None,
    ) -> None:
        self._conn = conn
        self._sql = sql.strip().rstrip(";")
        self._params = list(parameters) if parameters is not None else []
        try:
            default_chunk = getattr(conn, "iter_chunk_size", 64)
            default_chunk = int(default_chunk) if default_chunk is not None else 64
        except (TypeError, ValueError):
            default_chunk = 64
        self._chunk_size = int(chunk_size) if chunk_size is not None else default_chunk
        self._offset = 0

    def __aiter__(self) -> "_StreamChunksIterator":
        return self

    async def __anext__(self) -> list[list[Any]]:
        # Wrap query so we can paginate: SELECT * FROM (user_query) LIMIT ? OFFSET ?
        wrapped = f"SELECT * FROM ({self._sql}) LIMIT ? OFFSET ?"
        params = self._params + [self._chunk_size, self._offset]
        rows = await self._conn.fetch_all(wrapped, params)
        if not rows:
            raise StopAsyncIteration
        self._offset += len(rows)
        return cast(list[list[Any]], rows)
