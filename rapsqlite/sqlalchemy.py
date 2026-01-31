"""
SQLAlchemy dialect for rapsqlite: ``sqlite+rapsqlite``.

Use with create_async_engine:

    import rapsqlite.sqlalchemy  # register dialect
    from sqlalchemy.ext.asyncio import create_async_engine
    engine = create_async_engine("sqlite+rapsqlite:///path.db")
    # or sqlite+rapsqlite:///:memory:

Requires ``pip install rapsqlite[sqlalchemy]`` (adds sqlalchemy dependency).
"""

import sqlite3
from typing import Any

try:
    from sqlalchemy.dialects import registry
    from sqlalchemy.dialects.sqlite.pysqlite import SQLiteDialect_pysqlite
    from sqlalchemy import pool
    from sqlalchemy.engine import URL
    from sqlalchemy.connectors.asyncio import (
        AsyncAdapt_dbapi_connection,
        AsyncAdapt_dbapi_cursor,
    )
except ImportError as e:
    raise ImportError(
        "rapsqlite sqlite+rapsqlite dialect requires sqlalchemy. "
        "Install with: pip install 'rapsqlite[sqlalchemy]'"
    ) from e

from . import dbapi as _dbapi


class _RapsqliteCursor(AsyncAdapt_dbapi_cursor):
    __slots__ = ()

    def _make_new_cursor(self, connection: Any) -> Any:
        return self._adapt_connection.await_(connection.cursor())


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
    DataError = _dbapi.DataError
    OperationalError = _dbapi.OperationalError
    IntegrityError = _dbapi.IntegrityError
    InternalError = _dbapi.InternalError
    ProgrammingError = _dbapi.ProgrammingError
    NotSupportedError = _dbapi.NotSupportedError

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
        """No regexp/floor on_connect; rapsqlite does not expose create_function via DBAPI."""
        return None


dialect = SQLiteDialect_rapsqlite

registry.register("sqlite.rapsqlite", __name__, "SQLiteDialect_rapsqlite")
