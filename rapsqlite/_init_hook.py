"""Context tracking for reentrant Rust connection initialization hooks."""

from collections.abc import Awaitable, Callable
from contextvars import ContextVar, Token
from typing import Any

__all__ = ["_run_hook", "_is_active"]

_ACTIVE_CONNECTION: ContextVar[object | None] = ContextVar(
    "rapsqlite_active_init_hook_connection", default=None
)


async def _run_hook(
    hook: Callable[[object], Awaitable[Any]], connection: object
) -> Any:
    """Run a connection init hook with its connection marked as reentrant."""
    token: Token[object | None] = _ACTIVE_CONNECTION.set(connection)
    try:
        return await hook(connection)
    finally:
        _ACTIVE_CONNECTION.reset(token)


def _is_active(connection: object) -> bool:
    """Return whether this context is currently running that connection's hook."""
    return _ACTIVE_CONNECTION.get() is connection
