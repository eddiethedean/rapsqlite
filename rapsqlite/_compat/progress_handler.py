"""Compat for set_progress_handler accepting (n, callback) or (callback, n)."""

from __future__ import annotations

from typing import Any

_orig_set_progress_handler: Any = None


async def _set_progress_handler_wrapper(self: Any, a: Any, b: Any = None) -> Any:
    if b is not None and callable(a) and isinstance(b, int):
        n, callback = b, a
    else:
        n, callback = a, b
    return await _orig_set_progress_handler(self, n, callback)


def apply_progress_handler_compat(Connection: type) -> None:
    """Compat for set_progress_handler accepting (n, callback) or (callback, n)."""
    global _orig_set_progress_handler

    _orig_set_progress_handler = Connection.set_progress_handler  # type: ignore[attr-defined]
    Connection.set_progress_handler = _set_progress_handler_wrapper  # type: ignore[attr-defined]
