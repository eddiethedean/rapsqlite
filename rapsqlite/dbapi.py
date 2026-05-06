"""
True Async DBAPI 2.0-compliant module for SQLAlchemy and other consumers.
See docs/true_async_dbapi_spec.md for the interface contract.
"""

from __future__ import annotations  # PEP 563: forward references without quotes

import asyncio
import re
from typing import Any, Callable, Coroutine, Iterable, Sequence, TypeVar, cast

from . import (
    ConnectionT,
    Cursor as _Cursor,
    CursorT,
    Error,
    InterfaceError,
)
from . import connect as _connect
from ._compat.commit_rollback import _is_no_tx_error_message

T = TypeVar("T")


def _parse_select_column_names(sql: str) -> list[str] | None:
    """Parse column names from SELECT ... FROM for 0-row description fallback."""
    q = sql.strip()
    if not re.match(r"^\s*SELECT\s+", q, re.IGNORECASE):
        return None
    m = re.search(r"\s+FROM\s+", q, re.IGNORECASE)
    if not m:
        return None
    select_list = q[6 : m.start()].strip()
    if not select_list or select_list == "*":
        return None
    names = [part.strip() for part in select_list.split(",")]
    if not all(names):
        return None
    return names


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


# DBAPI exception list: (name, base). Each is exposed as a module-level name.
_DBAPI_EXCEPTIONS = [
    ("DataError", DatabaseError),
    ("OperationalError", DatabaseError),
    ("IntegrityError", DatabaseError),
    ("InternalError", DatabaseError),
    ("NotSupportedError", DatabaseError),
    ("ProgrammingError", DatabaseError),
]
for _name, _base in _DBAPI_EXCEPTIONS:
    _ext_cls = getattr(_ext, _name, type(_name, (DatabaseError,), {}))
    globals()[_name] = _dbapi_exc(_name, _ext_cls, _base)

