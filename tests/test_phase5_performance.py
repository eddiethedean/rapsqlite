"""Correctness coverage for the Phase 0.5 low-latency APIs."""

import asyncio

import pytest

from rapsqlite import DatabaseError, NotSupportedError, ProgrammingError, connect_memory

pytestmark = [pytest.mark.unit]


@pytest.mark.asyncio
async def test_fetch_scalar_and_blob_paths_preserve_values() -> None:
    async with connect_memory(session_affinity=True) as conn:
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER, text_value TEXT, blob_value BLOB)"
        )
        await conn.execute(
            "INSERT INTO values_table VALUES (?, ?, ?)",
            [1, "hello", b"payload"],
        )

        assert (
            await conn.fetch_scalar(
                "SELECT text_value FROM values_table WHERE id = ?", [1]
            )
            == "hello"
        )
        assert (
            await conn.fetch_blob(
                "SELECT blob_value FROM values_table WHERE id = ?", [1]
            )
            == b"payload"
        )
        assert (
            await conn.fetch_scalar(
                "SELECT text_value FROM values_table WHERE id = ?", [99]
            )
            is None
        )
        assert (
            await conn.fetch_blob(
                "SELECT blob_value FROM values_table WHERE id = ?", [99]
            )
            is None
        )

        with pytest.raises(ProgrammingError, match="exactly one result column"):
            await conn.fetch_scalar(
                "SELECT id, text_value FROM values_table WHERE id = ?", [1]
            )
        with pytest.raises(TypeError, match="BLOB or NULL"):
            await conn.fetch_blob(
                "SELECT text_value FROM values_table WHERE id = ?", [1]
            )
        with pytest.raises(ProgrammingError, match="returns rows"):
            await conn.raw_fetch_scalar("UPDATE values_table SET id = id")


@pytest.mark.asyncio
async def test_raw_scalar_path_supports_transactions_and_named_parameters() -> None:
    async with connect_memory() as conn:
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER PRIMARY KEY, value BLOB)"
        )
        async with conn.transaction():
            await conn.execute(
                "INSERT INTO values_table (value) VALUES (?)", [b"inside-tx"]
            )
            assert (
                await conn.raw_fetch_scalar(
                    "SELECT value FROM values_table WHERE id = :id",
                    {"id": 1},
                    True,
                )
                == b"inside-tx"
            )


@pytest.mark.asyncio
async def test_prepared_query_uses_normal_and_raw_modes() -> None:
    async with connect_memory() as conn:
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER PRIMARY KEY, value BLOB)"
        )
        await conn.execute("INSERT INTO values_table (value) VALUES (?)", [b"prepared"])

        normal = conn.prepare("SELECT value FROM values_table WHERE id = ?")
        raw = conn.prepare(
            "SELECT value FROM values_table WHERE id = ?", raw=True, blob=True
        )
        assert await normal.fetch_scalar([1]) == b"prepared"
        assert await raw.fetch_blob([1]) == b"prepared"
        assert normal.query == "SELECT value FROM values_table WHERE id = ?"


@pytest.mark.asyncio
async def test_query_usage_tracking_is_opt_in() -> None:
    async with connect_memory() as conn:
        await conn.execute("SELECT 1")
        assert conn.query_usage() == {}

        conn.query_usage_tracking = True
        await conn.fetch_all("SELECT  1")
        await conn.fetch_all("SELECT 1")
        assert conn.query_usage() == {"SELECT 1": 2}

        conn.clear_query_usage()
        assert conn.query_usage() == {}
        conn.query_usage_tracking = False
        await conn.fetch_all("SELECT 1")
        assert conn.query_usage() == {}


@pytest.mark.asyncio
async def test_raw_scalar_rejects_callback_connections() -> None:
    async with connect_memory() as conn:
        await conn.create_function("identity", 1, lambda value: value)
        with pytest.raises(NotSupportedError, match="callbacks"):
            await conn.raw_fetch_scalar("SELECT identity(?)", [1])


@pytest.mark.asyncio
async def test_session_affinity_can_be_disabled_after_use() -> None:
    async with connect_memory(session_affinity=True) as conn:
        await conn.execute("SELECT 1")
        assert conn.session_affinity is True
        conn.session_affinity = False
        await conn.execute("SELECT 1")
        assert conn.session_affinity is False


@pytest.mark.asyncio
async def test_raw_scalar_does_not_block_cancellation_setup() -> None:
    async with connect_memory() as conn:
        await conn.execute("CREATE TABLE numbers (value INTEGER)")
        await conn.execute_many(
            "INSERT INTO numbers VALUES (?)", [[i] for i in range(10)]
        )
        task = asyncio.ensure_future(
            conn.raw_fetch_scalar("SELECT SUM(value) FROM numbers")
        )
        assert await task == 45


@pytest.mark.asyncio
async def test_interrupt_reaches_active_raw_scalar() -> None:
    async with connect_memory(session_affinity=True) as conn:
        task = asyncio.ensure_future(
            conn.raw_fetch_scalar(
                "WITH RECURSIVE counter(value) AS ("
                "SELECT 1 UNION ALL SELECT value + 1 FROM counter "
                "WHERE value < 100000000) SELECT max(value) FROM counter"
            )
        )
        await asyncio.sleep(0.01)
        await conn.interrupt()
        with pytest.raises(DatabaseError, match="interrupted"):
            await task
