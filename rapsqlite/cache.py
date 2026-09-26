"""A small TTL-aware cache API backed by a rapsqlite connection."""

from __future__ import annotations

import asyncio
import math
import re
from collections.abc import Awaitable, Callable
from contextvars import ContextVar
from typing import Protocol, TypeVar


_Result = TypeVar("_Result")
_INITIALIZING_CACHES: ContextVar[frozenset[object]] = ContextVar(
    "rapsqlite_initializing_caches", default=frozenset()
)


class _CacheConnection(Protocol):
    def _cache_initialize(self, table_name: str) -> Awaitable[bool]: ...

    def _cache_get(self, table_name: str, key: str) -> Awaitable[bytes | None]: ...

    def _cache_set(
        self,
        table_name: str,
        key: str,
        value: bytes,
        ttl_seconds: float | None = None,
    ) -> Awaitable[None]: ...

    def _cache_delete(self, table_name: str, key: str) -> Awaitable[bool]: ...

    def _cache_cleanup_expired(self, table_name: str, limit: int) -> Awaitable[int]: ...


class SQLiteCache:
    """A TTL-aware BLOB cache using an existing rapsqlite connection.

    The cache does not own or close ``connection``. Call :meth:`initialize`
    to create its table eagerly, or let the first operation initialize it.
    Keys are strings and values are bytes so callers can choose their own
    serialization format.

    Expiration uses Unix wall-clock seconds. A missing or expired key returns
    ``None``; expired rows remain until :meth:`cleanup_expired` is called.
    Cache operations are serialized with other work on the same connection.
    If a transaction rolls back schema creation, the next cache operation
    detects the missing table, recreates the schema, and retries once.
    """

    __slots__ = ("_connection", "_table_name", "_initialize_lock", "_initialized")

    def __init__(
        self, connection: _CacheConnection, *, table_name: str = "rapsqlite_cache"
    ) -> None:
        self._table_name = self._validate_table_name(table_name)
        self._connection = connection
        # A regular asyncio.Lock coalesces concurrent setup, but an init hook
        # can re-enter this cache while setup is awaiting the connection. The
        # context marker makes that same logical initialization re-entrant.
        self._initialize_lock = asyncio.Lock()
        self._initialized = False

    async def initialize(self) -> None:
        """Create the cache table and expiration index if needed.

        Initialization is also performed automatically by the first cache
        operation, so this method is optional when lazy setup is acceptable.
        """

        if self in _INITIALIZING_CACHES.get():
            if not self._initialized:
                await self._initialize_schema()
            return
        async with self._initialize_lock:
            await self._initialize_schema_guarded()

    async def _ensure_initialized(self) -> None:
        if self._initialized:
            return
        if self in _INITIALIZING_CACHES.get():
            await self._initialize_schema()
            return
        async with self._initialize_lock:
            if not self._initialized:
                await self._initialize_schema_guarded()

    async def _initialize_schema_guarded(self) -> None:
        token = _INITIALIZING_CACHES.set(_INITIALIZING_CACHES.get() | {self})
        try:
            await self._initialize_schema()
        finally:
            _INITIALIZING_CACHES.reset(token)

    async def _initialize_schema(self) -> None:
        # The table may be rolled back with its surrounding transaction. Mark it
        # initialized optimistically to avoid repeating CREATE IF NOT EXISTS on
        # every operation; _run_cache_operation repairs the state on a missing
        # table error after a rollback.
        await self._connection._cache_initialize(  # pyright: ignore[reportPrivateUsage]
            self._table_name
        )
        self._initialized = True

    async def _run_cache_operation(
        self, operation: Callable[[], Awaitable[_Result]]
    ) -> _Result:
        try:
            return await operation()
        except Exception as error:
            if "no such table:" not in str(error).lower():
                raise
            if self in _INITIALIZING_CACHES.get():
                raise

        async with self._initialize_lock:
            self._initialized = False
            await self._initialize_schema_guarded()
        return await operation()

    async def get(self, key: str) -> bytes | None:
        """Return a BLOB for a live key, or ``None`` for a miss/expired key."""

        key = self._validate_key(key)
        await self._ensure_initialized()
        return await self._run_cache_operation(
            lambda: self._connection._cache_get(  # pyright: ignore[reportPrivateUsage]
                self._table_name, key
            )
        )

    async def set(self, key: str, value: bytes, *, ttl: float | None = None) -> None:
        """Store bytes, optionally expiring ``ttl`` seconds from now.

        ``ttl=None`` stores a non-expiring entry. A zero TTL makes the entry
        immediately expired. Negative, non-finite, and boolean TTLs are rejected.
        The timestamp is calculated in Rust when the write reaches SQLite.
        """

        key = self._validate_key(key)
        value = self._validate_value(value)
        ttl_seconds = self._validate_ttl(ttl)
        await self._ensure_initialized()
        await self._run_cache_operation(
            lambda: self._connection._cache_set(  # pyright: ignore[reportPrivateUsage]
                self._table_name, key, value, ttl_seconds
            )
        )

    async def delete(self, key: str) -> bool:
        """Delete ``key`` and return whether a row was present."""

        key = self._validate_key(key)
        await self._ensure_initialized()
        return await self._run_cache_operation(
            lambda: self._connection._cache_delete(  # pyright: ignore[reportPrivateUsage]
                self._table_name, key
            )
        )

    async def cleanup_expired(self, *, limit: int = 1000) -> int:
        """Delete at most ``limit`` expired rows and return the deleted count.

        The bounded default keeps maintenance work from turning into one
        unbounded request-path operation. Call this periodically or from a
        background task; reads already treat expired rows as misses.
        """

        limit = self._validate_cleanup_limit(limit)
        await self._ensure_initialized()
        return await self._run_cache_operation(
            lambda: self._connection._cache_cleanup_expired(  # pyright: ignore[reportPrivateUsage]
                self._table_name, limit
            )
        )

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
    def _validate_ttl(ttl: object) -> float | None:
        if ttl is None:
            return None
        if isinstance(ttl, bool) or not isinstance(ttl, (int, float)):
            raise TypeError("ttl must be a finite non-negative number or None")
        try:
            ttl_seconds = float(ttl)
        except OverflowError:
            raise ValueError("ttl is too large") from None
        if not math.isfinite(ttl_seconds) or ttl_seconds < 0:
            raise ValueError("ttl must be finite and non-negative")
        return ttl_seconds
