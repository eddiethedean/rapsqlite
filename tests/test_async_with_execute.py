"""Tests for async with db.execute() pattern (aiosqlite compatibility)."""

import pytest
import tempfile
import os
import sys

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
async def test_async_with_execute_select():
    """Test async with db.execute() for SELECT queries."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            # For non-SELECT queries, use the old pattern (execute immediately)
            # Note: execute() now returns ExecuteContextManager, but for compatibility
            # we can still await it for non-SELECT (it will execute in __aenter__)
            # Actually, let's use the old fetch_all pattern for setup
            await conn.fetch_all(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            )
            await conn.fetch_all("INSERT INTO test (value) VALUES ('hello')")
            await conn.fetch_all("INSERT INTO test (value) VALUES ('world')")

            # Test async with db.execute() pattern for SELECT
            async with conn.execute("SELECT * FROM test ORDER BY id") as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 2
                assert rows[0][1] == "hello"
                assert rows[1][1] == "world"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_async_with_execute_insert():
    """Test async with db.execute() for INSERT queries (non-SELECT)."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            # Use async with for CREATE (executes in __aenter__)
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass

            # Non-SELECT queries can use async with pattern
            # The query executes in __aenter__, then cursor is returned
            async with conn.execute("INSERT INTO test (value) VALUES ('hello')") as _:
                # Cursor is returned but has no results for non-SELECT
                # Note: Don't call fetchall() on non-SELECT cursors as it may re-execute
                # The query is already executed in __aenter__
                pass

            # Verify the insert worked (should be 1 row, not 2)
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert rows[0][1] == "hello"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_returns_cursor():
    """Test that execute() returns ExecuteContextManager which works with async with."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass

            # execute() returns ExecuteContextManager, use async with to get cursor
            async with conn.execute("SELECT 1, 2") as cursor:
                assert cursor is not None

                # Cursor should have fetch methods
                rows = await cursor.fetchall()
                assert len(rows) == 1
                assert rows[0] == [1, 2]
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_from_execute_fetchone():
    """Test fetchone() on cursor returned from execute()."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('hello')") as _:
                pass

            async with conn.execute(
                "SELECT * FROM test WHERE value = 'hello'"
            ) as cursor:
                row = await cursor.fetchone()
                assert row is not None
                assert row[1] == "hello"

                # Should return None on subsequent calls
                row2 = await cursor.fetchone()
                assert row2 is None
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_from_execute_fetchmany():
    """Test fetchmany() on cursor returned from execute()."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            ) as _:
                pass
            for i in range(5):
                async with conn.execute(f"INSERT INTO test (value) VALUES ({i})") as _:
                    pass

            async with conn.execute("SELECT * FROM test ORDER BY id") as cursor:
                # Fetch 2 rows
                rows = await cursor.fetchmany(2)
                assert len(rows) == 2
                assert rows[0][1] == 0
                assert rows[1][1] == 1

                # Fetch 2 more
                rows = await cursor.fetchmany(2)
                assert len(rows) == 2
                assert rows[0][1] == 2
                assert rows[1][1] == 3

                # Fetch remaining
                rows = await cursor.fetchmany(2)
                assert len(rows) == 1
                assert rows[0][1] == 4
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_async_with_execute_parameterized():
    """Test async with db.execute() with parameterized queries."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('hello')") as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('world')") as _:
                pass

            # Named parameters
            async with conn.execute(
                "SELECT * FROM test WHERE value = :value", {"value": "hello"}
            ) as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 1
                assert rows[0][1] == "hello"

            # Positional parameters
            async with conn.execute(
                "SELECT * FROM test WHERE value = ?", ["world"]
            ) as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 1
                assert rows[0][1] == "world"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_async_with_execute_in_transaction():
    """Test async with db.execute() inside a transaction."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            ) as _:
                pass

            async with conn.transaction():
                async with conn.execute("INSERT INTO test (value) VALUES (1)") as _:
                    pass

                # Use execute() pattern inside transaction
                async with conn.execute("SELECT * FROM test") as cursor:
                    rows = await cursor.fetchall()
                    assert len(rows) == 1
                    assert rows[0][1] == 1

            # Verify transaction committed
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_cursor_context_manager():
    """Test that cursor from execute() works as context manager."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('hello')") as _:
                pass

            # Use execute() with async with to get cursor
            async with conn.execute("SELECT * FROM test") as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 1
                assert rows[0][1] == "hello"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_update_returns_cursor():
    """Test that UPDATE queries return cursor (even though no results)."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            ) as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES (1)") as _:
                pass

            # UPDATE should return cursor
            async with conn.execute("UPDATE test SET value = 2 WHERE id = 1") as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 0  # No results for UPDATE

            # Verify update worked
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert rows[0][1] == 2
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_delete_returns_cursor():
    """Test that DELETE queries return cursor."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            async with conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
            ) as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('hello')") as _:
                pass
            async with conn.execute("INSERT INTO test (value) VALUES ('world')") as _:
                pass

            # DELETE should return cursor
            async with conn.execute("DELETE FROM test WHERE value = 'hello'") as cursor:
                rows = await cursor.fetchall()
                assert len(rows) == 0  # No results for DELETE

            # Verify delete worked
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert rows[0][1] == "world"
    finally:
        cleanup_db(test_db)
