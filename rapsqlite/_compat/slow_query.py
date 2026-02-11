"""Compat for slow query threshold and __await__ on Connection."""

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Any, cast

_orig_connection_fetch_all: Any = None
_slow_query_state: dict[int, tuple[float, Callable[[float, str], None] | None]] = {}


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
        return cast(
            list[list[Any]], await _orig_connection_fetch_all(self, query, parameters)
        )
    t0 = time.perf_counter()
    try:
        return cast(
            list[list[Any]],
            await _orig_connection_fetch_all(self, query, parameters),
        )
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


def apply_slow_query_and_await_compat(Connection: type) -> None:
    """Compat for slow query tracking and __await__."""
    global _orig_connection_fetch_all

    _orig_connection_fetch_all = Connection.fetch_all  # type: ignore[attr-defined]
    Connection.set_slow_query_threshold = set_slow_query_threshold  # type: ignore[attr-defined]
    Connection.fetch_all = _fetch_all_with_slow_check  # type: ignore[attr-defined]
    Connection.__await__ = _connection_await  # type: ignore[attr-defined]
