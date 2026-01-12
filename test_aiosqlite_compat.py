"""Migrated aiosqlite tests for rapsqlite compatibility testing.

This file contains tests migrated from aiosqlite to validate rapsqlite's
compatibility with the aiosqlite API. Tests for features not yet implemented
are marked with pytest.skip.

Source: https://github.com/omnilib/aiosqlite/tree/main/aiosqlite/tests
"""

import asyncio
import os
import pytest
import sys
import tempfile
from pathlib import Path

from rapsqlite import Connection, connect, OperationalError


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


@pytest.fixture
def test_db():
    """Create a temporary database file for testing."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        yield db_path
    finally:
        cleanup_db(db_path)


@pytest.mark.asyncio
async def test_connection_context(test_db):
    """Test connection context manager."""
    async with connect(test_db) as db:
        assert isinstance(db, Connection)

        # Use connection methods directly
        rows = await db.fetch_all("SELECT 1, 2")
        assert rows == [[1, 2]]


@pytest.mark.asyncio
async def test_connection_locations(test_db):
    """Test connection with different location types."""
    TEST_DB = test_db

    class Fake:
        def __str__(self):
            return TEST_DB

    locs = (Path(TEST_DB), TEST_DB, Fake())

    async with connect(str(locs[0])) as db:
        await db.execute("CREATE TABLE foo (i INTEGER, k INTEGER)")
        await db.begin()
        await db.execute("INSERT INTO foo (i, k) VALUES (1, 5)")
        await db.commit()

        rows = await db.fetch_all("SELECT * FROM foo")

    for loc in locs:
        async with connect(str(loc)) as db:
            result = await db.fetch_all("SELECT * FROM foo")
            assert result == rows


@pytest.mark.asyncio
async def test_multiple_connections(test_db):
    """Test multiple concurrent connections."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE multiple_connections (i INTEGER PRIMARY KEY ASC, k INTEGER)"
        )

    async def do_one_conn(i):
        async with connect(test_db) as db:
            # Note: Parameterized queries not yet supported, use string formatting
            await db.begin()
            await db.execute(f"INSERT INTO multiple_connections (k) VALUES ({i})")
            await db.commit()

    await asyncio.gather(*[do_one_conn(i) for i in range(10)])

    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT * FROM multiple_connections")

    assert len(rows) == 10


@pytest.mark.asyncio
async def test_multiple_queries(test_db):
    """Test multiple queries on same connection.
    
    Note: In rapsqlite/SQLite, concurrent writes within a transaction cause locks.
    This test executes inserts sequentially instead of concurrently to avoid locking.
    The original aiosqlite test uses concurrent execution, which works differently
    in aiosqlite due to its threading model.
    """
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE multiple_queries (i INTEGER PRIMARY KEY ASC, k INTEGER)"
        )

        # Execute multiple inserts sequentially (concurrent writes cause SQLite locks)
        await db.begin()
        for i in range(10):
            await db.execute(f"INSERT INTO multiple_queries (k) VALUES ({i})")
        await db.commit()

    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT * FROM multiple_queries")

    assert len(rows) == 10


@pytest.mark.asyncio
async def test_context_cursor(test_db):
    """Test cursor context manager."""
    async with connect(test_db) as db:
        async with db.cursor() as cursor:
            await cursor.execute(
                "CREATE TABLE context_cursor (i INTEGER PRIMARY KEY ASC, k INTEGER)"
            )
            # Note: executemany not fully implemented, use execute in loop
            await db.begin()
            for i in range(10):
                await cursor.execute(f"INSERT INTO context_cursor (k) VALUES ({i})")
            await db.commit()

    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT * FROM context_cursor")

    assert len(rows) == 10


