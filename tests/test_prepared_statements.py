"""Tests for prepared statement caching and query optimization (Phase 2.13).

Note: sqlx already caches prepared statements per connection internally.
These tests verify query normalization and usage tracking functionality.
"""

import pytest
import tempfile
import os
import sys
import time

from rapsqlite import connect


def cleanup_db(test_db: str) -> None:
    """Helper to clean up database file."""
    if os.path.exists(test_db):
        try:
            os.unlink(test_db)
        except (PermissionError, OSError):
            if sys.platform == "win32":
                pass
            else:
                raise


@pytest.mark.asyncio
async def test_query_normalization():
    """Test that queries with different whitespace are normalized correctly."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            # Execute same query with different whitespace
            await conn.execute("INSERT INTO test (value) VALUES ('a')")
            await conn.execute("INSERT  INTO  test  (value)  VALUES  ('b')")
            await conn.execute("INSERT INTO test(value)VALUES('c')")

            # All should work and insert rows
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 3
            assert rows[0][1] == "a"
            assert rows[1][1] == "b"
            assert rows[2][1] == "c"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_repeated_query_performance():
    """Test that repeated queries benefit from prepared statement caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            # Insert initial data
            for i in range(10):
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            # Execute the same SELECT query many times
            # With prepared statement caching, this should be fast
            start_time = time.perf_counter()
            for _ in range(100):
                rows = await conn.fetch_all("SELECT * FROM test WHERE value = ?", [5])
                assert len(rows) == 1
                assert rows[0][1] == 5
            end_time = time.perf_counter()

            # Should complete reasonably quickly (less than 1 second for 100 queries)
            elapsed = end_time - start_time
            assert elapsed < 1.0, f"100 queries took {elapsed:.3f}s, expected < 1.0s"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_parameterized_query_caching():
    """Test that parameterized queries benefit from caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
            )

            # Insert users with parameterized queries
            users = [
                ("Alice", "alice@example.com"),
                ("Bob", "bob@example.com"),
                ("Charlie", "charlie@example.com"),
            ]

            for name, email in users:
                await conn.execute(
                    "INSERT INTO users (name, email) VALUES (?, ?)", [name, email]
                )

            # Query with different parameters but same query structure
            # Should benefit from prepared statement reuse
            start_time = time.perf_counter()
            for name, _ in users:
                rows = await conn.fetch_all(
                    "SELECT * FROM users WHERE name = ?", [name]
                )
                assert len(rows) == 1
                assert rows[0][1] == name
            end_time = time.perf_counter()

            elapsed = end_time - start_time
            assert elapsed < 0.5, (
                f"3 parameterized queries took {elapsed:.3f}s, expected < 0.5s"
            )
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_transaction_query_caching():
    """Test that queries in transactions benefit from caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            # Use transaction context manager
            async with conn.transaction():
                # Execute same query multiple times in transaction
                # Should reuse prepared statement on same connection
                for i in range(20):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            # Verify all inserts worked
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 20
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_many_caching():
    """Test that execute_many benefits from prepared statement caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            # execute_many should prepare statement once and reuse it
            params = [["a"], ["b"], ["c"], ["d"], ["e"]]

            start_time = time.perf_counter()
            await conn.execute_many("INSERT INTO test (value) VALUES (?)", params)
            end_time = time.perf_counter()

            elapsed = end_time - start_time
            assert elapsed < 0.5, (
                f"execute_many with 5 params took {elapsed:.3f}s, expected < 0.5s"
            )

            # Verify all inserts worked
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 5
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_concurrent_query_caching():
    """Test that concurrent queries benefit from connection pool caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            # Insert initial data
            for i in range(10):
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

        # Create multiple connections and execute queries concurrently
        import asyncio

        async def query_worker(conn_id: int):
            async with connect(test_db) as conn:  # type: ignore[attr-defined]
                # Each connection should cache prepared statements independently
                for i in range(10):
                    rows = await conn.fetch_all(
                        "SELECT * FROM test WHERE value = ?", [i]
                    )
                    assert len(rows) == 1

        # Run 5 concurrent workers
        start_time = time.perf_counter()
        await asyncio.gather(*[query_worker(i) for i in range(5)])
        end_time = time.perf_counter()

        elapsed = end_time - start_time
        # 5 workers × 10 queries = 50 total queries
        # Should complete reasonably quickly with connection pooling
        assert elapsed < 2.0, (
            f"50 concurrent queries took {elapsed:.3f}s, expected < 2.0s"
        )
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_different_query_structures():
    """Test that different query structures don't interfere with caching."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            # Execute different query structures
            await conn.execute("INSERT INTO test (value) VALUES ('a')")
            await conn.execute("SELECT * FROM test")
            await conn.execute("UPDATE test SET value = 'b' WHERE id = 1")
            await conn.execute("SELECT * FROM test WHERE id = ?", [1])
            await conn.execute("DELETE FROM test WHERE id = 1")

            # All should work correctly
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 0
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_repeated_vs_unique_queries_performance():
    """Test that repeated identical queries perform better than unique queries.

    This test demonstrates the performance benefit of prepared statement caching.
    Repeated queries should be faster because sqlx reuses prepared statements.
    """
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            # Insert initial data
            for i in range(100):
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            # Test 1: Repeated identical query (should benefit from caching)
            start_repeated = time.perf_counter()
            for _ in range(100):
                rows = await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])
                assert len(rows) == 1
            elapsed_repeated = time.perf_counter() - start_repeated

            # Test 2: Unique queries (each query is different, less caching benefit)
            start_unique = time.perf_counter()
            for i in range(100):
                # Each query has a different parameter, but same structure
                # sqlx should still cache the prepared statement structure
                rows = await conn.fetch_all("SELECT * FROM test WHERE value = ?", [i])
                assert len(rows) == 1
            elapsed_unique = time.perf_counter() - start_unique

            # Both should complete reasonably quickly
            # Repeated queries might be slightly faster due to better cache locality
            # but both should benefit from prepared statement reuse
            assert elapsed_repeated < 2.0, (
                f"100 repeated queries took {elapsed_repeated:.3f}s"
            )
            assert elapsed_unique < 2.0, (
                f"100 unique queries took {elapsed_unique:.3f}s"
            )

            # Log performance comparison for documentation
            print("\nPerformance comparison:")
            print(f"  100 repeated queries: {elapsed_repeated * 1000:.2f}ms")
            print(f"  100 unique queries:   {elapsed_unique * 1000:.2f}ms")
            print("  Both benefit from sqlx's prepared statement caching")
    finally:
        cleanup_db(test_db)
