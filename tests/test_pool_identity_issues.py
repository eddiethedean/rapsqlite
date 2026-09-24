"""Regression tests for pool sizing, URI handling, and pool lifetimes."""

import gc
import os
import sqlite3

import pytest

from rapsqlite import connect, connect_memory, pool_metrics_gauges

pytestmark = [pytest.mark.unit]


@pytest.mark.asyncio
@pytest.mark.parametrize("pool_size", [1, 4, 8])
async def test_explicit_pool_size_and_shared_pool_metrics(test_db, tmp_path, pool_size):
    db_path = tmp_path / f"pool-size-{pool_size}.db"
    first = connect(db_path, pool_size=pool_size)
    second = connect(db_path, pool_size=2)
    try:
        await first.fetch_all("SELECT 1")
        metrics_first = await first.pool_metrics()
        metrics_second = await second.pool_metrics()
        assert metrics_first["max_connections"] == pool_size
        assert metrics_second["max_connections"] == pool_size
        assert (await pool_metrics_gauges(first))["rapsqlite_pool_max_connections"] == pool_size
    finally:
        await second.close()
        await first.close()


@pytest.mark.asyncio
async def test_default_pool_size_is_reported_from_shared_pool(tmp_path):
    db = connect(tmp_path / "default-pool-size.db")
    try:
        metrics = await db.pool_metrics()
        assert metrics["max_connections"] == 25
    finally:
        await db.close()


@pytest.mark.asyncio
async def test_connect_memory_named_databases_share_and_isolate():
    first = connect_memory(name="shared-cache", pool_size=2)
    second = connect_memory(name="shared-cache", pool_size=4)
    isolated = connect_memory(name="other-cache")
    try:
        await first.execute("CREATE TABLE cache_data (value TEXT)")
        await first.execute("INSERT INTO cache_data VALUES ('shared')")
        await first.commit()
        assert await second.fetch_all("SELECT value FROM cache_data") == [["shared"]]
        with pytest.raises(Exception):
            await isolated.fetch_all("SELECT value FROM cache_data")
        assert (await second.pool_metrics())["max_connections"] == 2
    finally:
        await isolated.close()
        await second.close()
        await first.close()

    fresh = connect_memory(name="shared-cache")
    try:
        with pytest.raises(Exception):
            await fresh.fetch_all("SELECT value FROM cache_data")
    finally:
        await fresh.close()


@pytest.mark.asyncio
async def test_connect_memory_without_name_is_isolated():
    first = connect_memory()
    second = connect_memory()
    try:
        await first.execute("CREATE TABLE private_data (value TEXT)")
        with pytest.raises(Exception):
            await second.fetch_all("SELECT value FROM private_data")
    finally:
        await second.close()
        await first.close()


@pytest.mark.asyncio
async def test_file_uri_memory_mode_does_not_create_a_disk_file(tmp_path):
    db_path = tmp_path / "cache"
    async with connect(f"file:{db_path}?mode=memory&cache=shared") as db:
        await db.execute("CREATE TABLE cache_data (value TEXT)")
        assert await db.fetch_all("SELECT name FROM sqlite_master WHERE name='cache_data'")
    assert not db_path.exists()


@pytest.mark.asyncio
async def test_file_uri_read_only_mode_rejects_writes(tmp_path):
    db_path = tmp_path / "readonly.db"
    with sqlite3.connect(db_path) as setup:
        setup.execute("CREATE TABLE existing (value TEXT)")
    async with connect(f"file:{db_path}?mode=ro") as db:
        assert await db.fetch_all("SELECT name FROM sqlite_master WHERE name='existing'")
        with pytest.raises(Exception):
            await db.execute("CREATE TABLE unexpected_write (value TEXT)")
    with sqlite3.connect(db_path) as verify:
        assert verify.execute(
            "SELECT name FROM sqlite_master WHERE name='unexpected_write'"
        ).fetchone() is None


@pytest.mark.asyncio
@pytest.mark.skipif(not os.path.isdir("/dev/fd"), reason="requires /dev/fd descriptor listing")
async def test_closing_many_unique_database_pools_releases_file_descriptors(tmp_path):
    gc.collect()
    baseline = len(os.listdir("/dev/fd"))
    for index in range(12):
        db = connect(tmp_path / f"database-{index}.db")
        await db.fetch_all("SELECT 1")
        await db.close()
        del db
        gc.collect()
    assert len(os.listdir("/dev/fd")) <= baseline + 3


def test_connect_memory_rejects_empty_name():
    with pytest.raises(ValueError, match="name must be a non-empty string"):
        connect_memory(name="")
