"""aiosqlite compatibility patches for Connection and Cursor.

Patches: __del__, stop, set_progress_handler, cursor return self,
commit/rollback no-op, iterdump dual-mode, backup (sqlite3 vs rapsqlite),
__await__, execute_fetchall, explain_query_plan, pool_health, executemany alias,
slow query threshold, execute_iter. Call apply_compat(Connection, Cursor, operational_error).
"""

from __future__ import annotations

import asyncio
import os
import time
from collections.abc import Callable
from typing import Any, TYPE_CHECKING

from rapsqlite._connection_state import _cleanup_conn_state

if TYPE_CHECKING:
    pass

# Module-level refs set in apply_compat
_orig_set_progress_handler: Any = None
_orig_cursor_execute: Any = None
_orig_cursor_executemany: Any = None
_orig_cursor_executescript: Any = None
_orig_commit: Any = None
_orig_rollback: Any = None
_raw_iterdump: Any = None
_raw_backup: Any = None
_orig_connection_fetch_all: Any = None
_slow_query_state: dict[int, tuple[float, Callable[[float, str], None] | None]] = {}


def _connection_del(self: Any) -> None:
    """Best-effort: schedule close() on the running event loop to avoid Tokio panic during GC."""
    _cleanup_conn_state(self)
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return
    try:
        loop.call_soon(lambda: asyncio.ensure_future(self.close()))
    except Exception:
        pass


async def _set_progress_handler_wrapper(
    self: Any, a: Any, b: Any = None
) -> Any:
    if b is not None and callable(a) and isinstance(b, int):
        n, callback = b, a
    else:
        n, callback = a, b
    return await _orig_set_progress_handler(self, n, callback)


async def _cursor_execute_return_self(
    self: Any, query: str, parameters: Any = None
) -> Any:
    await _orig_cursor_execute(self, query, parameters)
    return self


async def _cursor_executemany_return_self(
    self: Any, query: str, parameters: Any
) -> Any:
    await _orig_cursor_executemany(self, query, parameters)
    return self


async def _cursor_executescript_return_self(self: Any, script: str) -> Any:
    await _orig_cursor_executescript(self, script)
    return self


async def _commit_noop_on_no_tx(self: Any, OperationalError: type) -> None:
    try:
        await _orig_commit(self)
    except OperationalError as e:
        msg = str(e).lower()
        if "transaction" in msg and (
            "not available" in msg or "in progress" in msg or "no transaction" in msg
        ):
            return
        raise


async def _rollback_noop_on_no_tx(self: Any, OperationalError: type) -> None:
    try:
        await _orig_rollback(self)
    except OperationalError as e:
        msg = str(e).lower()
        if "transaction" in msg and (
            "not available" in msg or "in progress" in msg or "no transaction" in msg
        ):
            return
        raise


class _IterdumpWrapper:
    """Dual-mode wrapper for iterdump: async-iter and await-to-list."""

    def __init__(self, conn: Any) -> None:
        self._conn = conn
        self._lines: list[str] | None = None
        self._index: int = 0

    def __aiter__(self) -> _IterdumpWrapper:
        return self

    async def __anext__(self) -> str:
        if self._lines is None:
            self._lines = await _raw_iterdump(self._conn)
            self._index = 0
        if self._index >= len(self._lines):
            raise StopAsyncIteration
        line = self._lines[self._index]
        self._index += 1
        return line

    def __await__(self) -> Any:
        async def _inner() -> list[str]:
            return await _raw_iterdump(self._conn)
        return _inner().__await__()


def _iterdump(self: Any) -> _IterdumpWrapper:
    """Return a dual-mode iterdump wrapper."""
    return _IterdumpWrapper(self)


async def _execute_fetchall(
    self: Any, sql: str, parameters: Any | None = None
) -> list[list[Any]]:
    return await self.fetch_all(sql, parameters)


async def _explain_query_plan(
    self: Any, sql: str, parameters: Any | None = None
) -> list[list[Any]]:
    return await self.fetch_all(f"EXPLAIN QUERY PLAN {sql}", parameters)


async def _pool_health(self: Any) -> bool:
    await self.fetch_all("SELECT 1")
    return True


def set_slow_query_threshold(
    self: Any,
    threshold_secs: float,
    callback: Callable[[float, str], None] | None = None,
) -> None:
    """Set threshold and optional callback for slow query detection."""
    cid = id(self)
    if threshold_secs <= 0:
        _slow_query_state.pop(cid, None)
    else:
        _slow_query_state[cid] = (threshold_secs, callback)


async def _fetch_all_with_slow_check(
    self: Any, query: str, parameters: Any = None
) -> list[list[Any]]:
    state = _slow_query_state.get(id(self), (0, None))
    threshold, cb = state[0], state[1]
    if threshold <= 0:
        return await _orig_connection_fetch_all(self, query, parameters)
    t0 = time.perf_counter()
    try:
        return await _orig_connection_fetch_all(self, query, parameters)
    finally:
        duration = time.perf_counter() - t0
        if duration >= threshold and cb:
            cb(duration, query)


