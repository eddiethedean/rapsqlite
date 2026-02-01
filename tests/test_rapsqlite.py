"""Test rapsqlite async functionality."""

import os

import pytest

from rapsqlite import Connection, connect

pytestmark = [pytest.mark.unit]


@pytest.mark.asyncio
async def test_create_table(test_db):
    """Test creating a table."""
    async with connect(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        # If no exception is raised, test passes
        assert os.path.exists(test_db), "Database file should exist"


@pytest.mark.asyncio
async def test_insert_data(test_db):
    """Test inserting data into a table."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
            )
            await conn.execute(
                "INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')"
            )
            await conn.execute(
                "INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')"
            )


@pytest.mark.asyncio
async def test_fetch_all(test_db):
    """Test fetching all rows from a table."""
    async with connect(test_db) as conn:
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


@pytest.mark.asyncio
async def test_fetch_all_with_filter(test_db):
    """Test fetching rows with a WHERE clause."""
    async with connect(test_db) as conn:
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


@pytest.mark.asyncio
async def test_multiple_operations(test_db):
    """Test multiple database operations in sequence."""
    async with connect(test_db) as conn:
            # Create table
            await conn.execute(
                "CREATE TABLE data (id INTEGER PRIMARY KEY, value INTEGER)"
            )

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


@pytest.mark.asyncio
async def test_empty_result(test_db):
    """Test fetching from an empty table."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE empty (id INTEGER PRIMARY KEY, name TEXT)")

            rows = await conn.fetch_all("SELECT * FROM empty")
            assert len(rows) == 0, f"Expected 0 rows, got {len(rows)}"


# Type system tests
@pytest.mark.asyncio
async def test_type_integer(test_db):
    """Test INTEGER type handling."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert isinstance(rows[0][1], int), f"Expected int, got {type(rows[0][1])}"
            assert rows[0][1] == 42


@pytest.mark.asyncio
async def test_type_real(test_db):
    """Test REAL type handling."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value REAL)")
            await conn.execute("INSERT INTO test (value) VALUES (3.14)")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert isinstance(rows[0][1], float), (
                f"Expected float, got {type(rows[0][1])}"
            )
            assert abs(rows[0][1] - 3.14) < 0.001


@pytest.mark.asyncio
async def test_type_text(test_db):
    """Test TEXT type handling."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert isinstance(rows[0][1], str), f"Expected str, got {type(rows[0][1])}"
            assert rows[0][1] == "hello"


@pytest.mark.asyncio
async def test_type_null(test_db):
    """Test NULL type handling."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES (NULL)")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert rows[0][1] is None, f"Expected None, got {rows[0][1]}"


# Transaction tests
@pytest.mark.asyncio
async def test_transaction_commit(test_db):
    """Test transaction commit."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            await conn.begin()
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")
            await conn.commit()

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 2


@pytest.mark.asyncio
async def test_transaction_rollback(test_db):
    """Test transaction rollback."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            await conn.begin()
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.rollback()

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 0


@pytest.mark.asyncio
async def test_execute_many_in_transaction_explicit(test_db):
    """Regression: execute_many works with explicit begin/commit."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.begin()
            await conn.execute_many(
                "INSERT INTO test (value) VALUES (?)",
                [["a"], ["b"], ["c"]],
            )
            await conn.commit()
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 3
            assert rows[0][1] == "a"
            assert rows[1][1] == "b"
            assert rows[2][1] == "c"


@pytest.mark.asyncio
async def test_execute_many_in_transaction_context_manager(test_db):
    """Regression: execute_many works inside async with db.transaction()."""
    async with connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            async with conn.transaction():
                await conn.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["x"], ["y"], ["z"]],
                )
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 3
            assert rows[0][1] == "x"
            assert rows[1][1] == "y"
            assert rows[2][1] == "z"


# API method tests
@pytest.mark.asyncio
async def test_fetch_one(test_db):
    """Test fetch_one method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

            row = await conn.fetch_one("SELECT * FROM test WHERE id = 1")
            assert len(row) == 2
            assert row[1] == 42


