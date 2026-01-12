"""Test rapsqlite async functionality."""

import pytest
import tempfile
import os
import sys

from rapsqlite import (
    Connection,
    connect,
)


def cleanup_db(test_db: str) -> None:
    """Helper to clean up database file."""
    if os.path.exists(test_db):
        try:
            os.unlink(test_db)
        except (PermissionError, OSError):
            # On Windows, database files may still be locked by SQLite
            # This is a cleanup issue, not a test failure
            if sys.platform == "win32":
                pass
            else:
                raise


@pytest.mark.asyncio
async def test_create_table():
    """Test creating a table."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        # If no exception is raised, test passes
        assert os.path.exists(test_db), "Database file should exist"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_insert_data():
    """Test inserting data into a table."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')"
        )
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_fetch_all():
    """Test fetching all rows from a table."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')"
        )

        rows = await conn.fetch_all("SELECT * FROM users")
        assert len(rows) == 2, f"Expected 2 rows, got {len(rows)}"
        assert len(rows[0]) == 3, f"Expected 3 columns, got {len(rows[0])}"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_fetch_all_with_filter():
    """Test fetching rows with a WHERE clause."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')"
        )
        await conn.execute(
            "INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')"
        )

        rows = await conn.fetch_all("SELECT * FROM users WHERE name = 'Alice'")
        assert len(rows) == 1, f"Expected 1 row, got {len(rows)}"
        assert rows[0][1] == "Alice", f"Expected name 'Alice', got '{rows[0][1]}'"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_multiple_operations():
    """Test multiple database operations in sequence."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        # Create table
        await conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert multiple rows
        for i in range(5):
            await conn.execute(f"INSERT INTO data (value) VALUES ({i})")

        # Fetch all
        rows = await conn.fetch_all("SELECT * FROM data")
        assert len(rows) == 5, f"Expected 5 rows, got {len(rows)}"

        # Update
        await conn.execute("UPDATE data SET value = 100 WHERE id = 1")

        # Fetch updated row
        rows = await conn.fetch_all("SELECT * FROM data WHERE id = 1")
        assert len(rows) == 1, f"Expected 1 row, got {len(rows)}"
        assert rows[0][1] == 100, f"Expected value 100, got '{rows[0][1]}'"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_empty_result():
    """Test fetching from an empty table."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE empty (id INTEGER PRIMARY KEY, name TEXT)")

        rows = await conn.fetch_all("SELECT * FROM empty")
        assert len(rows) == 0, f"Expected 0 rows, got {len(rows)}"
    finally:
        cleanup_db(test_db)


