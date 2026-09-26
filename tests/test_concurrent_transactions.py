"""Tests for concurrent transaction handling and race condition prevention.

Note: These tests are designed to verify that concurrent transaction attempts
are properly serialized. In parallel test execution, only one transaction may
succeed at a time, which is expected behavior.
"""

from typing import Any

import asyncio
import pytest
import rapsqlite
from rapsqlite import Error, OperationalError

# Mark tests that verify concurrent behavior (may have different results in parallel)
pytestmark = [pytest.mark.asyncio, pytest.mark.concurrency]


@pytest.mark.asyncio
async def test_fetch_waits_for_pending_begin_to_install_transaction_connection(
    unique_table_prefix: str,
) -> None:
    database_name = f"{unique_table_prefix}_pending_begin"
    holder = rapsqlite.connect_memory(name=database_name, pool_size=1)
    waiting = rapsqlite.connect_memory(name=database_name, pool_size=1)
    begin_task: asyncio.Task[None] | None = None
    fetch_task: asyncio.Future[Any] | None = None

    try:
        await holder.begin()
        begin_task = asyncio.create_task(waiting.begin())
        # The holder owns the only pooled connection, so the second begin must
        # have reserved its Starting state and be waiting for pool capacity.
        await asyncio.sleep(0.05)
        assert not begin_task.done()

        fetch_task = asyncio.ensure_future(waiting.fetch_scalar("SELECT 1"))
        await asyncio.sleep(0.02)
        fetch_waited_for_begin = not fetch_task.done()

        await holder.rollback()
        await begin_task
        fetch_result = await asyncio.gather(fetch_task, return_exceptions=True)

        assert fetch_waited_for_begin
        assert fetch_result == [1]
        await waiting.rollback()
    finally:
        if begin_task is not None and not begin_task.done():
            await holder.rollback()
            await begin_task
        if fetch_task is not None and not fetch_task.done():
            await asyncio.gather(fetch_task, return_exceptions=True)
        await holder.close()
        await waiting.close()


@pytest.mark.asyncio
async def test_raw_begin_racing_first_implicit_write_does_not_deadlock() -> None:
    async with rapsqlite.connect_memory(pool_size=1, session_affinity=True) as db:
        await db.execute("CREATE TABLE transaction_start_race (value INTEGER)")
        await db.commit()

        async def execute(query: str) -> None:
            await db.execute(query)

        for _ in range(128):
            begin_task = asyncio.create_task(execute("BEGIN"))
            # Let raw BEGIN reach its asynchronous SQLite execution before the
            # first implicit DML attempts to reserve transaction state.
            await asyncio.sleep(0)
            write_task = asyncio.create_task(
                execute("INSERT INTO transaction_start_race VALUES (1)")
            )

            begin_result, write_result = await asyncio.wait_for(
                asyncio.gather(begin_task, write_task, return_exceptions=True),
                timeout=2,
            )
            assert write_result is None
            if isinstance(begin_result, Exception):
                # If the write wins and starts the transaction, SQLite rejects
                # the nested BEGIN.
                assert isinstance(begin_result, Error)
                assert "transaction" in str(begin_result).lower()
                assert not await db.in_transaction_async()
            else:
                assert await db.in_transaction_async()

            await db.rollback()

        assert await db.fetch_scalar("SELECT COUNT(*) FROM transaction_start_race") == 0


