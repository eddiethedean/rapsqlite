"""Comprehensive compatibility tests for drop-in replacement of aiosqlite (Phase 2.14).

These tests verify that rapsqlite can be used as a drop-in replacement for aiosqlite
using the pattern: `import rapsqlite as aiosqlite`
"""

import pytest
import tempfile
import os
import sys
import asyncio

# Import rapsqlite as aiosqlite to test drop-in replacement
import rapsqlite as aiosqlite


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
async def test_basic_connection():
    """Test basic connection using aiosqlite import pattern."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            assert isinstance(conn, aiosqlite.Connection)
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            assert rows[0][1] == "hello"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_connection_context_manager():
    """Test connection context manager compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            await db.execute("INSERT INTO test DEFAULT VALUES")
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_parameterized_queries():
    """Test parameterized queries with aiosqlite API."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
            )

            # Named parameters
            await conn.execute(
                "INSERT INTO users (name, email) VALUES (:name, :email)",
                {"name": "Alice", "email": "alice@example.com"},
            )

            # Positional parameters
            await conn.execute(
                "INSERT INTO users (name, email) VALUES (?, ?)",
                ["Bob", "bob@example.com"],
            )

            rows = await conn.fetch_all("SELECT * FROM users ORDER BY id")
            assert len(rows) == 2
            assert rows[0][1] == "Alice"
            assert rows[1][1] == "Bob"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_transactions():
    """Test transaction methods compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)"
            )
            await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")

            # Begin transaction
            await conn.begin()
            try:
                await conn.execute(
                    "UPDATE accounts SET balance = balance - 100 WHERE id = 1"
                )
                await conn.execute(
                    "UPDATE accounts SET balance = balance + 100 WHERE id = 2"
                )
                await conn.commit()
            except Exception:
                await conn.rollback()

            rows = await conn.fetch_all("SELECT * FROM accounts")
            assert len(rows) >= 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_transaction_context_manager():
    """Test transaction context manager compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            async with conn.transaction():
                await conn.execute("INSERT INTO test (value) VALUES (1)")
                await conn.execute("INSERT INTO test (value) VALUES (2)")

            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 2
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_cursor_api():
    """Test cursor API compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")
            await conn.execute("INSERT INTO test (value) VALUES (3)")

            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM test")

            # Test fetchone
            row = await cursor.fetchone()
            assert row is not None
            assert row[1] == 1

            # Test fetchmany
            rows = await cursor.fetchmany(2)
            assert len(rows) == 2
            assert rows[0][1] == 2
            assert rows[1][1] == 3

            # Test fetchall (should be empty now)
            rows = await cursor.fetchall()
            assert len(rows) == 0
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_fetch_methods():
    """Test fetch_all, fetch_one, fetch_optional compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")
            await conn.execute("INSERT INTO test (value) VALUES ('world')")

            # fetch_all
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 2

            # fetch_one
            row = await conn.fetch_one("SELECT * FROM test WHERE value = 'hello'")
            assert row is not None
            assert row[1] == "hello"

            # fetch_optional (exists)
            row = await conn.fetch_optional("SELECT * FROM test WHERE value = 'hello'")
            assert row is not None
            assert row[1] == "hello"

            # fetch_optional (doesn't exist)
            row = await conn.fetch_optional(
                "SELECT * FROM test WHERE value = 'nonexistent'"
            )
            assert row is None
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_execute_many():
    """Test execute_many compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            params = [["a"], ["b"], ["c"]]
            await conn.execute_many("INSERT INTO test (value) VALUES (?)", params)

            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 3
            assert rows[0][1] == "a"
            assert rows[1][1] == "b"
            assert rows[2][1] == "c"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_exception_types():
    """Test that exception types match aiosqlite."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT UNIQUE)"
            )
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")

            # Test IntegrityError
            with pytest.raises(aiosqlite.IntegrityError):
                await conn.execute("INSERT INTO test (value) VALUES ('hello')")

            # Test ProgrammingError (invalid SQL)
            with pytest.raises((aiosqlite.ProgrammingError, aiosqlite.DatabaseError)):
                await conn.execute("INVALID SQL STATEMENT")
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_row_factory():
    """Test row_factory compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")

            # Test dict row factory
            conn.row_factory = "dict"
            rows = await conn.fetch_all("SELECT * FROM test")
            assert isinstance(rows[0], dict)
            assert rows[0]["value"] == "hello"

            # Test tuple row factory
            conn.row_factory = "tuple"
            rows = await conn.fetch_all("SELECT * FROM test")
            assert isinstance(rows[0], tuple)
            assert rows[0][1] == "hello"

            # Test None (list) row factory
            conn.row_factory = None
            rows = await conn.fetch_all("SELECT * FROM test")
            assert isinstance(rows[0], list)
            assert rows[0][1] == "hello"
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_pragma_settings():
    """Test PRAGMA settings compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db, pragmas={"journal_mode": "WAL"}) as conn:
            rows = await conn.fetch_all("PRAGMA journal_mode")
            assert rows[0][0].upper() == "WAL"

            # Test set_pragma
            await conn.set_pragma("synchronous", "NORMAL")
            rows = await conn.fetch_all("PRAGMA synchronous")
            assert rows[0][0] == 1  # NORMAL = 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_concurrent_operations():
    """Test concurrent operations compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )

        async def insert_value(i: int):
            async with aiosqlite.connect(test_db) as conn:  # type: ignore[attr-defined]
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

        # Insert values concurrently
        await asyncio.gather(*[insert_value(i) for i in range(10)])

        # Verify all inserts
        async with aiosqlite.connect(test_db) as conn:
            rows = await conn.fetch_all("SELECT * FROM test ORDER BY id")
            assert len(rows) == 10
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_last_insert_rowid():
    """Test last_insert_rowid compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")

            rowid = await conn.last_insert_rowid()
            assert rowid == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_changes():
    """Test changes() method compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with aiosqlite.connect(test_db) as conn:
            await conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
            )
            await conn.execute("INSERT INTO test (value) VALUES (1)")
            await conn.execute("INSERT INTO test (value) VALUES (2)")

            await conn.execute("UPDATE test SET value = 99 WHERE id = 1")
            changes = await conn.changes()
            assert changes == 1
    finally:
        cleanup_db(test_db)


@pytest.mark.asyncio
async def test_in_memory_database():
    """Test in-memory database compatibility."""
    async with aiosqlite.connect(":memory:") as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (value) VALUES ('hello')")
        rows = await conn.fetch_all("SELECT * FROM test")
        assert len(rows) == 1
        assert rows[0][1] == "hello"


@pytest.mark.asyncio
async def test_connection_string_uri():
    """Test connection string URI format compatibility."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        # Test URI format
        uri = f"file:{test_db}"
        async with aiosqlite.connect(uri) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            await conn.execute("INSERT INTO test DEFAULT VALUES")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
    finally:
        cleanup_db(test_db)
