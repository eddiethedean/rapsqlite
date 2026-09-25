"""A small TTL-aware cache API backed by a rapsqlite connection."""

from __future__ import annotations

import asyncio
import math
import re
import time
from collections.abc import Awaitable
from typing import Any, Protocol, cast


class _Cursor(Protocol):
    @property
    def rowcount(self) -> int: ...

    def close(self) -> Awaitable[None]: ...


class _PreparedOperation(Protocol):
    def execute(self, parameters: Any | None = None) -> Awaitable[Any]: ...

    def fetch_all(self, parameters: Any | None = None) -> Awaitable[list[Any]]: ...

    def fetch_blob(self, parameters: Any | None = None) -> Awaitable[bytes | None]: ...


class _CacheConnection(Protocol):
    def prepare(
        self, query: str, *, raw: bool = False, blob: bool = False
    ) -> _PreparedOperation: ...


class SQLiteCache:
    """A TTL-aware BLOB cache using an existing rapsqlite connection.

    The cache does not own or close ``connection``. Call :meth:`initialize`
    to create its table eagerly, or let the first operation initialize it.
    Keys are strings and values are bytes so callers can choose their own
    serialization format.

    Expiration uses Unix wall-clock seconds. A missing or expired key returns
    ``None``; expired rows remain until :meth:`cleanup_expired` is called.
    """

    __slots__ = (
        "_connection",
        "_table",
        "_operation_lock",
        "_initialized",
        "_create_table",
        "_create_expiration_index",
        "_get",
        "_set",
        "_delete",
        "_cleanup_expired",
    )

    def __init__(
        self, connection: _CacheConnection, *, table_name: str = "rapsqlite_cache"
    ) -> None:
        table_name = self._validate_table_name(table_name)

        self._connection = connection
        self._table = f'"{table_name}"'
        # rapsqlite connections may stall when multiple operations are
        # dispatched concurrently through the same connection object. Keep
        # this cache's complete operation sequence ordered on its connection.
        self._operation_lock = asyncio.Lock()
        self._initialized = False

        index_name = f'"{table_name}_expires_idx"'
        self._create_table = connection.prepare(
            f"CREATE TABLE IF NOT EXISTS {self._table} ("
            "key TEXT PRIMARY KEY NOT NULL, "
            "value BLOB NOT NULL, "
            "expires_at REAL"
            ") WITHOUT ROWID"
        )
        self._create_expiration_index = connection.prepare(
            f"CREATE INDEX IF NOT EXISTS {index_name} ON {self._table} (expires_at) "
            "WHERE expires_at IS NOT NULL"
        )
        self._get = connection.prepare(
            f"SELECT value FROM {self._table} WHERE key = ? "
            "AND (expires_at IS NULL OR expires_at > ?)"
        )
        self._set = connection.prepare(
            f"INSERT INTO {self._table} (key, value, expires_at) VALUES (?, ?, ?) "
            "ON CONFLICT(key) DO UPDATE SET "
            "value = excluded.value, expires_at = excluded.expires_at"
        )
        self._delete = connection.prepare(f"DELETE FROM {self._table} WHERE key = ?")
        self._cleanup_expired = connection.prepare(
            f"DELETE FROM {self._table} WHERE key IN ("
            f"SELECT key FROM {self._table} "
            "WHERE expires_at IS NOT NULL AND expires_at <= ? "
            "ORDER BY expires_at LIMIT ?"
            ")"
        )

    async def initialize(self) -> None:
        """Create the cache table and expiration index if needed.

        Initialization is also performed automatically by the first cache
        operation, so this method is optional when lazy setup is acceptable.
        """

        async with self._operation_lock:
            await self._initialize_unlocked()

    async def get(self, key: str) -> bytes | None:
        """Return a BLOB for a live key, or ``None`` for a miss/expired key."""

        key = self._validate_key(key)
        async with self._operation_lock:
            await self._initialize_unlocked()
            return await self._get.fetch_blob([key, time.time()])

    async def set(self, key: str, value: bytes, *, ttl: float | None = None) -> None:
        """Store bytes, optionally expiring ``ttl`` seconds from now.

        ``ttl=None`` stores a non-expiring entry. A zero TTL makes the entry
        immediately expired. Negative, non-finite, and boolean TTLs are rejected.
        """

        key = self._validate_key(key)
        value = self._validate_value(value)
        expires_at = self._expiration_time(ttl)
        async with self._operation_lock:
            await self._initialize_unlocked()
            await self._execute(self._set, [key, value, expires_at])

    async def delete(self, key: str) -> bool:
        """Delete ``key`` and return whether a row was present."""

        key = self._validate_key(key)
        async with self._operation_lock:
            await self._initialize_unlocked()
            return (await self._execute(self._delete, [key])) > 0

    async def cleanup_expired(self, *, limit: int = 1000) -> int:
        """Delete at most ``limit`` expired rows and return the deleted count.

        The bounded default keeps maintenance work from turning into one
        unbounded request-path operation. Call this periodically or from a
        background task; reads already treat expired rows as misses.
        """

        limit = self._validate_cleanup_limit(limit)
        async with self._operation_lock:
            await self._initialize_unlocked()
            return await self._execute(self._cleanup_expired, [time.time(), limit])

    async def _initialize_unlocked(self) -> None:
        if self._initialized:
            return
        await self._execute(self._create_table)
        await self._execute(self._create_expiration_index)
        self._initialized = True

    async def _execute(
        self, operation: _PreparedOperation, parameters: Any | None = None
    ) -> int:
        cursor = cast(_Cursor, await operation.execute(parameters))
        try:
            return cursor.rowcount
        finally:
            await cursor.close()

    @staticmethod
    def _validate_table_name(table_name: object) -> str:
        if not isinstance(table_name, str) or not re.fullmatch(
            r"[A-Za-z_][A-Za-z0-9_]{0,99}", table_name
        ):
            raise ValueError(
                "table_name must be a SQLite identifier of 1 to 100 "
                "letters, digits, and underscores, starting with a letter or underscore"
            )
        return table_name

    @staticmethod
    def _validate_key(key: object) -> str:
        if not isinstance(key, str):
            raise TypeError("cache keys must be strings")
        return key

    @staticmethod
    def _validate_value(value: object) -> bytes:
        if not isinstance(value, bytes):
            raise TypeError("cache values must be bytes")
        return value

    @staticmethod
    def _validate_cleanup_limit(limit: object) -> int:
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise TypeError("limit must be an integer")
        if limit < 1:
            raise ValueError("limit must be at least 1")
        return limit

    @staticmethod
    def _expiration_time(ttl: object) -> float | None:
        if ttl is None:
            return None
        if isinstance(ttl, bool) or not isinstance(ttl, (int, float)):
            raise TypeError("ttl must be a finite non-negative number or None")
        ttl_seconds = float(ttl)
        if not math.isfinite(ttl_seconds) or ttl_seconds < 0:
            raise ValueError("ttl must be finite and non-negative")
        expires_at = time.time() + ttl_seconds
        if not math.isfinite(expires_at):
            raise ValueError("ttl is too large")
        return expires_at
