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
from typing import Any, cast

try:
    from sqlalchemy.dialects import registry
    from sqlalchemy.dialects.sqlite.pysqlite import SQLiteDialect_pysqlite
    from sqlalchemy import pool
    from sqlalchemy.engine import URL
    from sqlalchemy.engine.interfaces import DBAPIModule
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
    Error = _dbapi.Error  # type: ignore[has-type]
    InterfaceError = _dbapi.InterfaceError  # type: ignore[has-type]
    DatabaseError = _dbapi.DatabaseError  # type: ignore[has-type]
    DataError = _dbapi.DataError  # type: ignore[attr-defined]
    OperationalError = _dbapi.OperationalError  # type: ignore[has-type]
    IntegrityError = _dbapi.IntegrityError  # type: ignore[attr-defined]
    InternalError = _dbapi.InternalError  # type: ignore[attr-defined]
    ProgrammingError = _dbapi.ProgrammingError  # type: ignore[has-type]
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
    def import_dbapi(cls) -> DBAPIModule:
        return cast(DBAPIModule, _RapsqliteDialectModule())

    @classmethod
    def get_pool_class(cls, url: URL) -> type[pool.Pool]:
        if cls._is_url_file_db(url):
            return cast(type[pool.Pool], pool.AsyncAdaptedQueuePool)
        return cast(type[pool.Pool], pool.StaticPool)

    def has_table(
        self, connection: Any, table_name: str, schema: str | None = None, **kw: Any
    ) -> bool:
        """Override to use sqlite_master SELECT instead of PRAGMA table_info.

        The base implementation uses _get_table_pragma and skips fetchall() when
        cursor._soft_closed is True, which can happen with async adapters and
        causes has_table to return False for existing tables (e.g. alembic_version
        during downgrade base). Using a direct SELECT avoids that and fixes
        Alembic downgrade with sqlite+rapsqlite.
        """
        self._ensure_has_table_connection(connection)
        if schema is not None and schema not in self.get_schema_names(connection, **kw):
            return False
        if schema is not None:
            qschema = self.identifier_preparer.quote_identifier(schema)
            stmt = (
                f"SELECT 1 FROM {qschema}.sqlite_master WHERE type='table' AND name=?"
            )
        else:
            stmt = "SELECT 1 FROM (SELECT name FROM sqlite_master UNION ALL SELECT name FROM sqlite_temp_master) WHERE name=?"
        result = connection.exec_driver_sql(stmt, (table_name,))
        row = result.fetchone()
        return row is not None

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