@pytest.mark.asyncio
async def test_fetch_optional(test_db):
    """Test fetch_optional method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            # Test with no rows
            result = await conn.fetch_optional("SELECT * FROM test WHERE id = 999")
            assert result is None

            # Test with one row
            await conn.execute("INSERT INTO test (value) VALUES (42)")
            result = await conn.fetch_optional("SELECT * FROM test WHERE id = 1")
            assert result is not None
            assert result[1] == 42


@pytest.mark.asyncio
async def test_last_insert_rowid(test_db):
    """Test last_insert_rowid method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

            rowid = await conn.last_insert_rowid()
            assert rowid == 1


@pytest.mark.asyncio
async def test_changes(test_db):
    """Test changes method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")

            await conn.execute("UPDATE test SET value = 99 WHERE id = 1")
            changes = await conn.changes()
            assert changes == 1


# Cursor tests
@pytest.mark.asyncio
async def test_cursor_execute(test_db):
    """Test cursor execute method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            cursor = conn.cursor()
            await cursor.execute("INSERT INTO test (value) VALUES (42)")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1


@pytest.mark.asyncio
async def test_cursor_fetchone(test_db):
    """Test cursor fetchone method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (42)")

            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM test WHERE id = 1")
            row = await cursor.fetchone()
            assert row is not None
            assert row[1] == 42


@pytest.mark.asyncio
async def test_cursor_fetchall(test_db):
    """Test cursor fetchall method."""
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")

            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM test")
            rows = await cursor.fetchall()
            assert len(rows) == 2


@pytest.mark.asyncio
async def test_cursor_fetchmany(test_db):
    """Test cursor fetchmany method."""
    # Phase 2: fetchmany now supports size-based slicing
    async with connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")
            await conn.execute("INSERT INTO test (value) VALUES (3)")

            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM test")
            # First call should return 2 rows
            rows = await cursor.fetchmany(2)
            assert len(rows) == 2
            assert rows[0] == [1, 1]
            assert rows[1] == [2, 2]
            # Second call should return the remaining 1 row
            rows = await cursor.fetchmany(2)
            assert len(rows) == 1
            assert rows[0] == [3, 3]
            # Third call should return empty list
            rows = await cursor.fetchmany(2)
            assert len(rows) == 0


# Context manager tests
@pytest.mark.asyncio
async def test_connection_context_manager(test_db):
    """Test connection async context manager."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
        )
        await conn.execute("INSERT INTO test (value) VALUES (42)")

    # Connection should be closed, but we can still verify the data was written
    async with connect(test_db) as conn2:
        rows = await conn2.fetch_all("SELECT * FROM test")
        assert len(rows) == 1


@pytest.mark.asyncio
async def test_cursor_context_manager(test_db):
    """Test cursor async context manager."""
    async with connect(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
        )

        async with conn.cursor() as cursor:
            await cursor.execute("INSERT INTO test (value) VALUES (42)")

        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1


# aiosqlite compatibility tests
@pytest.mark.asyncio
async def test_connect_function(test_db):
    """Test connect() factory function (aiosqlite compatibility)."""
    async with connect(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
        )
        await conn.execute("INSERT INTO test (value) VALUES (42)")

    # Verify data
    async with connect(test_db) as conn2:
        rows = await conn2.fetch_all("SELECT * FROM test")
        assert len(rows) == 1


# Error handling tests
@pytest.mark.asyncio
async def test_integrity_error(test_db):
    """Test integrity constraint violation."""
    async with connect(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER UNIQUE)"
        )
        await conn.execute("INSERT INTO test (value) VALUES (42)")

        # Try to insert duplicate value
        with pytest.raises(Exception):  # Should raise IntegrityError
            await conn.execute("INSERT INTO test (value) VALUES (42)")


@pytest.mark.asyncio
async def test_programming_error(test_db):
    """Test programming error (invalid SQL)."""
    async with connect(test_db) as conn:
        with pytest.raises(
            Exception
        ):  # Should raise ProgrammingError or DatabaseError
            await conn.execute("INVALID SQL STATEMENT")
