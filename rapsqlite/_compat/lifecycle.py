"""Lifecycle-related compat: __del__ and stop()."""

from __future__ import annotations

import asyncio
from typing import Any

from rapsqlite._connection_state import _cleanup_conn_state


def _connection_del(self: Any) -> None:
    """Best-effort: schedule close() on the running event loop to avoid cleanup issues during GC."""
    _cleanup_conn_state(self)
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return
    try:
        loop.call_soon(lambda: asyncio.ensure_future(self.close()))
    except Exception:
        pass


def apply_lifecycle_compat(Connection: type) -> None:
    """Lifecycle-related compat: __del__ and stop()."""
    Connection.__del__ = _connection_del  # type: ignore[attr-defined]
    if not hasattr(Connection, "stop"):

        def _stop_noop(self: Any) -> None:
            pass

        Connection.stop = _stop_noop  # type: ignore[attr-defined]
