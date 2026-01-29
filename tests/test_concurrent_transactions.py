"""Tests for concurrent transaction handling and race condition prevention."""

import pytest
import rapsqlite
import asyncio


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
        results = await asyncio.gather(*[attempt_begin(i) for i in range(10)], return_exceptions=True)

        # Verify that transactions were serialized (only one active at a time)
        # All should succeed because they complete sequentially, but they should
        # be properly serialized (not overlapping)
        successes = sum(1 for r in results if r is True)
        failures = sum(1 for r in results if isinstance(r, Exception) or r is False)

        # All should succeed because they're serialized (one completes before next starts)
        # The important thing is that they don't overlap (no "already in progress" errors)
        assert successes == 10, "All transactions should succeed when serialized"
        assert failures == 0, "No transactions should fail with 'already in progress'"

        # Verify all transactions committed
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 10


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
        results = await asyncio.gather(*[attempt_transaction(i) for i in range(10)], return_exceptions=True)

        # Transactions should be serialized - all should succeed
        # (they complete one at a time, so no "already in progress" errors)
        successes = sum(1 for r in results if r is True)
        failures = sum(1 for r in results if isinstance(r, Exception) or r is False)

        # All should succeed because they're properly serialized
        assert successes == 10, "All transactions should succeed when serialized"
        assert failures == 0, "No transactions should fail"

        # Verify all transactions committed
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 10


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