# Type system tests
@pytest.mark.asyncio
async def test_type_integer():
    """Test INTEGER type handling."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
        assert isinstance(rows[0][1], int), f"Expected int, got {type(rows[0][1])}"
        assert rows[0][1] == 42
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_type_real():
    """Test REAL type handling."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value REAL)")
        await conn.execute("INSERT INTO test (value) VALUES (3.14)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
        assert isinstance(rows[0][1], float), f"Expected float, got {type(rows[0][1])}"
        assert abs(rows[0][1] - 3.14) < 0.001
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_type_text():
    """Test TEXT type handling."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (value) VALUES ('hello')")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
        assert isinstance(rows[0][1], str), f"Expected str, got {type(rows[0][1])}"
        assert rows[0][1] == "hello"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_type_null():
    """Test NULL type handling."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (value) VALUES (NULL)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
        assert rows[0][1] is None, f"Expected None, got {rows[0][1]}"
    finally:
        cleanup_db(test_db)


# Transaction tests
@pytest.mark.asyncio
async def test_transaction_commit():
    """Test transaction commit."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

        await conn.begin()
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.execute("INSERT INTO test (value) VALUES (2)")
        await conn.commit()

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 2
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_transaction_rollback():
    """Test transaction rollback."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

        await conn.begin()
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.rollback()

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 0
    finally:
        cleanup_db(test_db)


# API method tests
@pytest.mark.asyncio
async def test_fetch_one():
    """Test fetch_one method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        row = await conn.fetch_one("SELECT * FROM test WHERE id = 1")
        assert len(row) == 2
        assert row[1] == 42
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_fetch_optional():
    """Test fetch_optional method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

        # Test with no rows
        result = await conn.fetch_optional("SELECT * FROM test WHERE id = 999")
        assert result is None

        # Test with one row
        await conn.execute("INSERT INTO test (value) VALUES (42)")
        result = await conn.fetch_optional("SELECT * FROM test WHERE id = 1")
        assert result is not None
        assert result[1] == 42
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_last_insert_rowid():
    """Test last_insert_rowid method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        rowid = await conn.last_insert_rowid()
        assert rowid == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_changes():
    """Test changes method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.execute("INSERT INTO test (value) VALUES (2)")

        await conn.execute("UPDATE test SET value = 99 WHERE id = 1")
        changes = await conn.changes()
        assert changes == 1
    finally:
        cleanup_db(test_db)


# Cursor tests
@pytest.mark.asyncio
async def test_cursor_execute():
    """Test cursor execute method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

        cursor = conn.cursor()
        await cursor.execute("INSERT INTO test (value) VALUES (42)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_fetchone():
    """Test cursor fetchone method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        cursor = conn.cursor()
        await cursor.execute("SELECT * FROM test WHERE id = 1")
        row = await cursor.fetchone()
        assert row is not None
        assert row[1] == 42
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_fetchall():
    """Test cursor fetchall method."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.execute("INSERT INTO test (value) VALUES (2)")

        cursor = conn.cursor()
        await cursor.execute("SELECT * FROM test")
        rows = await cursor.fetchall()
        assert len(rows) == 2
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_fetchmany():
    """Test cursor fetchmany method."""
    # Note: For Phase 1, fetchmany returns all rows (fetch_all behavior)
    # Proper size-based slicing will be implemented in Phase 2
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.execute("INSERT INTO test (value) VALUES (2)")
        await conn.execute("INSERT INTO test (value) VALUES (3)")

        cursor = conn.cursor()
        await cursor.execute("SELECT * FROM test")
        rows = await cursor.fetchmany(2)
        # Phase 1: fetchmany returns all rows (same as fetch_all)
        assert len(rows) == 3
        assert rows[0] == [1, 1]
        assert rows[1] == [2, 2]
        assert rows[2] == [3, 3]
    finally:
        cleanup_db(test_db)


# Context manager tests
@pytest.mark.asyncio
async def test_connection_context_manager():
    """Test connection async context manager."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with Connection(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

        # Connection should be closed, but we can still verify the data was written
        conn2 = Connection(test_db)
        rows = await conn2.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_context_manager():
    """Test cursor async context manager."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

        async with conn.cursor() as cursor:
            await cursor.execute("INSERT INTO test (value) VALUES (42)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
    finally:
        cleanup_db(test_db)


# aiosqlite compatibility tests
@pytest.mark.asyncio
async def test_connect_function():
    """Test connect() factory function (aiosqlite compatibility)."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

        # Verify data
        async with connect(test_db) as conn2:
            rows = await conn2.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
    finally:
        cleanup_db(test_db)


# Error handling tests
@pytest.mark.asyncio
async def test_integrity_error():
    """Test integrity constraint violation."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER UNIQUE)"
        )
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        # Try to insert duplicate value
        with pytest.raises(Exception):  # Should raise IntegrityError
            await conn.execute("INSERT INTO test (value) VALUES (42)")
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_programming_error():
    """Test programming error (invalid SQL)."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        conn = Connection(test_db)
        with pytest.raises(Exception):  # Should raise ProgrammingError or DatabaseError
            await conn.execute("INVALID SQL STATEMENT")
    finally:
        cleanup_db(test_db)
