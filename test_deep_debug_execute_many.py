"""Deep diagnostic tests for execute_many transaction lock issue."""

import asyncio
import tempfile
import os
from rapsqlite import connect


async def test_connection_identity():
    """Test that connection pointer remains stable across execute_many iterations."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            
            async with db.transaction():
                # This should show same connection pointer in logs
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["value1"], ["value2"], ["value3"]]
                )
            
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ Connection identity test completed - check logs for connection pointers")
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_single_param_set():
    """Test execute_many with single parameter set - if this works, issue is loop-specific."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            
            async with db.transaction():
                # Single parameter set
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["value1"]]
                )
            
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 1
            print("✓ Single param set works")
    except Exception as e:
        print(f"✗ Single param set failed: {e}")
        raise
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_two_param_sets():
    """Test execute_many with two parameter sets."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            
            async with db.transaction():
                # Two parameter sets
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["value1"], ["value2"]]
                )
            
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 2
            print("✓ Two param sets works")
    except Exception as e:
        print(f"✗ Two param sets failed: {e}")
        raise
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_execute_then_execute_many():
    """Test execute() followed by execute_many() in same transaction."""
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
                    "INSERT INTO test (value) VALUES (?)",
                    [["value2"], ["value3"]]
                )
            
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ Execute then execute_many works")
    except Exception as e:
        print(f"✗ Execute then execute_many failed: {e}")
        raise
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


async def test_execute_many_then_execute():
    """Test execute_many() followed by execute() in same transaction."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name

    try:
        async with connect(test_db) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            
            async with db.transaction():
                # First execute_many
                await db.execute_many(
                    "INSERT INTO test (value) VALUES (?)",
                    [["value1"], ["value2"]]
                )
                # Then execute
                await db.execute("INSERT INTO test (value) VALUES (?)", ["value3"])
            
            rows = await db.fetch_all("SELECT * FROM test")
            assert len(rows) == 3
            print("✓ Execute_many then execute works")
    except Exception as e:
        print(f"✗ Execute_many then execute failed: {e}")
        raise
    finally:
        if os.path.exists(test_db):
            os.unlink(test_db)


if __name__ == "__main__":
    print("=" * 60)
    print("Deep Debug Test Suite for execute_many Transaction Lock Issue")
    print("=" * 60)
    
    print("\n1. Testing connection identity...")
    try:
        asyncio.run(test_connection_identity())
    except Exception as e:
        print(f"   FAILED: {e}")
    
    print("\n2. Testing single parameter set...")
    try:
        asyncio.run(test_single_param_set())
    except Exception as e:
        print(f"   FAILED: {e}")
    
    print("\n3. Testing two parameter sets...")
    try:
        asyncio.run(test_two_param_sets())
    except Exception as e:
        print(f"   FAILED: {e}")
    
    print("\n4. Testing execute() then execute_many()...")
    try:
        asyncio.run(test_execute_then_execute_many())
    except Exception as e:
        print(f"   FAILED: {e}")
    
    print("\n5. Testing execute_many() then execute()...")
    try:
        asyncio.run(test_execute_many_then_execute())
    except Exception as e:
        print(f"   FAILED: {e}")
    
    print("\n" + "=" * 60)
    print("Test suite completed. Check stderr for DEBUG logs.")
    print("=" * 60)
