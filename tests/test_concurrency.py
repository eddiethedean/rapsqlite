"""Concurrency tests for rapsqlite.

Tests concurrent operations, race conditions, and lock handling.
"""

import asyncio
import pytest

from rapsqlite import Connection, connect, OperationalError, DatabaseError


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_reads(test_db):
    """Test multiple concurrent read operations."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert test data
        for i in range(10):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

    # Create multiple connections for concurrent reads
    async def read_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            rows = await db.fetch_all("SELECT * FROM t")
            assert len(rows) == 10
            return worker_id

    # Run 20 concurrent readers
    results = await asyncio.gather(*[read_worker(i) for i in range(20)])
    assert len(results) == 20
    assert set(results) == set(range(20))


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_writes_sequential(test_db):
    """Test concurrent writes - SQLite locks, so we test sequential execution."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    # SQLite doesn't support true concurrent writes, but we can test
    # that operations complete successfully when done sequentially
    async def write_worker(value: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            await db.execute("INSERT INTO t (value) VALUES (?)", [value])
            return value

    # Execute writes sequentially (not concurrently due to SQLite limitations)
    results = []
    for i in range(10):
        result = await write_worker(i)
        results.append(result)

    # Verify all writes succeeded
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT value FROM t ORDER BY value")
        assert len(rows) == 10
        assert [r[0] for r in rows] == list(range(10))


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_transactions(test_db):
    """Test multiple concurrent transactions."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def transaction_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            async with db.transaction():
                await db.execute("INSERT INTO t (value) VALUES (?)", [worker_id])
                # Verify insert
                rows = await db.fetch_all(
                    "SELECT value FROM t WHERE value = ?", [worker_id]
                )
                assert len(rows) == 1
                return worker_id

    # Run transactions sequentially (SQLite limitation)
    results = []
    for i in range(5):
        result = await transaction_worker(i)
        results.append(result)

    # Verify all transactions committed
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT value FROM t ORDER BY value")
        assert len(rows) == 5


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_pool_operations(test_db):
    """Test concurrent operations on connection pool."""
    async with connect(test_db) as db:
        db.pool_size = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def pool_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            db.pool_size = 5
            await db.execute("INSERT INTO t (value) VALUES (?)", [worker_id])
            rows = await db.fetch_all(
                "SELECT value FROM t WHERE value = ?", [worker_id]
            )
            return len(rows) == 1

    # Run operations that will use the pool
    results = await asyncio.gather(*[pool_worker(i) for i in range(10)])
    assert all(results)


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_race_condition_connection_acquisition(test_db):
    """Test race conditions in connection acquisition."""
    async with connect(test_db) as db:
        db.pool_size = 2
        db.connection_timeout = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    # Create multiple connections trying to use the pool
    async def acquire_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            db.pool_size = 2
            db.connection_timeout = 5
            # Try to execute operation
            await db.execute("INSERT INTO t (value) VALUES (?)", [worker_id])
            return worker_id

    # Run workers - some may timeout if pool is exhausted
    tasks = [acquire_worker(i) for i in range(10)]
    results = await asyncio.gather(*tasks, return_exceptions=True)

    # Some should succeed, some may timeout
    successes = [r for r in results if not isinstance(r, Exception)]
    assert len(successes) >= 2  # At least pool_size should succeed


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_database_locked_error(test_db):
    """Test database locked error handling."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    # Start a long transaction
    db1 = Connection(test_db)
    await db1.begin()
    await db1.execute("INSERT INTO t (value) VALUES (?)", [1])

    # Try to access from another connection - should handle lock gracefully
    db2 = Connection(test_db)

    # This might timeout or raise OperationalError
    try:
        await db2.execute("INSERT INTO t (value) VALUES (?)", [2])
    except (OperationalError, DatabaseError) as e:
        # Expected - database is locked
        assert "locked" in str(e).lower() or "timeout" in str(e).lower()

    # Commit first transaction
    await db1.commit()
    await db1.close()

    # Now second connection should work
    await db2.execute("INSERT INTO t (value) VALUES (?)", [2])
    await db2.close()

    # Verify both inserts succeeded
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT value FROM t ORDER BY value")
        assert len(rows) == 2


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_fetch_operations(test_db):
    """Test concurrent fetch operations."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")
        for i in range(100):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

    async def fetch_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            rows = await db.fetch_all("SELECT * FROM t WHERE value = ?", [worker_id])
            return len(rows)

    # Run 50 concurrent fetch operations
    results = await asyncio.gather(*[fetch_worker(i % 100) for i in range(50)])
    assert all(r == 1 for r in results)


@pytest.mark.concurrency
@pytest.mark.asyncio
async def test_concurrent_execute_many(test_db):
    """Test concurrent execute_many operations."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def execute_many_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            params = [[worker_id * 10 + i] for i in range(10)]
            await db.execute_many("INSERT INTO t (value) VALUES (?)", params)
            return worker_id

    # Run execute_many operations sequentially (SQLite limitation)
    results = []
    for i in range(5):
        result = await execute_many_worker(i)
        results.append(result)

    # Verify all data inserted
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 50  # 5 workers * 10 inserts each
