"""Connection state cache for sync total_changes / in_transaction (aiosqlite compat).

This module provides the cached state and wrappers that update it for
begin/commit/rollback/close/__aenter__/transaction. Used by rapsqlite.__init__
after _compat patches are applied.
"""

from __future__ import annotations

import threading
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING:
    from typing import Protocol

    class Connection(Protocol):
        def __aenter__(self): ...
        def __aexit__(self, *args): ...
        async def close(self): ...
        # total_changes, in_transaction, begin, commit, rollback, transaction
        # are patched by this module
else:
    Connection = Any

_connection_state: dict[int, dict[str, Any]] = {}
_connection_state_lock = threading.Lock()

# Set by apply_state(Connection) before attaching wrappers
_orig_total_changes: Any = None
_orig_in_transaction: Any = None
_orig_begin: Any = None
_orig_commit: Any = None
_orig_rollback: Any = None
_orig_close: Any = None
_orig_aenter: Any = None
_orig_transaction: Any = None


def _get_conn_state(conn: Connection) -> dict[str, Any]:
    """Get or create state dict for a connection (thread-safe)."""
    cid = id(conn)
    with _connection_state_lock:
        if cid not in _connection_state:
            _connection_state[cid] = {"total_changes": 0, "in_transaction": False}
        return _connection_state[cid]


def _cleanup_conn_state(conn: Connection) -> None:
    """Remove state for a connection (thread-safe, call on close)."""
    with _connection_state_lock:
        _connection_state.pop(id(conn), None)


def _total_changes_prop(self: Connection) -> int:
    """Get total database changes since connection was opened (sync property for aiosqlite compat)."""
    return _get_conn_state(self).get("total_changes", 0)


def _in_transaction_prop(self: Connection) -> bool:
    """Check if connection is in a transaction (sync property for aiosqlite compat)."""
    return _get_conn_state(self).get("in_transaction", False)


async def _update_connection_state(conn: Connection) -> None:
    """Update cached total_changes and in_transaction from the database."""
    state = _get_conn_state(conn)
    try:
        state["total_changes"] = await _orig_total_changes(conn)
    except Exception:
        pass
    try:
        state["in_transaction"] = await _orig_in_transaction(conn)
    except Exception:
        pass


async def _begin_with_state_update(self: Connection) -> None:
    await _orig_begin(self)
    _get_conn_state(self)["in_transaction"] = True


async def _commit_with_state_update(self: Connection) -> None:
    await _orig_commit(self)
    state = _get_conn_state(self)
    state["in_transaction"] = False
    try:
        state["total_changes"] = await _orig_total_changes(self)
    except Exception:
        pass


async def _rollback_with_state_update(self: Connection) -> None:
    await _orig_rollback(self)
    _get_conn_state(self)["in_transaction"] = False


async def _close_with_state_cleanup(self: Connection) -> None:
    _cleanup_conn_state(self)
    await _orig_close(self)


async def _aenter_with_state_init(self: Connection) -> Connection:
    result = await _orig_aenter(self)
    _get_conn_state(self)
    return result


class _TransactionContextManagerWithState:
    """Wrapper around TransactionContextManager that updates in_transaction state."""

    def __init__(self, conn: Connection, orig_cm: Any) -> None:
        self._conn = conn
        self._orig_cm = orig_cm

    async def __aenter__(self) -> Connection:
        result = await self._orig_cm.__aenter__()
        _get_conn_state(self._conn)["in_transaction"] = True
        return result

    async def __aexit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        await self._orig_cm.__aexit__(exc_type, exc_val, exc_tb)
        _get_conn_state(self._conn)["in_transaction"] = False


def _transaction_with_state(self: Connection) -> _TransactionContextManagerWithState:
    """Return a transaction context manager that updates in_transaction state."""
    return _TransactionContextManagerWithState(self, _orig_transaction(self))


def apply_state(Connection: type) -> None:
    """Attach connection state cache and wrappers to Connection.

    Call after _compat.apply_compat() so commit/rollback are already no-op wrappers.
    """
    global _orig_total_changes, _orig_in_transaction
    global _orig_begin, _orig_commit, _orig_rollback, _orig_close, _orig_aenter, _orig_transaction

    _orig_total_changes = Connection.total_changes
    _orig_in_transaction = Connection.in_transaction
    _orig_begin = Connection.begin
    _orig_commit = Connection.commit
    _orig_rollback = Connection.rollback
    _orig_close = Connection.close
    _orig_aenter = Connection.__aenter__
    _orig_transaction = Connection.transaction

    Connection.total_changes = property(_total_changes_prop)  # type: ignore[assignment,method-assign]
    Connection.in_transaction = property(_in_transaction_prop)  # type: ignore[assignment,method-assign]
    Connection.total_changes_async = _orig_total_changes  # type: ignore[attr-defined]
    Connection.in_transaction_async = _orig_in_transaction  # type: ignore[attr-defined]
    Connection.begin = _begin_with_state_update  # type: ignore[assignment,method-assign]
    Connection.commit = _commit_with_state_update  # type: ignore[assignment,method-assign]
    Connection.rollback = _rollback_with_state_update  # type: ignore[assignment,method-assign]
    Connection.close = _close_with_state_cleanup  # type: ignore[assignment,method-assign]
    Connection.__aenter__ = _aenter_with_state_init  # type: ignore[assignment,method-assign]
    Connection.transaction = _transaction_with_state  # type: ignore[assignment,method-assign]
