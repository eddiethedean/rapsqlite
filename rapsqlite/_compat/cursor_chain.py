"""Compat so cursor execute/executemany/executescript return self."""

from __future__ import annotations

from typing import Any

_orig_cursor_execute: Any = None
_orig_cursor_executemany: Any = None
_orig_cursor_executescript: Any = None


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


def apply_cursor_chaining_compat(Cursor: type) -> None:
    """Compat so cursor execute/executemany/executescript return self."""
    global _orig_cursor_execute, _orig_cursor_executemany, _orig_cursor_executescript

    _orig_cursor_execute = Cursor.execute  # type: ignore[attr-defined]
    _orig_cursor_executemany = Cursor.executemany  # type: ignore[attr-defined]
    _orig_cursor_executescript = Cursor.executescript  # type: ignore[attr-defined]
    Cursor.execute = _cursor_execute_return_self  # type: ignore[attr-defined]
    Cursor.executemany = _cursor_executemany_return_self  # type: ignore[attr-defined]
    Cursor.executescript = _cursor_executescript_return_self  # type: ignore[attr-defined]
