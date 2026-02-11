"""Compat so commit()/rollback() are no-ops when no transaction is active."""

from __future__ import annotations

from typing import Any, Type

_orig_commit: Any = None
_orig_rollback: Any = None


def _is_no_tx_error_message(msg: str) -> bool:
    """Return True if the error message indicates no active transaction.

    This centralizes the logic for treating commit()/rollback() outside a
    transaction as a no-op for DBAPI/aiosqlite compatibility.
    """
    msg = msg.lower()
    if "transaction" not in msg:
        return False
    return "not available" in msg or "in progress" in msg or "no transaction" in msg


async def _commit_noop_on_no_tx(
    self: Any, operational_error_type: Type[BaseException]
) -> None:
    try:
        await _orig_commit(self)
    except operational_error_type as e:
        if _is_no_tx_error_message(str(e)):
            return
        raise


async def _rollback_noop_on_no_tx(
    self: Any, operational_error_type: Type[BaseException]
) -> None:
    try:
        await _orig_rollback(self)
    except operational_error_type as e:
        if _is_no_tx_error_message(str(e)):
            return
        raise


def apply_commit_rollback_noop_compat(
    Connection: type,
    operational_error: Type[BaseException],
) -> None:
    """Compat so commit()/rollback() are no-ops when no transaction is active."""
    global _orig_commit, _orig_rollback

    _orig_commit = Connection.commit  # type: ignore[attr-defined]
    _orig_rollback = Connection.rollback  # type: ignore[attr-defined]

    async def _commit_noop(self: Any) -> None:
        await _commit_noop_on_no_tx(self, operational_error)

    async def _rollback_noop(self: Any) -> None:
        await _rollback_noop_on_no_tx(self, operational_error)

    Connection.commit = _commit_noop  # type: ignore[attr-defined]
    Connection.rollback = _rollback_noop  # type: ignore[attr-defined]
