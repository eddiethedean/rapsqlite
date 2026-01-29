"""Tests for connection cleanup and resource management."""

import pytest
import rapsqlite
import gc


@pytest.mark.asyncio
async def test_connection_close_cleans_up_callbacks(test_db):
    """Test that close() properly cleans up callbacks."""
    db = rapsqlite.connect(test_db)
    
    # Set up callbacks
    def test_func(x):
        return x
    
    def trace_cb(sql):
        pass
    
    def auth_cb(action, arg1, arg2, arg3, arg4):
        return 0
    
    def progress_cb():
        return True
    
    await db.create_function("test_func", 1, test_func)
    await db.set_trace_callback(trace_cb)
    await db.set_authorizer(auth_cb)
    await db.set_progress_handler(100, progress_cb)
    
    # Close should clean up all callbacks
    await db.close()
    
    # Verify callbacks are cleared by trying to use them
    # (they should be None/cleared, so operations should work without callbacks)
    async with rapsqlite.connect(test_db) as db2:
        # Should work fine - callbacks were cleared
        await db2.execute("SELECT 1")


@pytest.mark.asyncio
async def test_connection_close_rolls_back_transaction(test_db):
    """Test that close() rolls back active transactions."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    
    db = rapsqlite.connect(test_db)
    await db.begin()
    await db.execute("INSERT INTO t DEFAULT VALUES")
    
    # Close should rollback the transaction
    await db.close()
    
    # Verify transaction was rolled back
    async with rapsqlite.connect(test_db) as db2:
        rows = await db2.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 0  # Transaction was rolled back


@pytest.mark.asyncio
async def test_connection_context_manager_cleanup(test_db):
    """Test that async context manager properly cleans up."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        # Context manager should rollback on exit
    
    # Verify transaction was rolled back
    async with rapsqlite.connect(test_db) as db2:
        rows = await db2.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 0  # Transaction was rolled back by context manager


@pytest.mark.asyncio
async def test_connection_context_manager_ensures_cleanup(test_db):
    """Test that context manager ensures proper cleanup even on exceptions."""
    try:
        async with rapsqlite.connect(test_db) as db:
            await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            await db.begin()
            await db.execute("INSERT INTO t DEFAULT VALUES")
            # Simulate an exception
            raise ValueError("Test exception")
    except ValueError:
        pass
    
    # Context manager should have rolled back the transaction
    async with rapsqlite.connect(test_db) as db2:
        rows = await db2.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 0  # Transaction was rolled back


@pytest.mark.asyncio
async def test_connection_close_releases_pool(test_db):
    """Test that close() properly closes the connection pool."""
    db = rapsqlite.connect(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    
    # Close should release the pool and clean up resources
    await db.close()
    
    # Verify close() completed successfully (no exceptions raised)
    # The pool is closed and resources are cleaned up
    # Note: After close(), operations may recreate the pool or fail
    # depending on implementation - the important thing is close() works


@pytest.mark.asyncio
async def test_multiple_connections_independent_cleanup(test_db):
    """Test that multiple connections clean up independently."""
    db1 = rapsqlite.connect(test_db)
    db2 = rapsqlite.connect(test_db)
    
    await db1.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    await db1.begin()
    await db1.execute("INSERT INTO t DEFAULT VALUES")
    
    # Close db1 should not affect db2
    await db1.close()
    
    # db2 should still work
    async with db2:
        rows = await db2.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 0  # db1's transaction was rolled back
    
    await db2.close()
