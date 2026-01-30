"""
True Async DBAPI 2.0-compliant module for SQLAlchemy and other consumers.
See docs/true_async_dbapi_spec.md for the interface contract.
"""

from __future__ import annotations

import asyncio
from typing import Any

from . import (
    Connection as _Connection,
    Cursor as _Cursor,
    DataError,
    DatabaseError,
    Error,
    IntegrityError,
    InternalError,
    InterfaceError,
    NotSupportedError,
    OperationalError,
    ProgrammingError,
)
from . import connect as _connect

apilevel = "2.0"
threadsafety = 0
paramstyle = "qmark"

_ALLOWED_CONNECT_KWARGS = frozenset(("pragmas", "iter_chunk_size", "loop"))


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
        return self._raw.rowcount

    @property
    def lastrowid(self) -> int:
        return getattr(self._raw, "lastrowid", -1)

    @property
    def arraysize(self) -> int:
        return self._raw.arraysize

    @arraysize.setter
    def arraysize(self, value: int) -> None:
        self._raw.arraysize = value

    async def execute(self, sql: str, params: Any = None) -> None:
        async def _do() -> None:
            return await self._raw.execute(sql, params)

        return await self._with_lock(_do)

    async def executemany(self, sql: str, seq_of_params: Any) -> None:
        async def _do() -> None:
            await self._raw.executemany(sql, seq_of_params)

        await self._with_lock(_do)

    async def fetchone(self) -> Any:
        return await self._with_lock(self._raw.fetchone)

    async def fetchmany(self, size: int | None = None) -> list[Any]:
        async def _do() -> list[Any]:
            return await self._raw.fetchmany(size)

        return await self._with_lock(_do)

    async def fetchall(self) -> list[Any]:
        return await self._with_lock(self._raw.fetchall)

    async def close(self) -> None:
        async def _do() -> None:
            close_fn = getattr(self._raw, "close", None)
            if callable(close_fn):
                await close_fn()
            # else: no-op when raw Cursor lacks close (e.g. older build)

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

    async def cursor(self) -> AsyncCursor:
        async def _do() -> AsyncCursor:
            raw = self._conn.cursor()
            return AsyncCursor(self, raw)

        return await self._with_op_lock(_do)

    async def execute(self, sql: str, params: Any = None) -> AsyncCursor:
        async def _do() -> AsyncCursor:
            raw = await self._conn.execute(sql, params)
            return AsyncCursor(self, raw)

        return await self._with_op_lock(_do)

    async def executemany(self, sql: str, seq_of_params: Any) -> None:
        async def _do() -> None:
            await self._conn.executemany(sql, seq_of_params)

        await self._with_op_lock(_do)

    async def commit(self) -> None:
        await self._conn.commit()

    async def rollback(self) -> None:
        await self._conn.rollback()

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
        raise InterfaceError("connect() requires database as first positional or as keyword 'database'")
    if args:
        raise InterfaceError("connect() takes at most one positional argument (database)")

    timeout: float = float(kwargs.pop("timeout", 5.0))
    opts = {k: v for k, v in kwargs.items() if k in _ALLOWED_CONNECT_KWARGS}
    conn = _connect(str(database), timeout=timeout, **opts)
    await conn.__aenter__()
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
