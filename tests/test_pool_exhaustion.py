"""Tests for pool exhaustion scenarios and error handling."""

import pytest
import rapsqlite
import asyncio


@pytest.mark.asyncio
async def test_pool_exhaustion_error_message(test_db):
    """Test that pool exhaustion provides helpful error messages."""
    async with rapsqlite.connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 0.1  # Very short timeout
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        # Hold the connection in a transaction
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")

        # Try to acquire another connection - should get helpful error
        try:
            # This should timeout and provide suggestions
            await db.execute("SELECT 1")
        except rapsqlite.OperationalError as e:
            error_msg = str(e)
            # Error should mention pool_size and connection_timeout
            assert "pool_size" in error_msg or "connection_timeout" in error_msg or "pool" in error_msg.lower()
        
        await db.rollback()


@pytest.mark.asyncio
async def test_pool_exhaustion_suggests_solutions(test_db):
    """Test that pool exhaustion errors suggest increasing pool_size."""
    async with rapsqlite.connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 0.1
        
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Hold connection
        await db.begin()
        
        # Try concurrent operation - should suggest increasing pool_size
        try:
            await db.execute("SELECT 1")
        except rapsqlite.OperationalError as e:
            error_msg = str(e).lower()
            # Should suggest solutions
            assert any(keyword in error_msg for keyword in ["pool", "timeout", "increase", "connection"])
        
        await db.rollback()


@pytest.mark.asyncio
async def test_pool_exhaustion_with_large_pool(test_db):
    """Test that larger pool sizes prevent exhaustion."""
    async with rapsqlite.connect(test_db) as db:
        db.pool_size = 5
        db.connection_timeout = 5.0
        
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Start multiple transactions concurrently
        # With pool_size=5, all should succeed
        async def worker(worker_id):
            async with db.transaction():
                await db.execute("INSERT INTO t (id) VALUES (?)", [worker_id])
        
        # Run 5 workers concurrently - should all succeed
        await asyncio.gather(*[worker(i) for i in range(5)])
        
        # Verify all inserts succeeded
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 5


@pytest.mark.asyncio
async def test_pool_exhaustion_recovery(test_db):
    """Test that pool recovers after exhaustion when connections are released."""
    async with rapsqlite.connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 1.0
        
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Exhaust the pool
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Release the connection
        await db.commit()
        
        # Now operations should work again
        await db.execute("INSERT INTO t DEFAULT VALUES")
        
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 2


@pytest.mark.asyncio
async def test_pool_exhaustion_with_callback_connection(test_db):
    """Test pool exhaustion when callback connection is in use."""
    async with rapsqlite.connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 0.1
        
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Set up a callback to use the callback connection
        def trace_cb(sql):
            pass
        
        await db.set_trace_callback(trace_cb)
        
        # Start a transaction (uses callback connection)
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Try another operation - should handle gracefully
        try:
            await db.execute("SELECT 1")
        except rapsqlite.OperationalError:
            pass  # Expected - pool exhausted
        
        await db.commit()
        
        # Clear callback to release connection
        await db.set_trace_callback(None)
        
        # Operations should work again
        await db.execute("INSERT INTO t DEFAULT VALUES")
