"""
SQLAlchemy dialect for rapsqlite: ``sqlite+rapsqlite``.

Use with create_async_engine:

    import rapsqlite.sqlalchemy  # register dialect
    from sqlalchemy.ext.asyncio import create_async_engine
    engine = create_async_engine("sqlite+rapsqlite:///path.db")
    # or sqlite+rapsqlite:///:memory:

Requires SQLAlchemy to be installed (e.g. ``pip install sqlalchemy``).
You can also install both in one step: ``pip install rapsqlite[sqlalchemy]``.
"""

import math
import re
import sqlite3
from typing import Any

try:
    from sqlalchemy.dialects import registry
    from sqlalchemy.dialects.sqlite.pysqlite import SQLiteDialect_pysqlite
    from sqlalchemy import pool
    from sqlalchemy.engine import URL
    from sqlalchemy.util.concurrency import await_fallback as _await
    from sqlalchemy.connectors.asyncio import (
        AsyncAdapt_dbapi_connection,
        AsyncAdapt_dbapi_cursor,
    )
except ImportError as e:
    raise ImportError(
        "The sqlite+rapsqlite dialect requires SQLAlchemy to be installed. "
        "Install it with: pip install sqlalchemy "
        "(or pip install rapsqlite[sqlalchemy] to install both in one step)."
    ) from e

from . import dbapi as _dbapi


class _RapsqliteCursor(AsyncAdapt_dbapi_cursor):
    """Cache description so SQLAlchemy can build result metadata after cursor close."""

    __slots__ = ("_last_description",)

    def __init__(self, adapt_connection: Any) -> None:
        super().__init__(adapt_connection)
        self._last_description: Any = None

    def _make_new_cursor(self, connection: Any) -> Any:
        return self._adapt_connection.await_(connection.cursor())

    def execute(
        self,
        operation: Any,
        parameters: Any = None,
    ) -> Any:
        result = super().execute(operation, parameters)
        # Cache description right after execute so 0-row SELECT (e.g. session.get missing key)
        # is visible to SQLAlchemy when _setup_result_proxy reads context.cursor.description.
        if self._cursor is not None:
            desc = self._cursor.description
            if desc is not None:
                self._last_description = desc
        return result

    @property
    def description(self) -> Any:
        soft_memo = getattr(self, "_soft_closed_memoized", None)
        if soft_memo is not None and "description" in soft_memo:
            return soft_memo["description"]
        desc = self._cursor.description if self._cursor is not None else None
        if desc is not None:
            self._last_description = desc
        if self._last_description is not None:
            return self._last_description
        return desc


class _RapsqliteConnection(AsyncAdapt_dbapi_connection):
    __slots__ = ()
    _cursor_cls = _RapsqliteCursor


class _RapsqliteDialectModule:
    """DBAPI-compatible module for dialect. connect() is sync, uses await_."""

    paramstyle = _dbapi.paramstyle
    apilevel = _dbapi.apilevel
    threadsafety = _dbapi.threadsafety
    sqlite_version = sqlite3.sqlite_version
    sqlite_version_info = sqlite3.sqlite_version_info
    Error = _dbapi.Error
    InterfaceError = _dbapi.InterfaceError
    DatabaseError = _dbapi.DatabaseError
    DataError = _dbapi.DataError  # type: ignore[attr-defined]
    OperationalError = _dbapi.OperationalError
    IntegrityError = _dbapi.IntegrityError  # type: ignore[attr-defined]
    InternalError = _dbapi.InternalError  # type: ignore[attr-defined]
    ProgrammingError = _dbapi.ProgrammingError
    NotSupportedError = _dbapi.NotSupportedError  # type: ignore[attr-defined]

    def connect(self, *arg: Any, **kw: Any) -> _RapsqliteConnection:
        from sqlalchemy.util.concurrency import await_fallback

        creator_fn = kw.pop("async_creator_fn", None)
        if creator_fn:
            raw = await_fallback(creator_fn(*arg, **kw))
        else:
            raw = await_fallback(_dbapi.connect(*arg, **kw))
        return _RapsqliteConnection(self, raw)


class SQLiteDialect_rapsqlite(SQLiteDialect_pysqlite):
    """Async SQLite dialect using rapsqlite. Use with create_async_engine('sqlite+rapsqlite:///...')."""

    driver = "rapsqlite"
    supports_statement_cache = True
    is_async = True
    has_terminate = False
    supports_server_side_cursors = False

    @classmethod
    def import_dbapi(cls) -> _RapsqliteDialectModule:  # type: ignore[override]
        return _RapsqliteDialectModule()

    @classmethod
    def get_pool_class(cls, url: URL) -> type[pool.Pool]:
        if cls._is_url_file_db(url):
            return pool.AsyncAdaptedQueuePool
        return pool.StaticPool

    def on_connect(self) -> Any:
        """Register regexp and floor on each new connection (DBAPI create_function)."""

        def _regexp(pattern: str, value: Any) -> Any:
            if value is None:
                return None
            return re.search(pattern, value) is not None

        # deterministic=True for SQLite 3.9+ (match pysqlite)
        try:
            version = self._get_server_version_info(None)
            create_func_kw = (
                {"deterministic": True} if version and version >= (3, 9) else {}
            )
        except Exception:
            create_func_kw = {}

        def connect(dbapi_connection: Any) -> None:
            # Pool passes AsyncAdapt_dbapi_connection; raw is in ._connection
            raw = getattr(dbapi_connection, "_connection", dbapi_connection)
            _await(raw.create_function("regexp", 2, _regexp, **create_func_kw))
            _await(raw.create_function("floor", 1, math.floor, **create_func_kw))

        return connect


dialect = SQLiteDialect_rapsqlite

registry.register("sqlite.rapsqlite", __name__, "SQLiteDialect_rapsqlite")
