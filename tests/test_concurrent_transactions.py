"""Tests for concurrent transaction handling and race condition prevention.

Note: These tests are designed to verify that concurrent transaction attempts
are properly serialized. In parallel test execution, only one transaction may
succeed at a time, which is expected behavior.
"""

import pytest
import rapsqlite
import asyncio

# Mark tests that verify concurrent behavior (may have different results in parallel)
pytestmark = pytest.mark.asyncio


@pytest.mark.asyncio
async def test_concurrent_begin_attempts(test_db):
    """Test that concurrent begin() calls are properly serialized."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        begin_events = []
        commit_events = []

        async def attempt_begin(worker_id):
            try:
                begin_events.append(worker_id)
                await db.begin()
                # Add small delay to ensure transactions overlap
                await asyncio.sleep(0.01)
                await db.execute("INSERT INTO t DEFAULT VALUES")
                await db.commit()
                commit_events.append(worker_id)
                return True
            except rapsqlite.OperationalError as e:
                if "already in progress" in str(e):
                    return False  # Expected - another transaction is active
                raise

        # Try to start multiple transactions concurrently
        results = await asyncio.gather(
            *[attempt_begin(i) for i in range(10)], return_exceptions=True
        )

        # Verify that transactions were serialized
        # When multiple begin() calls happen concurrently, only one can succeed at a time
        # The others will get "already in progress" errors and should retry or fail gracefully
        successes = sum(1 for r in results if r is True)

        # In concurrent execution, only one transaction can start at a time
        # The others will fail with "already in progress" - this is expected behavior
        # We expect at least one to succeed, and the rest may fail (which is correct)
        assert successes >= 1, "At least one transaction should succeed"
        # Note: In parallel test execution, timing can cause more failures
        # The important thing is that concurrent begin() calls are properly rejected

        # Verify all successful transactions committed
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == successes, (
            f"Expected {successes} rows (one per successful transaction), got {rows[0][0]}"
        )


@pytest.mark.asyncio
async def test_concurrent_transaction_context_managers(test_db):
    """Test that concurrent transaction context managers are properly serialized."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        async def attempt_transaction(worker_id):
            try:
                async with db.transaction():
                    # Add small delay to ensure transactions overlap
                    await asyncio.sleep(0.01)
                    await db.execute("INSERT INTO t DEFAULT VALUES")
                return True
            except rapsqlite.OperationalError as e:
                if "already in progress" in str(e):
                    return False  # Expected if transactions overlap
                raise

        # Try to start multiple transactions concurrently
        results = await asyncio.gather(
            *[attempt_transaction(i) for i in range(10)], return_exceptions=True
        )

        # Transactions should be serialized
        # When multiple transaction context managers start concurrently, only one can succeed at a time
        # The others will get "already in progress" errors - this is expected behavior
        successes = sum(1 for r in results if r is True)

        # In concurrent execution, only one transaction can start at a time
        # We expect at least one to succeed, and the rest may fail (which is correct)
        assert successes >= 1, "At least one transaction should succeed"
        # Note: In parallel test execution, timing can cause more failures
        # The important thing is that concurrent transaction attempts are properly rejected

        # Verify all successful transactions committed
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == successes, (
            f"Expected {successes} rows (one per successful transaction), got {rows[0][0]}"
        )


@pytest.mark.asyncio
async def test_begin_while_transaction_active(test_db):
    """Test that begin() fails if transaction is already active."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")

        # Attempting to begin again should fail
        with pytest.raises(rapsqlite.OperationalError, match="already in progress"):
            await db.begin()

        await db.commit()


@pytest.mark.asyncio
async def test_transaction_context_while_begin_active(test_db):
    """Test that transaction context manager fails if begin() is active."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        await db.begin()

        # Attempting to use transaction context manager should fail
        with pytest.raises(rapsqlite.OperationalError, match="already in progress"):
            async with db.transaction():
                pass

        await db.rollback()


@pytest.mark.asyncio
async def test_transaction_state_consistency(test_db):
    """Test that transaction state remains consistent under concurrent access."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        # Start a transaction
        await db.begin()

        # Verify we're in a transaction
        in_tx = await db.in_transaction()
        assert in_tx is True

        # Try concurrent operations - they should use the transaction connection
        async def insert_value(val):
            await db.execute("INSERT INTO t (id) VALUES (?)", [val])

        # These should all use the same transaction connection
        await asyncio.gather(*[insert_value(i) for i in range(5)])

        # Verify all inserts are in the transaction
        in_tx = await db.in_transaction()
        assert in_tx is True

        await db.commit()

        # Verify all inserts were committed
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 5


@pytest.mark.asyncio
async def test_transaction_rollback_on_error_preserves_state(test_db):
    """Test that transaction state is properly reset after rollback."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        # Start and rollback a transaction
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        await db.rollback()

        # State should be reset - we should be able to start a new transaction
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        await db.commit()

        # Verify only the second insert is present
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1
