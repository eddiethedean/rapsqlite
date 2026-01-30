"""
True Async DBAPI 2.0-compliant module for SQLAlchemy and other consumers.
See docs/true_async_dbapi_spec.md for the interface contract.
"""

from __future__ import annotations  # PEP 563: forward references without quotes

import asyncio
from typing import Any

from . import (
    Connection as _Connection,
    Cursor as _Cursor,
    Error,
    InterfaceError,
)
from . import connect as _connect

# DBAPI exception hierarchy: ensure OperationalError etc. are subclasses of DatabaseError
# (pyo3 create_exception can expose types where issubclass(OperationalError, DatabaseError) is False)
try:
    import _rapsqlite as _ext
except ImportError:
    _parent = __import__(__name__.rsplit(".", 1)[0], fromlist=["Connection"])
    _ext = getattr(_parent, "_ext", None)
    if _ext is None:
        raise ImportError("rapsqlite extension (_rapsqlite) not found") from None
DatabaseError = _ext.DatabaseError

def _dbapi_exc(name: str, ext_cls: Any, base: type) -> type:
    """Ensure exception class is a subclass of base (DBAPI hierarchy)."""
    return type(name, (ext_cls, base), {})

DataError = _dbapi_exc("DataError", getattr(_ext, "DataError", type("DataError", (DatabaseError,), {})), DatabaseError)
OperationalError = _dbapi_exc("OperationalError", getattr(_ext, "OperationalError", type("OperationalError", (DatabaseError,), {})), DatabaseError)
IntegrityError = _dbapi_exc("IntegrityError", getattr(_ext, "IntegrityError", type("IntegrityError", (DatabaseError,), {})), DatabaseError)
InternalError = _dbapi_exc("InternalError", getattr(_ext, "InternalError", type("InternalError", (DatabaseError,), {})), DatabaseError)
NotSupportedError = _dbapi_exc("NotSupportedError", getattr(_ext, "NotSupportedError", type("NotSupportedError", (DatabaseError,), {})), DatabaseError)
ProgrammingError = _dbapi_exc("ProgrammingError", getattr(_ext, "ProgrammingError", type("ProgrammingError", (DatabaseError,), {})), DatabaseError)

apilevel = "2.0"
threadsafety = 0
paramstyle = "qmark"

_ALLOWED_CONNECT_KWARGS = frozenset(("pragmas", "iter_chunk_size", "loop"))


class _CursorContextManager:
    """Context manager returned by AsyncConnection.cursor() for SQLAlchemy etc.

    SQLAlchemy expects connection.cursor() to return an object with __aenter__,
    not a coroutine. This wrapper allows:
    - ``async with conn.cursor() as cur:`` (SQLAlchemy)
    - ``cur = await conn.cursor()`` (direct use)
    """

    __slots__ = ("_conn",)

    def __init__(self, conn: "AsyncConnection") -> None:
        self._conn = conn

    def __await__(self) -> Any:
        """Allow await conn.cursor() to return the cursor directly."""
        return self._conn._cursor_impl().__await__()

    async def __aenter__(self) -> "AsyncCursor":
        return await self._conn._cursor_impl()

    async def __aexit__(self, *args: Any) -> None:
        pass


class AsyncCursor:
    """DBAPI cursor wrapper. Serializes operations via connection lock."""

    def __init__(self, conn: "AsyncConnection", raw: _Cursor) -> None:
        self._conn = conn
        self._raw = raw

    def _with_lock(self, coro_factory):
        """Run coro_factory() while holding connection op lock; raise ProgrammingError if busy."""

        async def _run() -> Any:
            try:
                await asyncio.wait_for(self._conn._op_lock.acquire(), timeout=1e-6)
            except asyncio.TimeoutError:
                raise ProgrammingError(
                    "Concurrent operation on same connection not allowed; "
                    "one operation per connection at a time."
                )
            try:
                return await coro_factory()
            except asyncio.CancelledError:
                await self._conn._conn.interrupt()
                raise
            finally:
                self._conn._op_lock.release()

        return _run()

    @property
    def description(self) -> Any:
        return self._raw.description

    @property
    def rowcount(self) -> int:
        return self._raw.rowcount  # type: ignore[no-any-return]

    @property
    def lastrowid(self) -> int:
        return self._raw.lastrowid  # type: ignore[no-any-return]

    @property
    def arraysize(self) -> int:
        return self._raw.arraysize  # type: ignore[no-any-return]

    @arraysize.setter
    def arraysize(self, value: int) -> None:
        self._raw.arraysize = value

    async def execute(self, sql: str, params: Any = None) -> None:
        async def _do() -> None:
            return await self._raw.execute(sql, params)  # type: ignore[no-any-return]

        return await self._with_lock(_do)  # type: ignore[no-any-return]

    async def executemany(self, sql: str, seq_of_params: Any) -> None:
        async def _do() -> None:
            await self._raw.executemany(sql, seq_of_params)

        await self._with_lock(_do)  # type: ignore[no-any-return]

    async def fetchone(self) -> Any:
        return await self._with_lock(self._raw.fetchone)  # type: ignore[no-any-return]

    async def fetchmany(self, size: int | None = None) -> list[Any]:
        async def _do() -> list[Any]:
            return await self._raw.fetchmany(size)  # type: ignore[no-any-return]

        return await self._with_lock(_do)  # type: ignore[no-any-return]

    async def fetchall(self) -> list[Any]:
        return await self._with_lock(self._raw.fetchall)  # type: ignore[no-any-return]

    async def close(self) -> None:
        async def _do() -> None:
            await self._raw.close()

        await self._with_lock(_do)

    def __aiter__(self) -> AsyncCursor:
        return self

    async def __aenter__(self) -> AsyncCursor:
        return self

    async def __aexit__(self, *args: Any) -> None:
        pass

    async def __anext__(self) -> Any:
        async def _do() -> Any:
            return await self._raw.__anext__()

        return await self._with_lock(_do)


