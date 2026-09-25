"""Tests for the process-local SQLiteCache API."""

import asyncio
from typing import Any, cast

import pytest

from rapsqlite import SQLiteCache, connect_memory


class _TransactionalCacheConnection:
    """Minimal connection double for transaction-scoped schema setup tests."""

    def __init__(self) -> None:
        self.initialization_calls = 0

    async def _cache_initialize(self, table_name: str) -> bool:
        self.initialization_calls += 1
        return True

    async def _cache_get(self, table_name: str, key: str) -> bytes | None:
        return None


@pytest.mark.asyncio
async def test_cache_basic_get_set_delete_and_lazy_initialization() -> None:
    async with connect_memory(session_affinity=True) as conn:
        cache = SQLiteCache(conn)

        assert await cache.get("missing") is None
        await cache.set("key", b"payload")
        assert await cache.get("key") == b"payload"
        assert await cache.delete("key") is True
        assert await cache.delete("key") is False
        assert await cache.get("key") is None


@pytest.mark.asyncio
async def test_cache_initialization_is_retried_after_transaction_rollback() -> None:
    async with connect_memory(session_affinity=True) as conn:
        cache = SQLiteCache(conn)
        await conn.begin()

        await cache.initialize()
        await cache.set("rolled-back", b"value")

        await conn.rollback()

        assert await cache.get("rolled-back") is None


@pytest.mark.asyncio
async def test_cache_does_not_repeat_schema_setup_inside_a_transaction() -> None:
    connection = _TransactionalCacheConnection()
    cache = SQLiteCache(cast(Any, connection))

    assert await cache.get("first") is None
    assert await cache.get("second") is None

    assert connection.initialization_calls == 1


@pytest.mark.asyncio
async def test_cache_ttl_upsert_and_bounded_expiration_cleanup() -> None:
    async with connect_memory(session_affinity=True) as conn:
        cache = SQLiteCache(conn)
        await cache.initialize()

        await cache.set("live", b"old", ttl=60)
        await cache.set("live", b"new", ttl=60)
        await cache.set("expired-1", b"one", ttl=0)
        await cache.set("expired-2", b"two", ttl=0)
        await cache.set("expired-3", b"three", ttl=0)

        assert await cache.get("live") == b"new"
        assert await cache.get("expired-1") is None
        assert await cache.cleanup_expired(limit=2) == 2
        assert await cache.cleanup_expired(limit=2) == 1
        assert await cache.get("expired-2") is None


@pytest.mark.asyncio
async def test_cache_operations_preserve_registered_sqlite_callbacks() -> None:
    async with connect_memory(session_affinity=True) as conn:
        # Keep the shared in-memory database alive while callback operations
        # temporarily check out and discard their callback-bound connection.
        assert await conn.fetch_scalar("SELECT 1") == 1

        def double(value: int) -> int:
            return value * 2

        await conn.create_function("double", 1, double)
        cache = SQLiteCache(conn)

        await cache.set("key", b"value")

        assert await cache.get("key") == b"value"
        assert await conn.fetch_scalar("SELECT double(21)") == 42


@pytest.mark.asyncio
async def test_cache_concurrent_first_use_initializes_once_safely() -> None:
    async with connect_memory(session_affinity=True) as conn:
        caches = (SQLiteCache(conn), SQLiteCache(conn))

        await asyncio.gather(
            *(
                caches[index % len(caches)].set(f"key-{index}", str(index).encode())
                for index in range(20)
            )
        )

        assert await asyncio.gather(
            *(caches[index % len(caches)].get(f"key-{index}") for index in range(20))
        ) == [str(index).encode() for index in range(20)]


@pytest.mark.asyncio
async def test_cache_validates_table_name() -> None:
    async with connect_memory() as conn:
        with pytest.raises(ValueError, match="table_name"):
            SQLiteCache(conn, table_name='cache"; DROP TABLE users; --')


@pytest.mark.asyncio
async def test_cache_validates_keys_values_ttl_and_cleanup_limit() -> None:
    async with connect_memory() as conn:
        cache = SQLiteCache(conn)

        with pytest.raises(TypeError, match="keys must be strings"):
            await cache.get(1)  # type: ignore[arg-type]
        with pytest.raises(TypeError, match="values must be bytes"):
            await cache.set("key", "not-bytes")  # type: ignore[arg-type]
        for invalid_ttl in (-1, float("nan"), float("inf")):
            with pytest.raises(ValueError, match="ttl"):
                await cache.set("key", b"value", ttl=invalid_ttl)
        with pytest.raises(TypeError, match="ttl"):
            await cache.set("key", b"value", ttl=True)  # type: ignore[arg-type]
        with pytest.raises(TypeError, match="limit"):
            await cache.cleanup_expired(limit=True)  # type: ignore[arg-type]
        with pytest.raises(ValueError, match="at least 1"):
            await cache.cleanup_expired(limit=0)
