"""Stress and load tests for rapsqlite.

Tests high concurrency, long-running operations, memory leaks, and heavy load scenarios.
"""

import asyncio
import pytest
import gc

from rapsqlite import Connection, connect


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_high_concurrency_operations(test_db):
    """Test high concurrency scenarios (100+ concurrent operations)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def operation_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]  # type: ignore[attr-defined]
            # Read operation
            rows = await db.fetch_all("SELECT 1")
            assert len(rows) == 1
            return worker_id

    # Run 100 concurrent operations
    results = await asyncio.gather(*[operation_worker(i) for i in range(100)])
    assert len(results) == 100
    assert set(results) == set(range(100))


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_many_small_operations(test_db):
    """Test many small operations vs few large operations."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    # Many small operations
    for i in range(1000):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

    # Verify all inserted
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1000


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_large_result_set(test_db):
    """Test large result sets (10K+ rows)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert 10K rows
        params = [[i] for i in range(10000)]
        await db.execute_many("INSERT INTO t (value) VALUES (?)", params)

        # Fetch all rows
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 10000
        assert rows[0][1] == 0
        assert rows[9999][1] == 9999


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_connection_pool_heavy_load(test_db):
    """Test connection pool under heavy load."""
    async with connect(test_db) as db:
        db.pool_size = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def pool_worker(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            db.pool_size = 5
            await db.execute("INSERT INTO t (value) VALUES (?)", [worker_id])
            # Small delay to simulate work
            await asyncio.sleep(0.01)
            rows = await db.fetch_all(
                "SELECT value FROM t WHERE value = ?", [worker_id]
            )
            return len(rows) == 1

    # Run 50 workers with pool size of 5
    tasks = [pool_worker(i) for i in range(50)]
    results = await asyncio.gather(*tasks, return_exceptions=True)

    # Most should succeed (some may timeout)
    successes = [r for r in results if r is True]
    assert len(successes) >= 40  # At least 80% should succeed


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_memory_leak_detection(test_db):
    """Test for memory leaks with repeated operations."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value TEXT)")

    # Perform many operations and check for memory growth
    initial_objects = len(gc.get_objects())

    for i in range(1000):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            await db.execute("INSERT INTO t (value) VALUES (?)", [f"value_{i}"])
            rows = await db.fetch_all("SELECT * FROM t WHERE id = ?", [i + 1])
            assert len(rows) == 1

    # Force garbage collection
    gc.collect()

    # Check object count (shouldn't grow excessively)
    final_objects = len(gc.get_objects())
    # Allow some growth but not excessive
    growth_ratio = final_objects / initial_objects
    assert growth_ratio < 2.0, f"Possible memory leak: {growth_ratio}x object growth"


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_long_running_transaction(test_db):
    """Test long-running transaction."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    db = Connection(test_db)
    await db.begin()

    try:
        # Insert many rows in transaction
        for i in range(1000):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

        # Verify in transaction
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1000

        # Commit
        await db.commit()

        # Verify after commit
        async with connect(test_db) as db2:
            rows = await db2.fetch_all("SELECT COUNT(*) FROM t")
            assert rows[0][0] == 1000
    finally:
        await db.close()


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_repeated_prepared_statements(test_db):
    """Test repeated use of prepared statements (cache effectiveness)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert initial data
        for i in range(100):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

        # Execute same query many times (should benefit from prepared statement cache)
        for _ in range(1000):
            rows = await db.fetch_all("SELECT * FROM t WHERE value = ?", [50])
            assert len(rows) == 1
            assert rows[0][1] == 50


@pytest.mark.stress
@pytest.mark.slow
@pytest.mark.asyncio
async def test_concurrent_connections_stress(test_db):
    """Test stress with many concurrent connections."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def stress_worker(worker_id: int):
        # Each worker creates its own connection
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            for i in range(10):
                await db.execute(
                    "INSERT INTO t (value) VALUES (?)", [worker_id * 10 + i]
                )
            rows = await db.fetch_all(
                "SELECT COUNT(*) FROM t WHERE value >= ? AND value < ?",
                [worker_id * 10, (worker_id + 1) * 10],
            )
            return rows[0][0]

    # Run 20 concurrent workers
    results = await asyncio.gather(
        *[stress_worker(i) for i in range(20)], return_exceptions=True
    )

    # Most should succeed
    successes = [r for r in results if isinstance(r, int)]
    assert len(successes) >= 15  # At least 75% should succeed
