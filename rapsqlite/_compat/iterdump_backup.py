"""Compat for iterdump (dual-mode) and backup (sqlite3 vs rapsqlite targets)."""

from __future__ import annotations

import os
from typing import Any, Type, cast

_raw_iterdump: Any = None
_raw_backup: Any = None


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
            return cast(list[str], await _raw_iterdump(self._conn))

        return _inner().__await__()


def _iterdump(self: Any) -> _IterdumpWrapper:
    """Return a dual-mode iterdump wrapper."""
    return _IterdumpWrapper(self)


async def _backup_impl(
    self: Any,
    target: Any,
    *,
    pages: int = 0,
    progress: Any = None,
    name: str = "main",
    sleep: float = 0.25,
    operational_error: Type[BaseException] | None = None,
) -> None:
    """Backup supporting both rapsqlite.Connection and sqlite3.Connection targets."""
    import sqlite3

    if operational_error is None:
        raise RuntimeError("operational_error is required for backup_impl")
    op_err = operational_error

    if isinstance(target, sqlite3.Connection):
        conn_path = getattr(self, "path", None)
        if conn_path and conn_path != ":memory:" and (conn_path.strip() or "") != "":
            db_filename = os.path.abspath(conn_path)
        else:
            rows = await self.fetch_all("PRAGMA database_list")
            main_row = next((row for row in rows if row[1] == "main"), None)
            if not main_row or not main_row[2]:
                raise op_err(
                    "backup to sqlite3.Connection is only supported for file-backed "
                    "databases (got in-memory or unsupported URI)."
                )
            db_filename = os.path.abspath(main_row[2])
        try:
            await self.execute("PRAGMA wal_checkpoint(FULL)")
        except Exception:
            pass
        if getattr(target, "in_transaction", False):
            raise op_err(
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
    await _raw_backup(
        self, target, pages=pages, progress=progress, name=name, sleep=sleep
    )
    return None


def apply_iterdump_and_backup_compat(
    Connection: type,
    operational_error: Type[BaseException],
) -> None:
    """Compat for iterdump dual-mode and backup wrapper (sqlite3 + rapsqlite targets)."""
    global _raw_iterdump, _raw_backup

    _raw_iterdump = Connection.iterdump  # type: ignore[attr-defined]
    _raw_backup = Connection.backup  # type: ignore[attr-defined]
    Connection._iterdump_raw = _raw_iterdump  # type: ignore[attr-defined]
    Connection.iterdump = _iterdump  # type: ignore[attr-defined]
    Connection._backup_raw = _raw_backup  # type: ignore[attr-defined]

    async def _backup_wrapper(
        self: Any,
        target: Any,
        *,
        pages: int = 0,
        progress: Any = None,
        name: str = "main",
        sleep: float = 0.25,
    ) -> None:
        await _backup_impl(
            self,
            target,
            pages=pages,
            progress=progress,
            name=name,
            sleep=sleep,
            operational_error=operational_error,
        )

    Connection.backup = _backup_wrapper  # type: ignore[attr-defined]
