"""Reusable query objects for repeated low-latency operations."""

from __future__ import annotations

from typing import Any, cast


class PreparedQuery:
    """A connection-bound reusable SQL operation.

    The object keeps the SQL text and execution mode out of application hot
    loops. ``raw=True`` is limited to scalar/BLOB lookups and uses the opt-in
    raw SQLite path; the default delegates to the normal compatibility APIs.
    """

    __slots__ = (
        "_connection",
        "_query",
        "_raw",
        "_blob",
        "_execute",
        "_fetch_all",
        "_fetch_one",
        "_fetch_optional",
        "_fetch_scalar",
        "_fetch_blob",
        "_raw_fetch_scalar",
    )

    def __init__(
        self,
        connection: Any,
        query: object,
        *,
        raw: bool = False,
        blob: bool = False,
    ) -> None:
        if not isinstance(query, str) or not query:
            raise ValueError("query must be a non-empty string")
        self._connection = connection
        self._query = query
        self._raw = raw
        self._blob = blob
        # Resolve the connection methods once. This keeps repeated prepared
        # calls from redoing Python attribute lookup while still letting the
        # Rust connection apply current transaction/callback/close state.
        self._execute = connection.execute
        self._fetch_all = connection.fetch_all
        self._fetch_one = connection.fetch_one
        self._fetch_optional = connection.fetch_optional
        self._fetch_scalar = connection.fetch_scalar
        self._fetch_blob = connection.fetch_blob
        self._raw_fetch_scalar = connection.raw_fetch_scalar

    @property
    def query(self) -> str:
        """The SQL text retained by this prepared operation."""

        return self._query

    async def execute(self, parameters: Any | None = None) -> Any:
        """Execute the query through the normal compatibility path."""

        return await self._execute(self._query, parameters)

    async def fetch_all(self, parameters: Any | None = None) -> list[Any]:
        """Fetch rows through the normal compatibility path."""

        return cast(list[Any], await self._fetch_all(self._query, parameters))

    async def fetch_one(self, parameters: Any | None = None) -> Any:
        """Fetch exactly one row through the normal compatibility path."""

        return await self._fetch_one(self._query, parameters)

    async def fetch_optional(self, parameters: Any | None = None) -> Any:
        """Fetch one optional row through the normal compatibility path."""

        return await self._fetch_optional(self._query, parameters)

    async def fetch_scalar(self, parameters: Any | None = None) -> Any:
        """Fetch one scalar result, optionally using raw SQLite execution."""

        if self._raw:
            return await self._raw_fetch_scalar(self._query, parameters, self._blob)
        return await self._fetch_scalar(self._query, parameters)

    async def fetch_blob(self, parameters: Any | None = None) -> bytes | None:
        """Fetch one BLOB result, optionally using raw SQLite execution."""

        if self._raw:
            value = await self._raw_fetch_scalar(self._query, parameters, True)
        else:
            value = await self._fetch_blob(self._query, parameters)
        if value is None:
            return None
        if not isinstance(value, bytes):
            raise TypeError("fetch_blob() requires a BLOB or NULL result")
        return value

    def __repr__(self) -> str:
        mode = "raw" if self._raw else "sqlx"
        return f"PreparedQuery({self._query!r}, mode={mode!r})"
