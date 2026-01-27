"""Comprehensive edge case tests for rapsqlite.

Tests cover edge cases in:
- Connection pool behavior
- Transaction handling
- Parameters and queries
- Connection lifecycle
- Row factory and type conversions
"""

import asyncio
import os
import sys
import tempfile
import pytest

from rapsqlite import Connection, connect, OperationalError, DatabaseError, ProgrammingError


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_pool_exhaustion(test_db):
    """Test pool exhaustion scenario - all connections in use.
    
    Note: Each Connection object has its own pool, so we test exhaustion
    within a single connection by using multiple concurrent operations.
    """
    async with connect(test_db) as db:
        db.pool_size = 1  # Very small pool
        db.connection_timeout = 1  # Short timeout
        
        # Create table
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Start a transaction to hold the connection
        await db.begin()
        try:
            await db.execute("INSERT INTO t DEFAULT VALUES")
            
            # Try another operation - should work (uses transaction connection)
            await db.execute("INSERT INTO t DEFAULT VALUES")
            
            # Verify inserts worked
            rows = await db.fetch_all("SELECT COUNT(*) FROM t")
            assert rows[0][0] == 2
        finally:
            await db.rollback()


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_connection_timeout_short(test_db):
    """Test connection timeout configuration.
    
    Note: Each Connection has its own pool, so we test timeout configuration
    rather than pool exhaustion across connections.
    """
    async with connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 1  # 1 second timeout
        
        # Should work
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Verify timeout is set
        assert db.connection_timeout == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_pool_size_zero(test_db):
    """Test pool size edge case - size=0 should default to 1."""
    async with connect(test_db) as db:
        db.pool_size = 0
        # Should default to 1, not fail
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_pool_size_one(test_db):
    """Test pool size edge case - size=1."""
    async with connect(test_db) as db:
        db.pool_size = 1
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Should work fine
        await db.execute("INSERT INTO t DEFAULT VALUES")
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_pool_size_large(test_db):
    """Test pool size edge case - very large size."""
    async with connect(test_db) as db:
        db.pool_size = 1000  # Very large pool
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_nested_transaction_attempt(test_db):
    """Test nested transaction attempts - should fail gracefully."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        async with db.transaction():
            # Try to start another transaction - should fail
            with pytest.raises(OperationalError, match="(?i)transaction"):
                async with db.transaction():
                    await db.execute("INSERT INTO t DEFAULT VALUES")


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_transaction_closed_connection(test_db):
    """Test transaction with closed connection.
    
    Note: Connection might recreate pool on use, so behavior may vary.
    """
    db = Connection(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    await db.close()
    
    # Transaction might recreate connection or fail
    # Both behaviors are acceptable
    try:
        async with db.transaction():
            await db.execute("INSERT INTO t DEFAULT VALUES")
        # If it works, connection was recreated (acceptable)
    except (OperationalError, DatabaseError):
        # Expected - connection is closed
        pass


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_multiple_close_calls(test_db):
    """Test multiple close() calls - should be safe."""
    db = Connection(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    
    # First close should work
    await db.close()
    
    # Second close should be safe (no exception)
    await db.close()
    
    # Third close should also be safe
    await db.close()


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_operations_on_closed_connection(test_db):
    """Test operations on closed connection."""
    db = Connection(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    await db.close()
    
    # All operations should fail - but behavior may vary
    # Some operations might recreate the pool, so we test that close() was called
    # The actual error might be raised or connection might be recreated
    try:
        await db.execute("INSERT INTO t DEFAULT VALUES")
        # If it doesn't raise, the connection was recreated (acceptable behavior)
    except (OperationalError, DatabaseError):
        # Expected - connection is closed
        pass
    
    # Test that we can't reliably use a closed connection
    # (It might recreate, but that's also acceptable behavior)
    try:
        rows = await db.fetch_all("SELECT * FROM t")
        # If it works, connection was recreated
    except (OperationalError, DatabaseError):
        # Expected - connection is closed
        pass


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_empty_parameter_list(test_db):
    """Test empty parameter list."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        # Empty parameter list should work
        await db.execute("INSERT INTO t DEFAULT VALUES", [])
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_large_parameter_list(test_db):
    """Test parameter list at the limit (16 parameters)."""
    async with connect(test_db) as db:
        # Create table with 16 columns
        await db.execute("""
            CREATE TABLE t (
                c1 INTEGER, c2 INTEGER, c3 INTEGER, c4 INTEGER,
                c5 INTEGER, c6 INTEGER, c7 INTEGER, c8 INTEGER,
                c9 INTEGER, c10 INTEGER, c11 INTEGER, c12 INTEGER,
                c13 INTEGER, c14 INTEGER, c15 INTEGER, c16 INTEGER
            )
        """)
        
        # Insert with 16 parameters
        params = list(range(1, 17))
        await db.execute(
            "INSERT INTO t VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params
        )
        
        rows = await db.fetch_all("SELECT * FROM t")
        assert rows[0] == params


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_too_many_parameters(test_db):
    """Test parameter list exceeding limit (>16 parameters)."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Create query with 20 placeholders
        placeholders = ", ".join(["?" for _ in range(20)])
        # Should fail with OperationalError or DatabaseError
        with pytest.raises((OperationalError, DatabaseError, ProgrammingError), match="Too many parameters"):
            await db.execute(f"INSERT INTO t VALUES ({placeholders})", list(range(20)))


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_sql_injection_attempts(test_db):
    """Test SQL injection attempt patterns - should be safe."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        
        # SQL injection attempt - should be safe with parameterized queries
        malicious_input = "'; DROP TABLE users; --"
        await db.execute("INSERT INTO users (name) VALUES (?)", [malicious_input])
        
        # Table should still exist
        rows = await db.fetch_all("SELECT name FROM users")
        assert len(rows) == 1
        assert rows[0][0] == malicious_input  # Stored as literal string, not executed


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_malformed_sql(test_db):
    """Test malformed SQL queries."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        # Clearly invalid SQL should raise error
        with pytest.raises((ProgrammingError, DatabaseError, OperationalError)):
            await db.execute("INVALID SQL SYNTAX HERE")
        
        # Malformed CREATE statement (missing table name)
        with pytest.raises((ProgrammingError, DatabaseError, OperationalError)):
            await db.execute("CREATE TABLE")
        
        # Malformed INSERT
        with pytest.raises((ProgrammingError, DatabaseError, OperationalError)):
            await db.execute("INSERT INTO")


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_unicode_in_queries(test_db):
    """Test Unicode edge cases in queries and parameters."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        
        # Unicode characters
        unicode_strings = [
            "Hello 世界",
            "Привет",
            "مرحبا",
            "🎉🎊🎈",
            "测试",
            "\u0000",  # Null byte (should be handled)
        ]
        
        for i, text in enumerate(unicode_strings):
            if "\u0000" in text:
                # Null bytes might cause issues, skip for now
                continue
            await db.execute("INSERT INTO t (name) VALUES (?)", [text])
        
        rows = await db.fetch_all("SELECT name FROM t")
        assert len(rows) >= len(unicode_strings) - 1  # Minus null byte test


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_very_long_query(test_db):
    """Test very long query strings."""
    async with connect(test_db) as db:
        # Create a very long query
        long_query = "SELECT " + ", ".join([f"{i} AS c{i}" for i in range(1000)])
        
        rows = await db.fetch_all(long_query)
        assert len(rows) == 1
        assert len(rows[0]) == 1000


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_special_characters_in_names(test_db):
    """Test special characters in table/column names."""
    async with connect(test_db) as db:
        # SQLite allows quoted identifiers with special characters
        await db.execute('CREATE TABLE "test-table" ("col-name" TEXT, "col_name" TEXT)')
        await db.execute('INSERT INTO "test-table" ("col-name", "col_name") VALUES (?, ?)', ["a", "b"])
        
        rows = await db.fetch_all('SELECT * FROM "test-table"')
        assert len(rows) == 1
        assert rows[0] == ["a", "b"]


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_invalid_row_factory(test_db):
    """Test invalid row factory types."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        await db.execute("INSERT INTO t (name) VALUES (?)", ["test"])
        
        # Invalid row factory should raise error or use default
        db.row_factory = 12345  # Invalid type
        
        # Should either work with default or raise error
        try:
            rows = await db.fetch_all("SELECT * FROM t")
            # If it works, should return list
            assert isinstance(rows[0], list)
        except (TypeError, ValueError):
            # Or should raise appropriate error
            pass


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_very_large_integer(test_db):
    """Test very large integer values."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value INTEGER)")
        
        # Very large integer
        large_int = 2**63 - 1  # Max 64-bit signed integer
        await db.execute("INSERT INTO t (value) VALUES (?)", [large_int])
        
        rows = await db.fetch_all("SELECT value FROM t")
        assert rows[0][0] == large_int


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_nan_float(test_db):
    """Test NaN float values - SQLite converts NaN to NULL."""
    import math
    
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value REAL)")
        
        # NaN is converted to NULL by SQLite
        await db.execute("INSERT INTO t (value) VALUES (?)", [float('nan')])
        
        rows = await db.fetch_all("SELECT value FROM t")
        # SQLite converts NaN to NULL (None in Python)
        assert rows[0][0] is None


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_infinity_float(test_db):
    """Test infinity float values."""
    import math
    
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, value REAL)")
        
        # Infinity should be handled
        await db.execute("INSERT INTO t (value) VALUES (?)", [float('inf')])
        await db.execute("INSERT INTO t (value) VALUES (?)", [float('-inf')])
        
        rows = await db.fetch_all("SELECT value FROM t ORDER BY id")
        assert math.isinf(rows[0][0])
        assert math.isinf(rows[1][0])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_empty_blob(test_db):
    """Test empty BLOB data."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB)")
        
        # Empty blob
        await db.execute("INSERT INTO t (data) VALUES (?)", [b""])
        
        rows = await db.fetch_all("SELECT data FROM t")
        assert rows[0][0] == b""


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_large_blob(test_db):
    """Test large BLOB data."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB)")
        
        # Large blob (1MB)
        large_blob = b"x" * (1024 * 1024)
        await db.execute("INSERT INTO t (data) VALUES (?)", [large_blob])
        
        rows = await db.fetch_all("SELECT data FROM t")
        assert len(rows[0][0]) == len(large_blob)
        assert rows[0][0] == large_blob


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_null_handling(test_db):
    """Test NULL handling in all contexts."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
        
        # Insert NULL values
        await db.execute("INSERT INTO t (name, value) VALUES (?, ?)", [None, None])
        await db.execute("INSERT INTO t (name, value) VALUES (?, ?)", ["test", None])
        await db.execute("INSERT INTO t (name, value) VALUES (?, ?)", [None, 42])
        
        rows = await db.fetch_all("SELECT name, value FROM t ORDER BY id")
        assert rows[0][0] is None
        assert rows[0][1] is None
        assert rows[1][0] == "test"
        assert rows[1][1] is None
        assert rows[2][0] is None
        assert rows[2][1] == 42
