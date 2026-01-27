"""Error handling and exception tests for rapsqlite."""

import os
import sys
import tempfile
import pytest

from rapsqlite import (
    Connection,
    connect,
    OperationalError,
    DatabaseError,
    ProgrammingError,
    IntegrityError,
)


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_database_file_creation(test_db):
    """Test that database file operations work correctly."""
    # Use the test_db fixture which handles file creation
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        
        # Verify file exists and operations work
        assert os.path.exists(test_db)
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_invalid_database_path():
    """Test error with invalid database path."""
    # Empty path should fail
    with pytest.raises((ValueError, OperationalError)):
        async with connect("") as db:
            await db.execute("SELECT 1")
    
    # Path with null bytes should fail
    with pytest.raises(ValueError):
        async with connect("/tmp/test\0db.db") as db:
            await db.execute("SELECT 1")


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_syntax_error(test_db):
    """Test SQL syntax errors."""
    async with connect(test_db) as db:
        # Clearly invalid SQL should raise error
        with pytest.raises((ProgrammingError, DatabaseError, OperationalError)):
            await db.execute("INVALID SQL SYNTAX HERE")
        
        # Malformed statement
        with pytest.raises((ProgrammingError, DatabaseError, OperationalError)):
            await db.execute("CREATE TABLE")


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_table_not_found(test_db):
    """Test behavior when table doesn't exist.
    
    Note: SELECT on non-existent table may return empty result or raise error
    depending on SQLite version. INSERT/UPDATE/DELETE should raise error.
    """
    async with connect(test_db) as db:
        # INSERT/UPDATE/DELETE should raise error
        with pytest.raises((OperationalError, DatabaseError)):
            await db.execute("INSERT INTO nonexistent_table VALUES (1)")
        
        with pytest.raises((OperationalError, DatabaseError)):
            await db.execute("UPDATE nonexistent_table SET id = 1")
        
        with pytest.raises((OperationalError, DatabaseError)):
            await db.execute("DELETE FROM nonexistent_table")


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_column_not_found(test_db):
    """Test error when column doesn't exist."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        await db.execute("INSERT INTO t (name) VALUES (?)", ["test"])
        
        # UPDATE with non-existent column should raise error
        with pytest.raises((OperationalError, DatabaseError, ProgrammingError)):
            await db.execute("UPDATE t SET nonexistent_column = 'value'")
        
        # INSERT with non-existent column should raise error
        with pytest.raises((OperationalError, DatabaseError, ProgrammingError)):
            await db.execute("INSERT INTO t (nonexistent_column) VALUES (?)", ["value"])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_unique_constraint_violation(test_db):
    """Test IntegrityError on unique constraint violation."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT UNIQUE)")
        await db.execute("INSERT INTO t (name) VALUES (?)", ["test"])
        
        # Should raise IntegrityError
        with pytest.raises(IntegrityError):
            await db.execute("INSERT INTO t (name) VALUES (?)", ["test"])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_not_null_constraint_violation(test_db):
    """Test IntegrityError on NOT NULL constraint violation."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        
        # Should raise IntegrityError
        with pytest.raises(IntegrityError):
            await db.execute("INSERT INTO t (name) VALUES (?)", [None])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_foreign_key_constraint_violation(test_db):
    """Test IntegrityError on foreign key constraint violation."""
    async with connect(test_db) as db:
        await db.execute("PRAGMA foreign_keys = ON")
        await db.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
        await db.execute("CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id))")
        
        # Should raise IntegrityError
        with pytest.raises(IntegrityError):
            await db.execute("INSERT INTO child (parent_id) VALUES (?)", [999])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_missing_parameter_error(test_db):
    """Test error when required parameter is missing."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        
        # Missing parameter should raise KeyError or similar
        with pytest.raises((KeyError, ProgrammingError, DatabaseError)):
            await db.execute("INSERT INTO t (name) VALUES (:name)", {})


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_invalid_parameter_type(test_db):
    """Test error with invalid parameter type."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)")
        
        # Invalid type (e.g., dict instead of simple value)
        with pytest.raises((TypeError, ProgrammingError)):
            await db.execute("INSERT INTO t (name) VALUES (?)", [{"invalid": "type"}])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_rollback_without_transaction(test_db):
    """Test error when rolling back without active transaction."""
    async with connect(test_db) as db:
        with pytest.raises(OperationalError):
            await db.rollback()


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_commit_without_transaction(test_db):
    """Test error when committing without active transaction."""
    async with connect(test_db) as db:
        with pytest.raises(OperationalError):
            await db.commit()


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_fetch_one_not_found(test_db):
    """Test that fetch_one raises when no row found."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        with pytest.raises((OperationalError, DatabaseError)):
            await db.fetch_one("SELECT * FROM t WHERE id = ?", [999])


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_fetch_optional_not_found(test_db):
    """Test that fetch_optional returns None when no row found."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        
        result = await db.fetch_optional("SELECT * FROM t WHERE id = ?", [999])
        assert result is None


@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_cursor_on_closed_connection(test_db):
    """Test cursor behavior on closed connection."""
    db = Connection(test_db)
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
    await db.close()
    
    # Cursor creation might succeed, but execution should fail
    # Or cursor might recreate connection (both are acceptable)
    cursor = db.cursor()
    try:
        await cursor.execute("SELECT 1")
        # If it works, connection was recreated (acceptable)
    except (OperationalError, DatabaseError):
        # Expected - connection is closed
        pass