def _connection_await(self: Any) -> Any:
    """Support await conn (aiosqlite-compatible)."""

    async def _inner() -> Any:
        await self.__aenter__()
        return self
    return _inner().__await__()


async def _backup_impl(
    self: Any,
    target: Any,
    *,
    pages: int = 0,
    progress: Any = None,
    name: str = "main",
    sleep: float = 0.25,
    operational_error: type = None,
) -> None:
    """Backup supporting both rapsqlite.Connection and sqlite3.Connection targets."""
    import sqlite3
    if isinstance(target, sqlite3.Connection):
        conn_path = getattr(self, "path", None)
        if conn_path and conn_path != ":memory:" and (conn_path.strip() or "") != "":
            db_filename = os.path.abspath(conn_path)
        else:
            rows = await self.fetch_all("PRAGMA database_list")
            main_row = next((row for row in rows if row[1] == "main"), None)
            if not main_row or not main_row[2]:
                raise operational_error(
                    "backup to sqlite3.Connection is only supported for file-backed "
                    "databases (got in-memory or unsupported URI)."
                )
            db_filename = os.path.abspath(main_row[2])
        try:
            await self.execute("PRAGMA wal_checkpoint(FULL)")
        except Exception:
            pass
        if getattr(target, "in_transaction", False):
            raise operational_error(
                "Cannot backup to sqlite3.Connection while it has an active transaction."
            )
        source_sqlite3 = sqlite3.connect(db_filename)
        try:
            source_sqlite3.backup(
                target, pages=pages, progress=progress, name=name, sleep=sleep
            )
        finally:
            source_sqlite3.close()
        return None
    await _raw_backup(self, target, pages=pages, progress=progress, name=name, sleep=sleep)
    return None


def apply_compat(
    Connection: type,
    Cursor: type,
    *,
    operational_error: type,
) -> None:
    """Attach all aiosqlite compatibility patches to Connection and Cursor.

    Call with the extension Connection and Cursor classes and the
    OperationalError exception class (from the extension or rapsqlite).
    """
    global _orig_set_progress_handler, _orig_cursor_execute, _orig_cursor_executemany
    global _orig_cursor_executescript, _orig_commit, _orig_rollback
    global _raw_iterdump, _raw_backup, _orig_connection_fetch_all

    Connection.__del__ = _connection_del  # type: ignore[assignment]
    if not hasattr(Connection, "stop"):
        def _stop_noop(self: Any) -> None:
            pass
        Connection.stop = _stop_noop  # type: ignore[assignment]

    _orig_set_progress_handler = Connection.set_progress_handler
    Connection.set_progress_handler = _set_progress_handler_wrapper  # type: ignore[assignment]

    _orig_cursor_execute = Cursor.execute
    _orig_cursor_executemany = Cursor.executemany
    _orig_cursor_executescript = Cursor.executescript
    Cursor.execute = _cursor_execute_return_self  # type: ignore[assignment]
    Cursor.executemany = _cursor_executemany_return_self  # type: ignore[assignment]
    Cursor.executescript = _cursor_executescript_return_self  # type: ignore[assignment]

    _orig_commit = Connection.commit
    _orig_rollback = Connection.rollback
    _commit_noop = lambda self: _commit_noop_on_no_tx(self, operational_error)
    _rollback_noop = lambda self: _rollback_noop_on_no_tx(self, operational_error)
    Connection.commit = _commit_noop  # type: ignore[assignment]
    Connection.rollback = _rollback_noop  # type: ignore[assignment]

    _raw_iterdump = Connection.iterdump
    _raw_backup = Connection.backup
    Connection._iterdump_raw = _raw_iterdump  # type: ignore[attr-defined]
    Connection.iterdump = _iterdump  # type: ignore[assignment]
    Connection.execute_fetchall = _execute_fetchall  # type: ignore[attr-defined]
    Connection.explain_query_plan = _explain_query_plan  # type: ignore[attr-defined]
    Connection.pool_health = _pool_health  # type: ignore[attr-defined]
    Connection.executemany = Connection.execute_many  # type: ignore[attr-defined,assignment]

    _orig_connection_fetch_all = Connection.fetch_all
    Connection.set_slow_query_threshold = set_slow_query_threshold  # type: ignore[attr-defined,assignment]
    Connection.fetch_all = _fetch_all_with_slow_check  # type: ignore[assignment]
    Connection.__await__ = _connection_await  # type: ignore[attr-defined]

    async def _backup_wrapper(
        self: Any, target: Any, *, pages: int = 0, progress: Any = None,
        name: str = "main", sleep: float = 0.25
    ) -> None:
        await _backup_impl(
            self, target, pages=pages, progress=progress, name=name, sleep=sleep,
            operational_error=operational_error,
        )
    Connection._backup_raw = _raw_backup  # type: ignore[attr-defined]
    Connection.backup = _backup_wrapper  # type: ignore[assignment]