@pytest.mark.asyncio
async def test_fetch_all(test_db):
    """Test fetch_all method."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE test_fetch_all (i INTEGER PRIMARY KEY ASC, k INTEGER)"
        )
        await db.begin()
        await db.execute("INSERT INTO test_fetch_all (k) VALUES (10), (24), (16), (32)")
        await db.commit()

    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT k FROM test_fetch_all WHERE k < 30")
        # rapsqlite returns lists, not tuples
        assert len(rows) == 3
        assert rows[0][0] == 10
        assert rows[1][0] == 24
        assert rows[2][0] == 16


@pytest.mark.asyncio
async def test_connect_error():
    """Test connection error handling."""
    # Use a path that doesn't exist and can't be created
    # Note: In rapsqlite, connection creation succeeds but database access fails later
    bad_db = "/something/that/shouldnt/exist/test.db"
    with pytest.raises(OperationalError):
        async with connect(bad_db) as db:
            # Trigger database access to cause the error
            await db.execute("SELECT 1")


@pytest.mark.asyncio
async def test_close_twice(test_db):
    """Test closing connection twice."""
    db = Connection(test_db)

    await db.close()

    # Should not raise error
    await db.close()


@pytest.mark.asyncio
async def test_connection_await(test_db):
    """Test connection creation (rapsqlite doesn't require await for connect)."""
    # In rapsqlite, connect() returns Connection directly, not awaitable
    # But Connection() also works
    db = Connection(test_db)
    assert isinstance(db, Connection)

    rows = await db.fetch_all("SELECT 1, 2")
    assert rows == [[1, 2]]

    await db.close()


# Tests for features not yet implemented - marked as skip
@pytest.mark.skip(reason="Row factory not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_connection_properties(test_db):
    """Test connection properties (row_factory, etc.)."""
    pass


@pytest.mark.skip(reason="Parameterized queries not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_iterable_cursor(test_db):
    """Test iterable cursor with parameterized queries."""
    pass


@pytest.mark.skip(reason="enable_load_extension not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_enable_load_extension(test_db):
    """Test extension loading."""
    pass


@pytest.mark.skip(reason="set_progress_handler not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_set_progress_handler(test_db):
    """Test progress handler."""
    pass


@pytest.mark.skip(reason="create_function not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_create_function(test_db):
    """Test custom SQL functions."""
    pass


@pytest.mark.skip(reason="set_trace_callback not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_set_trace_callback(test_db):
    """Test trace callback."""
    pass


@pytest.mark.skip(reason="set_authorizer not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_set_authorizer_deny_drops(test_db):
    """Test authorizer."""
    pass


@pytest.mark.skip(reason="iterdump not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_iterdump(test_db):
    """Test database dump."""
    pass


@pytest.mark.skip(reason="backup not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_backup_aiosqlite(test_db):
    """Test backup functionality."""
    pass


@pytest.mark.skip(reason="backup not yet implemented in rapsqlite")
@pytest.mark.asyncio
async def test_backup_sqlite(test_db):
    """Test backup to sqlite3 connection."""
    pass


@pytest.mark.skip(reason="Multi-loop usage pattern not applicable to rapsqlite")
@pytest.mark.asyncio
async def test_multi_loop_usage(test_db):
    """Test multi-loop usage (aiosqlite-specific pattern)."""
    pass


@pytest.mark.skip(reason="Cursor return self pattern differs in rapsqlite")
@pytest.mark.asyncio
async def test_cursor_return_self(test_db):
    """Test cursor execute return value."""
    pass


@pytest.mark.skip(reason="Connection internal state tracking differs in rapsqlite")
@pytest.mark.asyncio
async def test_cursor_on_closed_connection(test_db):
    """Test cursor behavior on closed connection."""
    pass


@pytest.mark.skip(reason="Connection internal state tracking differs in rapsqlite")
@pytest.mark.asyncio
async def test_close_blocking_until_transaction_queue_empty(test_db):
    """Test close blocking behavior."""
    pass


@pytest.mark.skip(reason="ResourceWarning pattern differs in rapsqlite")
@pytest.mark.asyncio
async def test_emits_warning_when_left_open(test_db):
    """Test resource warning."""
    pass


@pytest.mark.skip(reason="stop() method not implemented in rapsqlite")
@pytest.mark.asyncio
async def test_stop_without_close(test_db):
    """Test stop method."""
    pass