@pytest.mark.asyncio
async def test_concurrent_begin_attempts(test_db: str, unique_table_prefix: str):
    """Test that concurrent begin() calls are properly serialized."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        started = asyncio.Event()
        release = asyncio.Event()

        async def holder_transaction() -> None:
            # Hold an active transaction open so concurrent begin() calls are forced to fail.
            await db.begin()
            started.set()
            await release.wait()
            await db.execute(f"INSERT INTO {tbl} DEFAULT VALUES")
            await db.commit()

        async def attempt_begin_while_active() -> bool:
            await started.wait()
            try:
                await db.begin()
                # If this succeeds, we must clean up to avoid leaking an open tx.
                await db.rollback()
                return True
            except (Error, OperationalError) as e:
                # Depending on timing/implementation, we may see:
                # - OperationalError: "already in progress"
                # - Database error: "cannot start a transaction within a transaction"
                # OperationalError may not subclass Error on older builds.
                msg = str(e).lower()
                if (
                    "already in progress" in msg
                    or "cannot start a transaction within a transaction" in msg
                ):
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
        assert not unexpected, (
            f"Unexpected exceptions from begin attempts: {unexpected!r}"
        )

        # All concurrent attempts should be rejected while the holder tx is active.
        assert all(x is False for x in attempts), (
            f"Expected all attempts to fail, got: {attempts!r}"
        )

        # Verify the holder transaction committed exactly one insert.
        rows = await db.fetch_all(f"SELECT COUNT(*) FROM {tbl}")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_concurrent_transaction_context_managers(
    test_db: str, unique_table_prefix: str
):
    """Test that concurrent transaction context managers are properly serialized."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        started = asyncio.Event()
        release = asyncio.Event()

        async def holder_transaction_cm() -> None:
            async with db.transaction():
                started.set()
                await release.wait()
                await db.execute(f"INSERT INTO {tbl} DEFAULT VALUES")

        async def attempt_transaction_while_active() -> bool:
            await started.wait()
            try:
                async with db.transaction():
                    # If this succeeds, insert is not expected; ensure we exit cleanly.
                    return True
            except (Error, OperationalError) as e:
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
        assert not unexpected, (
            f"Unexpected exceptions from transaction attempts: {unexpected!r}"
        )
        assert all(x is False for x in attempts), (
            f"Expected all attempts to fail, got: {attempts!r}"
        )

        # Verify the holder transaction committed exactly one insert.
        rows = await db.fetch_all(f"SELECT COUNT(*) FROM {tbl}")
        assert rows[0][0] == 1


@pytest.mark.asyncio
async def test_begin_while_transaction_active(test_db: str, unique_table_prefix: str):
    """Test that begin() fails if transaction is already active."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        await db.begin()
        await db.execute(f"INSERT INTO {tbl} DEFAULT VALUES")

        # Attempting to begin again should fail
        with pytest.raises(rapsqlite.OperationalError, match="already in progress"):
            await db.begin()

        await db.commit()


@pytest.mark.asyncio
async def test_transaction_context_while_begin_active(
    test_db: str, unique_table_prefix: str
):
    """Test that transaction context manager fails if begin() is active."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        await db.begin()

        # Attempting to use transaction context manager should fail
        with pytest.raises(rapsqlite.OperationalError, match="already in progress"):
            async with db.transaction():
                pass

        await db.rollback()


@pytest.mark.asyncio
async def test_transaction_state_consistency(test_db: str, unique_table_prefix: str):
    """Test that transaction state remains consistent under concurrent access."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        # Start a transaction
        await db.begin()

        # Verify we're in a transaction
        in_tx = db.in_transaction  # sync property
        assert in_tx is True

        # Try concurrent operations - they should use the transaction connection
        async def insert_value(val: Any):
            await db.execute(f"INSERT INTO {tbl} (id) VALUES (?)", [val])

        # These should all use the same transaction connection
        await asyncio.gather(*[insert_value(i) for i in range(5)])

        # Verify all inserts are in the transaction
        in_tx = db.in_transaction  # sync property
        assert in_tx is True

        await db.commit()

        # Verify all inserts were committed
        rows = await db.fetch_all(f"SELECT COUNT(*) FROM {tbl}")
        assert rows[0][0] == 5


@pytest.mark.asyncio
async def test_transaction_rollback_on_error_preserves_state(
    test_db: str, unique_table_prefix: str
):
    """Test that transaction state is properly reset after rollback."""
    tbl = unique_table_prefix
    async with rapsqlite.connect(test_db) as db:
        await db.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY)")

        # Start and rollback a transaction
        await db.begin()
        await db.execute(f"INSERT INTO {tbl} DEFAULT VALUES")
        await db.rollback()

        # State should be reset - we should be able to start a new transaction
        await db.begin()
        await db.execute(f"INSERT INTO {tbl} DEFAULT VALUES")
        await db.commit()

        # Verify only the second insert is present
        rows = await db.fetch_all(f"SELECT COUNT(*) FROM {tbl}")
        assert rows[0][0] == 1
