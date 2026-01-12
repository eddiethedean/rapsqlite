"""Test rapsqlite async functionality."""

import pytest
import tempfile
import os
import sys

from rapsqlite import Connection


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
        assert rows[0][1] == "100", f"Expected value '100', got '{rows[0][1]}'"
    finally:
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
