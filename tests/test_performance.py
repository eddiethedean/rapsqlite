"""Performance regression tests for rapsqlite.

Tests baseline performance metrics and detects performance regressions.
"""

import asyncio
import time
import pytest

from rapsqlite import connect


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_query_execution_time(test_db):
    """Test that query execution time is reasonable."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert test data
        for i in range(100):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

        # Measure query time
        start = time.perf_counter()
        for _ in range(100):
            rows = await db.fetch_all("SELECT * FROM t WHERE value = ?", [50])
            assert len(rows) == 1
        elapsed = time.perf_counter() - start

        # Should complete 100 queries in reasonable time (< 1 second)
        assert elapsed < 1.0, f"100 queries took {elapsed:.3f}s, expected < 1.0s"


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_connection_pool_performance(test_db):
    """Test connection pool performance."""
    async with connect(test_db) as db:
        db.pool_size = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

    async def pool_operation(worker_id: int):
        async with connect(test_db) as db:  # type: ignore[attr-defined]
            db.pool_size = 5
            await db.execute("INSERT INTO t (value) VALUES (?)", [worker_id])
            rows = await db.fetch_all(
                "SELECT value FROM t WHERE value = ?", [worker_id]
            )
            return len(rows) == 1

    # Measure pool performance
    start = time.perf_counter()
    results = await asyncio.gather(*[pool_operation(i) for i in range(50)])
    elapsed = time.perf_counter() - start

    assert all(results)
    # Should complete 50 operations in reasonable time (< 5.0 seconds)
    # Allow extra time for CI environments which may be slower, especially Python 3.14
    assert elapsed < 5.0, f"50 pool operations took {elapsed:.3f}s, expected < 5.0s"


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_prepared_statement_cache_performance(test_db):
    """Test prepared statement cache effectiveness."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert test data
        for i in range(100):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

        # First run (no cache)
        start1 = time.perf_counter()
        for i in range(100):
            rows = await db.fetch_all("SELECT * FROM t WHERE value = ?", [i % 100])
            assert len(rows) == 1
        elapsed1 = time.perf_counter() - start1

        # Second run (with cache)
        start2 = time.perf_counter()
        for i in range(100):
            rows = await db.fetch_all("SELECT * FROM t WHERE value = ?", [i % 100])
            assert len(rows) == 1
        elapsed2 = time.perf_counter() - start2

        # Cached queries should be faster (or at least not slower)
        # Allow some variance
    # Allow up to 1.3x for CI variability (macOS runners can be slower)
    assert elapsed2 <= elapsed1 * 1.3, (
        f"Cached queries ({elapsed2:.3f}s) should be similar to first run ({elapsed1:.3f}s)"
    )


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_execute_many_performance(test_db):
    """Test execute_many performance."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Measure execute_many time
        params = [[i] for i in range(1000)]

        start = time.perf_counter()
        await db.execute_many("INSERT INTO t (value) VALUES (?)", params)
        elapsed = time.perf_counter() - start

        # Should complete 1000 inserts in reasonable time (< 2.0 seconds)
        # Allow extra time for CI environments which may be slower
        assert elapsed < 2.0, f"1000 inserts took {elapsed:.3f}s, expected < 2.0s"

        # Verify all inserted
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1000


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_large_result_set_performance(test_db):
    """Test performance with large result sets."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert large dataset
        params = [[i] for i in range(10000)]
        await db.execute_many("INSERT INTO t (value) VALUES (?)", params)

        # Measure fetch time
        start = time.perf_counter()
        rows = await db.fetch_all("SELECT * FROM t")
        elapsed = time.perf_counter() - start

        assert len(rows) == 10000
        # Should fetch 10K rows in reasonable time (< 2 seconds)
        assert elapsed < 2.0, f"Fetching 10K rows took {elapsed:.3f}s, expected < 2.0s"


@pytest.mark.performance
@pytest.mark.slow
@pytest.mark.asyncio
async def test_transaction_performance(test_db):
    """Test transaction performance."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")

        # Measure transaction time
        start = time.perf_counter()
        async with db.transaction():
            for i in range(1000):
                await db.execute("INSERT INTO t (value) VALUES (?)", [i])
        elapsed = time.perf_counter() - start

        # Should complete transaction in reasonable time (< 1 second)
        assert elapsed < 1.0, (
            f"Transaction with 1000 inserts took {elapsed:.3f}s, expected < 1.0s"
        )

        # Verify all inserted
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1000
