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

        started = asyncio.Event()
        release = asyncio.Event()

        async def holder_transaction() -> None:
            # Hold an active transaction open so concurrent begin() calls are forced to fail.
            await db.begin()
            started.set()
            await release.wait()
            await db.execute("INSERT INTO t DEFAULT VALUES")
            await db.commit()

        async def attempt_begin_while_active() -> bool:
            await started.wait()
            try:
                await db.begin()
                # If this succeeds, we must clean up to avoid leaking an open tx.
                await db.rollback()
                return True
            except rapsqlite.Error as e:
                # Depending on timing/implementation, we may see:
                # - rapsqlite.OperationalError: "already in progress"
                # - Database error: "cannot start a transaction within a transaction"
                msg = str(e).lower()
                if "already in progress" in msg or "cannot start a transaction within a transaction" in msg:
                    return False
                raise

        holder = asyncio.create_task(holder_transaction())
        attempts = await asyncio.gather(
            *[attempt_begin_while_active() for _ in range(10)],
            return_exceptions=True,
        )
        release.set()
        await holder

        unexpected = [x for x in attempts if isinstance(x, Exception)]
        assert not unexpected, f"Unexpected exceptions from begin attempts: {unexpected!r}"

        # All concurrent attempts should be rejected while the holder tx is active.
        assert all(x is False for x in attempts), f"Expected all attempts to fail, got: {attempts!r}"

        # Verify the holder transaction committed exactly one insert.
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_concurrent_transaction_context_managers(test_db):
    """Test that concurrent transaction context managers are properly serialized."""
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

        started = asyncio.Event()
        release = asyncio.Event()

        async def holder_transaction_cm() -> None:
            async with db.transaction():
                started.set()
                await release.wait()
                await db.execute("INSERT INTO t DEFAULT VALUES")

        async def attempt_transaction_while_active() -> bool:
            await started.wait()
            try:
                async with db.transaction():
                    # If this succeeds, insert is not expected; ensure we exit cleanly.
                    return True
            except rapsqlite.Error as e:
                msg = str(e).lower()
                if (
                    "already in progress" in msg
                    or "cannot start a transaction within a transaction" in msg
                ):
                    return False
                raise

        holder = asyncio.create_task(holder_transaction_cm())
        attempts = await asyncio.gather(
            *[attempt_transaction_while_active() for _ in range(10)],
            return_exceptions=True,
        )
        release.set()
        await holder

        unexpected = [x for x in attempts if isinstance(x, Exception)]
        assert not unexpected, f"Unexpected exceptions from transaction attempts: {unexpected!r}"
        assert all(x is False for x in attempts), f"Expected all attempts to fail, got: {attempts!r}"

        # Verify the holder transaction committed exactly one insert.
        rows = await db.fetch_all("SELECT COUNT(*) FROM t")
        assert rows[0][0] == 1


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
