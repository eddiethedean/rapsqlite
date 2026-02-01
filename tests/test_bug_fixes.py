"""Tests for bug fixes: __del__ cleanup, transaction_retry edge cases, connection state, etc."""

import asyncio
import gc

import pytest

from rapsqlite import connect, transaction_retry


@pytest.mark.asyncio
async def test_transaction_retry_max_retries_zero_raises(test_db):
    """transaction_retry with max_retries=0 raises RuntimeError (no loop iterations)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        async def do_work():
            await db.execute("INSERT INTO t DEFAULT VALUES")

        with pytest.raises(RuntimeError, match="max_retries must be at least 1"):
            await transaction_retry(db, do_work, max_retries=0)


@pytest.mark.asyncio
async def test_transaction_retry_max_retries_one_succeeds(test_db):
    """transaction_retry with max_retries=1 runs exactly one attempt on success."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")

        async def do_work():
            await db.execute("INSERT INTO t (x) VALUES (?)", ["ok"])

        await transaction_retry(db, do_work, max_retries=1)
        rows = await db.fetch_all("SELECT * FROM t")
    assert rows == [[1, "ok"]]


@pytest.mark.asyncio
async def test_connection_state_cleanup_on_close(test_db):
    """total_changes and in_transaction state are cleaned up when connection is closed."""
    db = connect(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    # Access properties to populate _connection_state
    _ = db.total_changes
    _ = db.in_transaction
    await db.close()
    # After close, _cleanup_conn_state should have run; a new connection should work
    async with connect(test_db) as db2:
        rows = await db2.fetch_all("SELECT 1")
    assert rows == [[1]]


@pytest.mark.asyncio
async def test_concurrent_total_changes_in_transaction_access(test_db):
    """Concurrent access to total_changes and in_transaction is thread-safe (no race)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x INT)")

        async def reader():
            for _ in range(50):
                _ = db.total_changes
                _ = db.in_transaction
                await asyncio.sleep(0)

        # Run many readers concurrently to stress the lock on _connection_state
        await asyncio.gather(*[reader() for _ in range(10)])


@pytest.mark.asyncio
async def test_connection_gc_cleanup_does_not_leak(test_db):
    """Abandoning connections without close triggers __del__ which cleans up state.

    This is best-effort; we verify that creating and discarding connections
    does not cause obvious failure when creating a new connection afterward.
    """
    for _ in range(5):
        db = connect(test_db)
        await db.execute("SELECT 1")
        # Do not call close() - let __del__ handle cleanup
        del db

    gc.collect()
    # New connection should work; if __del__ left things broken, we might see issues
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT 1")
    assert rows == [[1]]
