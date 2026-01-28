"""Debug test for execute_many transaction issue."""

import asyncio
import tempfile
import os
from rapsqlite import connect


async def test_execute_in_loop():
    """Test if execute() works when called multiple times in a transaction."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            # Test execute() in a loop within transaction
            async with db.transaction():
                for value in ["value1", "value2", "value3"]:
                    await db.execute("INSERT INTO test (value) VALUES (?)", [value])

            # Verify all committed
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ execute() in loop works")
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_execute_many_minimal():
    """Minimal test for execute_many in transaction."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            # Test execute_many within transaction
            async with db.transaction():
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["value1"], ["value2"], ["value3"]],
                )

            # Verify all committed
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ execute_many works")
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_execute_then_execute_many():
    """Test execute() then execute_many() in same transaction."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

            async with db.transaction():
                # First execute should work
                await db.execute("INSERT INTO test (value) VALUES (?)", ["value1"])
                # Then execute_many
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)", [["value2"], ["value3"]]
                )

            # Verify all committed
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ execute then execute_many works")
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


if __name__ == "__main__":
    print("Testing execute() in loop...")
    asyncio.run(test_execute_in_loop())

    print("\nTesting execute_many minimal...")
    try:
        asyncio.run(test_execute_many_minimal())
    except Exception as e:
        print(f"✗ execute_many failed: {e}")

    print("\nTesting execute then execute_many...")
    try:
        asyncio.run(test_execute_then_execute_many())
    except Exception as e:
        print(f"✗ execute then execute_many failed: {e}")
