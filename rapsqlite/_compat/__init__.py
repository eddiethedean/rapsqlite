"""aiosqlite compatibility patches for Connection and Cursor.

Patch groups: lifecycle, progress_handler, cursor chaining, commit/rollback no-op,
iterdump and backup, connection conveniences, slow query and __await__.
Single entry point: apply_compat().
"""

from __future__ import annotations

from typing import Type

from rapsqlite._compat.commit_rollback import apply_commit_rollback_noop_compat
from rapsqlite._compat.connection_conveniences import (
    apply_connection_conveniences_compat,
)
from rapsqlite._compat.cursor_chain import apply_cursor_chaining_compat
from rapsqlite._compat.iterdump_backup import apply_iterdump_and_backup_compat
from rapsqlite._compat.lifecycle import apply_lifecycle_compat
from rapsqlite._compat.progress_handler import apply_progress_handler_compat
from rapsqlite._compat.slow_query import apply_slow_query_and_await_compat


def apply_compat(
    Connection: type,
    Cursor: type,
    *,
    operational_error: Type[BaseException],
) -> None:
    """Attach all aiosqlite compatibility patches to Connection and Cursor.

    Call with the extension Connection and Cursor classes and the
    OperationalError exception class (from the extension or rapsqlite).
    """
    apply_lifecycle_compat(Connection)
    apply_progress_handler_compat(Connection)
    apply_cursor_chaining_compat(Cursor)
    apply_commit_rollback_noop_compat(Connection, operational_error)
    apply_iterdump_and_backup_compat(Connection, operational_error)
    apply_connection_conveniences_compat(Connection)
    apply_slow_query_and_await_compat(Connection)
