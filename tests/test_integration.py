"""Integration tests for rapsqlite.

Tests real-world scenarios, common usage patterns, and framework integration examples.
"""

import asyncio
import pytest

from rapsqlite import connect


@pytest.mark.integration
@pytest.mark.asyncio
async def test_web_framework_pattern(test_db):
    """Test common web framework usage pattern (request-scoped connection)."""

    # Simulate FastAPI/aiohttp pattern
    async def handle_request():
        async with connect(test_db) as db:
            await db.execute(
                "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)"
            )
            await db.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
            user = await db.fetch_one("SELECT * FROM users WHERE name = ?", ["Alice"])
            return user[1]  # Return name

    result = await handle_request()
    assert result == "Alice"


@pytest.mark.integration
@pytest.mark.asyncio
async def test_orm_like_pattern(test_db):
    """Test ORM-like usage pattern."""
    async with connect(test_db) as db:
        await db.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT UNIQUE,
                email TEXT,
                created_at INTEGER
            )
        """)

        # Insert user
        await db.execute(
            "INSERT INTO users (username, email, created_at) VALUES (?, ?, ?)",
            ["alice", "alice@example.com", 1234567890],
        )

        # Query user
        user = await db.fetch_one("SELECT * FROM users WHERE username = ?", ["alice"])
        assert user[1] == "alice"
        assert user[2] == "alice@example.com"
        assert user[3] == 1234567890


@pytest.mark.integration
@pytest.mark.asyncio
async def test_batch_processing_pattern(test_db):
    """Test batch processing pattern."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER)")

        # Batch insert
        items = [[i] for i in range(1000)]
        await db.execute_many("INSERT INTO items (value) VALUES (?)", items)

        # Batch process in chunks
        chunk_size = 100
        total = 0
        for offset in range(0, 1000, chunk_size):
            rows = await db.fetch_all(
                "SELECT value FROM items WHERE id > ? AND id <= ? ORDER BY id",
                [offset, offset + chunk_size],
            )
            total += len(rows)

        assert total == 1000


@pytest.mark.integration
@pytest.mark.asyncio
async def test_transaction_rollback_pattern(test_db):
    """Test transaction rollback pattern for error handling."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)"
        )
        await db.execute("INSERT INTO accounts (balance) VALUES (?)", [1000])

    # Simulate transfer with rollback on error
    async def transfer_money(from_id: int, to_id: int, amount: int):
        async with connect(test_db) as db:
            try:
                async with db.transaction():
                    # Check balance
                    from_account = await db.fetch_one(
                        "SELECT balance FROM accounts WHERE id = ?", [from_id]
                    )
                    if from_account[0] < amount:
                        raise ValueError("Insufficient funds")

                    # Transfer
                    await db.execute(
                        "UPDATE accounts SET balance = balance - ? WHERE id = ?",
                        [amount, from_id],
                    )
                    await db.execute(
                        "UPDATE accounts SET balance = balance + ? WHERE id = ?",
                        [amount, to_id],
                    )
            except ValueError:
                # Transaction automatically rolls back
                pass

    # This should rollback
    await transfer_money(1, 2, 2000)  # Insufficient funds

    # Verify balance unchanged
    async with connect(test_db) as db:
        balance = await db.fetch_one("SELECT balance FROM accounts WHERE id = ?", [1])
        assert balance[0] == 1000


@pytest.mark.integration
@pytest.mark.asyncio
async def test_connection_pooling_pattern(test_db):
    """Test connection pooling pattern for high-throughput scenarios."""
    async with connect(test_db) as db:
        db.pool_size = 10
        await db.execute(
            "CREATE TABLE logs (id INTEGER PRIMARY KEY, message TEXT, timestamp INTEGER)"
        )

    # Simulate high-throughput logging
    async def log_message(message: str):
        async with connect(test_db) as db:
            db.pool_size = 10
            import time

            await db.execute(
                "INSERT INTO logs (message, timestamp) VALUES (?, ?)",
                [message, int(time.time())],
            )

    # Log many messages concurrently
    messages = [f"Log message {i}" for i in range(100)]
    await asyncio.gather(*[log_message(msg) for msg in messages])

    # Verify all logged
    async with connect(test_db) as db:
        count = await db.fetch_one("SELECT COUNT(*) FROM logs")
        assert count[0] >= 100


@pytest.mark.integration
@pytest.mark.asyncio
async def test_schema_migration_pattern(test_db):
    """Test schema migration pattern."""
    async with connect(test_db) as db:
        # Initial schema
        await db.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT
            )
        """)

        # Migration: add column
        await db.execute("ALTER TABLE users ADD COLUMN email TEXT")

        # Verify migration
        await db.execute(
            "INSERT INTO users (name, email) VALUES (?, ?)",
            ["Alice", "alice@example.com"],
        )
        user = await db.fetch_one("SELECT * FROM users")
        assert len(user) == 3  # id, name, email
        assert user[1] == "Alice"
        assert user[2] == "alice@example.com"


@pytest.mark.integration
@pytest.mark.asyncio
async def test_row_factory_integration(test_db):
    """Test row factory in real-world usage."""
    async with connect(test_db) as db:
        db.row_factory = "dict"
        await db.execute(
            "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price REAL)"
        )
        await db.execute(
            "INSERT INTO products (name, price) VALUES (?, ?)", ["Widget", 19.99]
        )

        # Fetch as dict
        product = await db.fetch_one("SELECT * FROM products")
        assert isinstance(product, dict)
        assert product["name"] == "Widget"
        assert product["price"] == 19.99


@pytest.mark.integration
@pytest.mark.asyncio
async def test_cursor_iteration_pattern(test_db):
    """Test cursor iteration pattern."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER)")

        # Insert items
        for i in range(10):
            await db.execute("INSERT INTO items (value) VALUES (?)", [i])

        # Iterate with cursor - fetch all first, then iterate
        cursor = db.cursor()
        await cursor.execute("SELECT * FROM items ORDER BY id")

        items = []
        # Fetch all rows and iterate
        rows = await cursor.fetchall()
        for row in rows:
            items.append(row[1])  # value column

        assert items == list(range(10))
