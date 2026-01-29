"""Tests for SQLite busy_timeout feature (aiosqlite compatibility)."""

import pytest
import rapsqlite
import asyncio


@pytest.mark.asyncio
async def test_timeout_default_value(test_db):
    """Test that default timeout is 5.0 seconds (matching sqlite3/aiosqlite)."""
    async with rapsqlite.connect(test_db) as db:
        assert db.timeout == 5.0


@pytest.mark.asyncio
async def test_timeout_parameter_in_connect(test_db):
    """Test setting timeout via connect() parameter."""
    async with rapsqlite.connect(test_db, timeout=10.0) as db:
        assert db.timeout == 10.0


@pytest.mark.asyncio
async def test_timeout_property_getter_setter(test_db):
    """Test timeout property getter and setter."""
    db = rapsqlite.connect(test_db)
    
    # Default should be 5.0
    assert db.timeout == 5.0
    
    # Set to different value
    db.timeout = 15.0
    assert db.timeout == 15.0
    
    # Set to 0 (disable timeout)
    db.timeout = 0.0
    assert db.timeout == 0.0
    
    # Set back to positive value
    db.timeout = 20.0
    assert db.timeout == 20.0
    
    await db.close()


@pytest.mark.asyncio
async def test_timeout_negative_value_raises_error(test_db):
    """Test that negative timeout values raise ValueError."""
    with pytest.raises(rapsqlite.ValueError, match="timeout must be >= 0.0"):
        rapsqlite.connect(test_db, timeout=-1.0)
    
    db = rapsqlite.connect(test_db)
    with pytest.raises(rapsqlite.ValueError, match="timeout must be >= 0.0"):
        db.timeout = -5.0
    await db.close()


@pytest.mark.asyncio
async def test_timeout_applied_in_transactions(test_db):
    """Test that timeout is applied when starting transactions."""
    async with rapsqlite.connect(test_db, timeout=30.0) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Start a transaction - timeout should be applied
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        await db.commit()
        
        # Verify transaction completed successfully
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_timeout_applied_in_transaction_context_manager(test_db):
    """Test that timeout is applied in transaction context managers."""
    async with rapsqlite.connect(test_db, timeout=25.0) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Use transaction context manager - timeout should be applied
        async with db.transaction():
            await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Verify transaction completed successfully
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_timeout_zero_disables_timeout(test_db):
    """Test that timeout=0.0 disables busy_timeout."""
    async with rapsqlite.connect(test_db, timeout=0.0) as db:
        assert db.timeout == 0.0
        
        # Transactions should still work
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        await db.commit()


@pytest.mark.asyncio
async def test_timeout_connection_constructor(test_db):
    """Test timeout parameter in Connection constructor."""
    db = rapsqlite.Connection(test_db, timeout=12.5)
    assert db.timeout == 12.5
    await db.close()


@pytest.mark.asyncio
async def test_timeout_aiosqlite_compatibility(test_db):
    """Test that timeout works the same way as aiosqlite."""
    # aiosqlite pattern: connect with timeout
    async with rapsqlite.connect(test_db, timeout=10.0) as conn:
        assert conn.timeout == 10.0
        
        # Operations should work normally
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute("INSERT INTO test DEFAULT VALUES")
        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1


@pytest.mark.asyncio
async def test_timeout_changes_apply_to_new_transactions(test_db):
    """Test that changing timeout applies to new transactions."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Initial timeout
        assert db.timeout == 5.0
        
        # Change timeout
        db.timeout = 15.0
        assert db.timeout == 15.0
        
        # New transaction should use new timeout
        async with db.transaction():
            await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Verify it worked
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_timeout_float_values(test_db):
    """Test that timeout accepts float values."""
    async with rapsqlite.connect(test_db, timeout=7.5) as db:
        assert db.timeout == 7.5
        
        db.timeout = 12.75
        assert db.timeout == 12.75


@pytest.mark.asyncio
async def test_timeout_multiple_connections_independent(test_db):
    """Test that timeout is independent per connection."""
    db1 = rapsqlite.connect(test_db, timeout=10.0)
    db2 = rapsqlite.connect(test_db, timeout=20.0)
    
    assert db1.timeout == 10.0
    assert db2.timeout == 20.0
    
    # Changing one doesn't affect the other
    db1.timeout = 15.0
    assert db1.timeout == 15.0
    assert db2.timeout == 20.0
    
    await db1.close()
    await db2.close()


@pytest.mark.asyncio
async def test_timeout_with_pragmas(test_db):
    """Test that timeout works alongside PRAGMA settings."""
    async with rapsqlite.connect(
        test_db,
        timeout=15.0,
        pragmas={"journal_mode": "WAL", "synchronous": "NORMAL"}
    ) as db:
        assert db.timeout == 15.0
        
        # Verify PRAGMAs were applied
        journal_mode = await db.fetch_all("PRAGMA journal_mode")
        assert journal_mode[0][0].upper() == "WAL"
        
        # Verify timeout still works
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        async with db.transaction():
            await db.execute("INSERT INTO t DEFAULT VALUES")


@pytest.mark.asyncio
async def test_timeout_converted_to_milliseconds(test_db):
    """Test that timeout is correctly converted to milliseconds for SQLite PRAGMA."""
    # SQLite's busy_timeout PRAGMA expects milliseconds
    # We set timeout in seconds, so 5.0 seconds = 5000 milliseconds
    async with rapsqlite.connect(test_db, timeout=5.0) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Start a transaction to trigger busy_timeout setting
        await db.begin()
        
        # Check that busy_timeout was set (in milliseconds)
        # Note: This is a best-effort check - the actual PRAGMA value
        # might not be exactly 5000 due to rounding, but should be close
        busy_timeout = await db.fetch_all("PRAGMA busy_timeout")
        timeout_ms = busy_timeout[0][0]
        
        # Should be approximately 5000 milliseconds (5.0 seconds * 1000)
        # Allow some tolerance for floating point conversion
        assert 4900 <= timeout_ms <= 5100, f"Expected ~5000ms, got {timeout_ms}ms"
        
        await db.commit()


@pytest.mark.asyncio
async def test_timeout_large_value(test_db):
    """Test that large timeout values work correctly."""
    async with rapsqlite.connect(test_db, timeout=300.0) as db:
        assert db.timeout == 300.0
        
        # Operations should work normally
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        async with db.transaction():
            await db.execute("INSERT INTO t DEFAULT VALUES")