# For mypy: names are set by loop above
ProgrammingError = globals()["ProgrammingError"]
OperationalError = globals()["OperationalError"]

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
    """DBAPI cursor wrapper. Serializes operations via connection lock.

    Buffers result rows after execute() so that fetchone()/fetchall() can be
    called later (e.g. by SQLAlchemy's async adapter) without requiring
    another async round-trip to the DB, which can run in a context where
    the driver's runtime (e.g. Tokio) is not available.
    """

    __slots__ = (
        "_conn",
        "_raw",
        "_result_buffer",
        "_result_index",
        "_cached_description",
    )

    def __init__(self, conn: "AsyncConnection", raw: CursorT) -> None:
        self._conn = conn
        self._raw = raw
        self._result_buffer: list[Any] | None = None
        self._result_index: int = 0
        self._cached_description: Any = None

    def _with_lock(
        self, coro_factory: Callable[[], Coroutine[Any, Any, T]]
    ) -> Coroutine[Any, Any, T]:
        """Run coro_factory() while holding connection op lock; raise ProgrammingError if busy."""

        async def _run() -> T:
            # Fail fast for true concurrent use, without using tiny timeouts that can
            # spuriously trip under scheduler load.
            if self._conn._op_lock.locked():
                raise ProgrammingError(
                    "Concurrent operation on same connection not allowed; "
                    "one operation per connection at a time."
                )
            await self._conn._op_lock.acquire()
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
        # Use cached description when set so it remains available after cursor close
        # (SQLAlchemy may close the cursor before consuming the result).
        if self._cached_description is not None:
            return self._cached_description
        return self._raw.description

    @property
    def rowcount(self) -> int:
        return cast(int, self._raw.rowcount)

    @property
    def lastrowid(self) -> int:
        return cast(int, self._raw.lastrowid)

    @property
    def arraysize(self) -> int:
        return cast(int, self._raw.arraysize)

    @arraysize.setter
    def arraysize(self, value: int) -> None:
        self._raw.arraysize = value

    async def execute(self, sql: str, params: Any = None) -> None:
        async def _do() -> None:
            self._result_buffer = None
            self._result_index = 0
            self._cached_description = None
            await self._raw.execute(sql, params)
            # Capture description immediately after execute (Rust sets pending_description
            # in __aenter__; fetchall() later moves it to description, so read before fetchall
            # so 0-row SELECT has description for SQLAlchemy).
            self._cached_description = self._raw.description
            # Do not eagerly buffer all rows here: callers may execute large SELECTs and
            # expect fetch* to stream. We only ensure `description` is available for
            # SQLAlchemy's 0-row SELECT metadata cases.
            if self._cached_description is None:
                names = _parse_select_column_names(sql)
                if names:
                    self._cached_description = tuple(
                        (name, None, None, None, None, None, None) for name in names
                    )

        return await self._with_lock(_do)

    async def executemany(
        self, sql: str, seq_of_params: Iterable[Sequence[Any]]
    ) -> None:
        async def _do() -> None:
            self._result_buffer = None
            self._result_index = 0
            self._cached_description = None
            await self._raw.executemany(sql, seq_of_params)
            self._cached_description = self._raw.description

        await self._with_lock(_do)

    async def fetchone(self) -> Any:
        if self._result_buffer is not None:
            if self._result_index < len(self._result_buffer):
                row = self._result_buffer[self._result_index]
                self._result_index += 1
                return row
            return None
        return await self._with_lock(self._raw.fetchone)

    async def fetchmany(self, size: int | None = None) -> list[Any]:
        if self._result_buffer is not None:
            remaining = len(self._result_buffer) - self._result_index
            n = remaining if size is None else min(size, remaining)
            chunk = self._result_buffer[self._result_index : self._result_index + n]
            self._result_index += len(chunk)
            return chunk

        async def _do() -> list[Any]:
            return await self._raw.fetchmany(size)  # type: ignore[no-any-return]

        return await self._with_lock(_do)

    async def fetchall(self) -> list[Any]:
        if self._result_buffer is not None:
            rest = self._result_buffer[self._result_index :]
            self._result_index = len(self._result_buffer)
            return rest
        return await self._with_lock(self._raw.fetchall)

    async def close(self) -> None:
        # Keep _result_buffer so fetchone/fetchall can still be called after close()
        # (e.g. SQLAlchemy may close the cursor before consuming the result).
        self._result_index = 0

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

    def __init__(self, conn: ConnectionT) -> None:
        self._conn = conn
        self._closed = False
        self._op_lock = asyncio.Lock()

    async def _with_op_lock(
        self, coro_factory: Callable[[], Coroutine[Any, Any, T]]
    ) -> T:
        """Run coro_factory() while holding op lock; raise ProgrammingError if busy."""
        # Fail fast for true concurrent use, without using tiny timeouts that can
        # spuriously trip under scheduler load.
        if self._op_lock.locked():
            raise ProgrammingError(
                "Concurrent operation on same connection not allowed; "
                "one operation per connection at a time."
            )
        await self._op_lock.acquire()
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

    async def begin(self) -> None:
        """Start a transaction (for SQLAlchemy engine.begin() etc.)."""
        await self._conn.begin()

    async def commit(self) -> None:
        try:
            await self._conn.commit()
        except OperationalError as e:
            # DBAPI compat: commit() is a no-op when not in a transaction.
            if _is_no_tx_error_message(str(e)):
                return
            raise

    async def rollback(self) -> None:
        try:
            await self._conn.rollback()
        except OperationalError as e:
            # DBAPI compat: rollback() is a no-op when not in a transaction.
            if _is_no_tx_error_message(str(e)):
                return
            raise

    async def create_function(
        self,
        name: str,
        nargs: int,
        func: Any = None,
        *,
        deterministic: bool = False,
    ) -> None:
        """Create or remove a user-defined SQL function (DBAPI/SQLAlchemy on_connect compat).

        Delegates to the underlying rapsqlite Connection. Pass func=None to remove.
        """
        await self._with_op_lock(
            lambda: self._conn.create_function(
                name, nargs, func, deterministic=deterministic
            )
        )

    async def close(self) -> None:
        if self._closed:
            return
        # Route explicit close() through the same path as context manager exit
        # so that the underlying connection's __aexit__ is always invoked once.
        await self.__aexit__(None, None, None)


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
    await conn.__aenter__()
    return AsyncConnection(conn)


# Alias for DBAPI consumers that expect Connection
Connection = AsyncConnection

# Exception names are set dynamically in the loop above
__all__ = [  # noqa: F822
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
