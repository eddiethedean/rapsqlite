"""True async SQLite — no fake async, no GIL stalls.

rapsqlite provides true async SQLite operations for Python, backed by Rust,
Tokio, and sqlx. Unlike libraries that wrap blocking database calls in async
syntax, rapsqlite guarantees that all database operations execute outside the
Python GIL, ensuring event loops never stall under load.

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
from typing import Any, List, Optional

import builtins as _builtins

try:
    # Preferred: import extension from the local module name used when installed.
    import _rapsqlite as _ext  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - fallback for editable installs/alt layouts
    try:
        from rapsqlite import _rapsqlite as _ext  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        ) from exc

# Re-export symbols from the extension module.
Connection = _ext.Connection


def _connection_del(self: "Connection") -> None:  # type: ignore[valid-type]
    """Best-effort: schedule close() on the running event loop to avoid Tokio panic during GC.

    If a Connection is dropped without close(), Python GC can drop the Rust object;
    sqlx's PoolConnection::Drop then calls Tokio spawn, which panics when no runtime
    is current. Scheduling close() here (when a loop exists) runs cleanup under Tokio.
    This is best-effort only; always use async with or await conn.close().
    """
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return
    try:
        def _schedule_close() -> None:
            try:
                asyncio.create_task(self.close())  # type: ignore[attr-defined]
            except Exception:
                pass
        loop.call_soon_threadsafe(_schedule_close)
    except Exception:
        pass


Connection.__del__ = _connection_del  # type: ignore[assignment]
Cursor = _ext.Cursor
Error = _ext.Error
Warning = _ext.Warning
DatabaseError = _ext.DatabaseError
OperationalError = _ext.OperationalError
ProgrammingError = _ext.ProgrammingError
IntegrityError = _ext.IntegrityError
try:
    InterfaceError = _ext.InterfaceError
except AttributeError:  # pragma: no cover - compatibility with older wheels
    class InterfaceError(Error):  # type: ignore[no-redef,misc]
        pass
try:
    DataError = _ext.DataError
except AttributeError:  # pragma: no cover - compatibility with older wheels
    class DataError(DatabaseError):  # type: ignore[no-redef,misc]
        pass
try:
    InternalError = _ext.InternalError
except AttributeError:  # pragma: no cover - compatibility with older wheels
    class InternalError(DatabaseError):  # type: ignore[no-redef,misc]
        pass
try:
    NotSupportedError = _ext.NotSupportedError
except AttributeError:  # pragma: no cover - compatibility with older wheels
    class NotSupportedError(DatabaseError):  # type: ignore[no-redef,misc]
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

__version__: str = "0.3.0-dev"
__all__: List[str] = [
    "Connection",
    "Cursor",
    "Row",
    "connect",
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
    path: str,
    *,
    pragmas: Any = None,
    timeout: float = 5.0,
    iter_chunk_size: int = 64,
    loop: Any = None,
    **kwargs: Any,
) -> "Connection":  # type: ignore[valid-type]
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
        loop: Deprecated. Event loop (ignored). Accept-only for aiosqlite
            compatibility.
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
        return Connection(
            path_str,
            pragmas=pragmas,
            timeout=timeout,
            iter_chunk_size=iter_chunk_size,
            loop_param=loop,
        )  # type: ignore[no-any-return]
    except TypeError as e:
        err = str(e)
        if "iter_chunk_size" in err or "loop_param" in err or "unexpected keyword argument" in err:
            # Older _rapsqlite build without connect() params (e.g. wrong Python / stale wheel)
            return Connection(path_str, pragmas=pragmas, timeout=timeout)  # type: ignore[no-any-return]
        raise


# -----------------------------------------------------------------------------
# aiosqlite-compat helpers: iterdump and backup
# -----------------------------------------------------------------------------

# Save raw methods so we can wrap them while preserving original behaviour.
_raw_iterdump = Connection.iterdump
_raw_backup = Connection.backup


class _IterdumpWrapper:
    """Dual-mode wrapper for iterdump: async-iter and await-to-list.

    This wrapper allows iterdump() to support both async iteration and
    direct await patterns for backwards compatibility.

    Example:
        Async iteration (aiosqlite-compatible)::

            async for line in conn.iterdump():
                print(line)

        Direct await (rapsqlite enhancement)::

            lines = await conn.iterdump()
            for line in lines:
                print(line)
    """

    def __init__(self, conn: "Connection") -> None:  # type: ignore[valid-type]
        self._conn = conn
        self._lines: Optional[List[str]] = None
        self._index: int = 0

    def __aiter__(self) -> "_IterdumpWrapper":
        return self

    async def __anext__(self) -> str:
        # Lazily fetch all lines once using the underlying raw iterdump.
        if self._lines is None:
            self._lines = await _raw_iterdump(self._conn)  # type: ignore[arg-type]
            self._index = 0

        if self._index >= len(self._lines):
            raise StopAsyncIteration

        line = self._lines[self._index]
        self._index += 1
        return line

    def __await__(self):
        async def _inner() -> List[str]:
            # Preserve existing semantics: await conn.iterdump() -> List[str]
            result = await _raw_iterdump(self._conn)  # type: ignore[arg-type]
            return result  # type: ignore[no-any-return]

        return _inner().__await__()


def _iterdump(self: "Connection") -> _IterdumpWrapper:  # type: ignore[valid-type]
    """Return a dual-mode iterdump wrapper.

    - async for line in conn.iterdump():  # async iterator
    - lines = await conn.iterdump()       # List[str], backwards compatible
    """
    return _IterdumpWrapper(self)


Connection._iterdump_raw = _raw_iterdump  # type: ignore[attr-defined]
Connection.iterdump = _iterdump  # type: ignore[assignment]


async def _execute_fetchall(
    self: "Connection",  # type: ignore[valid-type]
    sql: str,
    parameters: Optional[Any] = None,
) -> List[Any]:
    """Execute a SELECT and return all rows (aiosqlite-compatible helper)."""
    return await self.fetch_all(sql, parameters)  # type: ignore[union-attr]


Connection.execute_fetchall = _execute_fetchall  # type: ignore[attr-defined]


async def _explain_query_plan(
    self: "Connection",  # type: ignore[valid-type]
    sql: str,
    parameters: Optional[Any] = None,
) -> List[Any]:
    """Run EXPLAIN QUERY PLAN for the given SQL and return result rows (Phase 3.1)."""
    prepended = f"EXPLAIN QUERY PLAN {sql}"
    return await self.fetch_all(prepended, parameters)  # type: ignore[union-attr]


Connection.explain_query_plan = _explain_query_plan  # type: ignore[attr-defined]


async def _pool_health(self: "Connection") -> bool:  # type: ignore[valid-type]
    """Run a minimal health check (SELECT 1) and return True (Phase 3.2). Raises on failure."""
    await self.fetch_all("SELECT 1")  # type: ignore[union-attr]
    return True


Connection.pool_health = _pool_health  # type: ignore[attr-defined]

# aiosqlite uses executemany; we expose execute_many. Alias for compat.
Connection.executemany = Connection.execute_many  # type: ignore[attr-defined,assignment]


def _connection_await(self: "Connection"):  # type: ignore[valid-type]
    """Support `await conn` (aiosqlite-compatible). Enters connection and returns self."""

    async def _inner() -> "Connection":
        await self.__aenter__()  # type: ignore[union-attr]
        return self  # type: ignore[return-value]

    return _inner().__await__()


Connection.__await__ = _connection_await  # type: ignore[attr-defined]


async def _backup(
    self: "Connection",  # type: ignore[valid-type]
    target: Any,
    *,
    pages: int = 0,
    progress: Any = None,
    name: str = "main",
    sleep: float = 0.25,
) -> None:
    """Backup supporting both rapsqlite.Connection and sqlite3.Connection targets.

    This wrapper provides safe backup functionality for both rapsqlite and
    sqlite3 connection targets. For rapsqlite targets, it delegates to the
    original Rust implementation. For sqlite3.Connection targets, it uses
    Python's sqlite3 backup API on the on-disk database file, avoiding unsafe
    handle sharing between different SQLite library instances.

    Args:
        self: The source connection to backup from.
        target: Target connection. Can be a rapsqlite.Connection or
            sqlite3.Connection. For sqlite3 targets, only file-backed databases
            are supported (not :memory: or non-file URIs).
        pages: Number of pages to copy per step. If 0, copy all pages in one step.
            For large databases, use a positive value to allow progress callbacks.
        progress: Optional progress callback function. Called with
            (remaining, page_count, pages_copied) after each step.
        name: Database name to backup (default: "main").
        sleep: Sleep duration in seconds between backup steps when pages > 0.

    Raises:
        OperationalError: If backup fails, target has active transaction,
            or target is not a supported type.

    Note:
        For sqlite3.Connection targets, the source database must be file-backed.
        The backup operation performs a WAL checkpoint before backing up to
        ensure committed state is visible.
    """
    import sqlite3  # Local import to avoid mandatory dependency at import time.

    # sqlite3.Connection target: use file-based backup via sqlite3 API.
    if isinstance(target, sqlite3.Connection):
        import os

        # Get file path: prefer Connection.path (same file we opened), fallback to PRAGMA.
        conn_path = getattr(self, "path", None)
        if conn_path and conn_path != ":memory:" and (conn_path.strip() or "") != "":
            db_filename = os.path.abspath(conn_path)
        else:
            rows = await self.fetch_all("PRAGMA database_list")  # type: ignore[attr-defined]
            main_row = next((row for row in rows if row[1] == "main"), None)
            if not main_row or not main_row[2]:
                raise OperationalError(
                    "backup to sqlite3.Connection is only supported for file-backed "
                    "databases (got in-memory or unsupported URI)."
                )
            db_filename = main_row[2]
            db_filename = os.path.abspath(db_filename)

        # Best-effort flush of WAL to ensure committed state is visible on disk.
        try:
            await self.execute("PRAGMA wal_checkpoint(FULL)")  # type: ignore[attr-defined]
        except Exception:
            # Not all configurations use WAL; ignore failures here.
            pass

        # Disallow backup if target has an active transaction, matching previous
        # error semantics and sqlite3 best practices.
        if getattr(target, "in_transaction", False):
            raise OperationalError(
                "Cannot backup to sqlite3.Connection while it has an active transaction."
            )

        # Open a temporary sqlite3.Connection to the same file and run backup in the
        # same thread (sqlite3 connections must be used in the thread that created them;
        # target was created by the caller so we run backup here).
        source_sqlite3 = sqlite3.connect(db_filename)
        try:
            source_sqlite3.backup(
                target,
                pages=pages,
                progress=progress,
                name=name,
                sleep=sleep,
            )
        finally:
            source_sqlite3.close()
        return None

    # Fallback: rapsqlite-to-rapsqlite backup via the original Rust method.
    await _raw_backup(
        self,
        target,
        pages=pages,
        progress=progress,
        name=name,
        sleep=sleep,
    )
    return None


Connection._backup_raw = _raw_backup  # type: ignore[attr-defined]
Connection.backup = _backup  # type: ignore[assignment]