class AsyncConnection:
    """
    DBAPI AsyncConnection wrapper. Provides async cursor() and delegates
    execute, executemany, commit, rollback, close to the underlying Connection.
    Supports ``async with (await connect(...)) as conn``; ``close()`` performs
    deterministic cleanup when not using a context manager.
    One operation per connection at a time; concurrent use raises ProgrammingError.
    """

    def __init__(self, conn: _Connection) -> None:
        self._conn = conn
        self._closed = False
        self._op_lock = asyncio.Lock()

    async def _with_op_lock(self, coro_factory) -> Any:
        """Run coro_factory() while holding op lock; raise ProgrammingError if busy."""
        try:
            await asyncio.wait_for(self._op_lock.acquire(), timeout=1e-6)
        except asyncio.TimeoutError:
            raise ProgrammingError(
                "Concurrent operation on same connection not allowed; "
                "one operation per connection at a time."
            )
        try:
            return await coro_factory()
        except asyncio.CancelledError:
            await self._conn.interrupt()
            raise
        finally:
            self._op_lock.release()

    async def __aenter__(self) -> AsyncConnection:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: Any,
    ) -> None:
        if self._closed:
            return
        await self._conn.__aexit__(exc_type, exc_val, exc_tb)
        await self._conn.close()
        self._closed = True

    def cursor(self) -> _CursorContextManager:
        """Return a context manager that yields the cursor (SQLAlchemy compat).

        Use: ``async with conn.cursor() as cur:`` or ``cur = await (await conn.cursor()).__aenter__()``.
        """
        return _CursorContextManager(self)

    async def _cursor_impl(self) -> AsyncCursor:
        """Actual cursor acquisition (used by _CursorContextManager.__aenter__)."""
        async def _do() -> AsyncCursor:
            raw = self._conn.cursor()
            return AsyncCursor(self, raw)  # type: ignore[no-any-return]

        return await self._with_op_lock(_do)  # type: ignore[no-any-return]

    async def execute(self, sql: str, params: Any = None) -> AsyncCursor:
        async def _do() -> AsyncCursor:
            raw = await self._conn.execute(sql, params)
            return AsyncCursor(self, raw)  # type: ignore[no-any-return]

        return await self._with_op_lock(_do)  # type: ignore[no-any-return]

    async def executemany(self, sql: str, seq_of_params: Any) -> None:
        async def _do() -> None:
            await self._conn.executemany(sql, seq_of_params)

        await self._with_op_lock(_do)

    async def begin(self) -> None:
        """Start a transaction (for SQLAlchemy engine.begin() etc.)."""
        await self._conn.begin()

    async def commit(self) -> None:
        try:
            await self._conn.commit()
        except OperationalError as e:
            # DBAPI compat: commit() is a no-op when not in a transaction
            msg = str(e).lower()
            if "transaction" in msg and (
                "not available" in msg or "in progress" in msg or "no transaction" in msg
            ):
                return
            raise

    async def rollback(self) -> None:
        try:
            await self._conn.rollback()
        except OperationalError as e:
            # DBAPI compat: rollback() is a no-op when not in a transaction
            msg = str(e).lower()
            if "transaction" in msg and (
                "not available" in msg or "in progress" in msg or "no transaction" in msg
            ):
                return
            raise

    async def close(self) -> None:
        if self._closed:
            return
        await self._conn.close()
        self._closed = True


# Re-export raw Cursor for type hints / consumers that need it
Cursor = _Cursor


async def connect(*args: Any, **kwargs: Any) -> AsyncConnection:
    """
    Async connect per True Async DBAPI spec.

    Signature: ``connect(database)`` or ``connect(database, **kwargs)``.
    ``database`` is the DB path (or ``:memory:``). For SQLAlchemy, this is
    typically the URL database segment.

    Supported kwargs: ``timeout`` (default 5.0), ``pragmas``, ``iter_chunk_size``,
    ``loop`` (ignored). Others are ignored.
    """
    database: Any = None
    if args:
        database = args[0]
        args = args[1:]
    if database is None:
        database = kwargs.pop("database", None)
    if database is None:
        raise InterfaceError(
            "connect() requires database as first positional or as keyword 'database'"
        )
    if args:
        raise InterfaceError(
            "connect() takes at most one positional argument (database)"
        )

    timeout: float = float(kwargs.pop("timeout", 5.0))
    opts = {k: v for k, v in kwargs.items() if k in _ALLOWED_CONNECT_KWARGS}
    conn = _connect(str(database), timeout=timeout, **opts)
    await conn.__aenter__()  # type: ignore[attr-defined]
    return AsyncConnection(conn)


# Alias for DBAPI consumers that expect Connection
Connection = AsyncConnection

__all__ = [
    "apilevel",
    "threadsafety",
    "paramstyle",
    "connect",
    "Connection",
    "AsyncConnection",
    "AsyncCursor",
    "Cursor",
    "Error",
    "InterfaceError",
    "DatabaseError",
    "DataError",
    "OperationalError",
    "IntegrityError",
    "InternalError",
    "ProgrammingError",
    "NotSupportedError",
]
