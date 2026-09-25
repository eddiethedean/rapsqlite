"""Correctness coverage for the Phase 0.5 low-latency APIs."""

from typing import Any

import asyncio

import pytest

from rapsqlite import (
    Connection,
    DatabaseError,
    InterfaceError,
    NotSupportedError,
    ProgrammingError,
    connect_memory,
)

pytestmark = [pytest.mark.unit]


def _identity(value: Any) -> Any:
    return value


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
async def test_fetch_blob_checks_sqlite_storage_class_before_text_factory() -> None:
    async with connect_memory() as conn:
        await conn.execute(
            "CREATE TABLE values_table (text_value TEXT, blob_value BLOB)"
        )
        await conn.execute("INSERT INTO values_table VALUES (?, ?)", ["text", b"blob"])
        conn.text_factory = bytes

        assert await conn.fetch_blob("SELECT blob_value FROM values_table") == b"blob"
        with pytest.raises(TypeError, match="BLOB or NULL"):
            await conn.fetch_blob("SELECT text_value FROM values_table")

        prepared = conn.prepare("SELECT text_value FROM values_table")
        with pytest.raises(TypeError, match="BLOB or NULL"):
            await prepared.fetch_blob()


@pytest.mark.asyncio
async def test_trace_callback_covers_scalar_paths() -> None:
    async with connect_memory() as conn:
        traced: list[str] = []
        await conn.set_trace_callback(traced.append)

        assert await conn.fetch_scalar("SELECT 1") == 1
        assert await conn.raw_fetch_scalar("SELECT 2") == 2

        assert traced == ["SELECT 1", "SELECT 2"]


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
async def test_raw_scalar_rejects_multiple_statements_without_executing_them() -> None:
    async with connect_memory() as conn:
        await conn.execute("CREATE TABLE values_table (value INTEGER)")

        with pytest.raises(DatabaseError, match="only one SQL statement"):
            await conn.raw_fetch_scalar("SELECT 1; INSERT INTO values_table VALUES (1)")

        assert await conn.raw_fetch_scalar("SELECT 2; -- trailing comment") == 2
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM values_table") == 0


@pytest.mark.asyncio
async def test_raw_statement_reuse_handles_bindings_schema_changes_and_reopen() -> None:
    database_name = "phase5-raw-statement-cache-lifecycle"
    query = "SELECT value FROM values_table WHERE id = ?"
    async with connect_memory(name=database_name, session_affinity=True) as conn:
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER PRIMARY KEY, value TEXT)"
        )
        await conn.execute_many(
            "INSERT INTO values_table VALUES (?, ?)", [[1, "one"], [2, "two"]]
        )
        assert await conn.raw_fetch_scalar(query, [1]) == "one"
        assert await conn.raw_fetch_scalar(query, [2]) == "two"

        await conn.execute("ALTER TABLE values_table ADD COLUMN extra TEXT")
        assert await conn.raw_fetch_scalar(query, [1]) == "one"

        await conn.execute("DROP TABLE values_table")
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER PRIMARY KEY, value TEXT)"
        )
        await conn.execute("INSERT INTO values_table VALUES (1, 'replacement')")
        assert await conn.raw_fetch_scalar(query, [1]) == "replacement"

    async with connect_memory(name=database_name, session_affinity=True) as reopened:
        await reopened.execute(
            "CREATE TABLE IF NOT EXISTS values_table "
            "(id INTEGER PRIMARY KEY, value TEXT)"
        )
        await reopened.execute(
            "INSERT OR REPLACE INTO values_table VALUES (1, 'reopened')"
        )
        prepared = reopened.prepare(query, raw=True)
        assert await prepared.fetch_scalar([1]) == "reopened"

        reopened.session_affinity = False
        assert await prepared.fetch_scalar([1]) == "reopened"


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
async def test_prepared_query_uses_pool_and_transaction_routes() -> None:
    async with connect_memory(pool_size=4) as conn:
        await conn.execute(
            "CREATE TABLE values_table (id INTEGER PRIMARY KEY, value TEXT)"
        )
        await conn.execute_many(
            "INSERT INTO values_table VALUES (?, ?)",
            [[index, f"value-{index}"] for index in range(100)],
        )
        lookup = conn.prepare("SELECT value FROM values_table WHERE id = ?")
        values = await asyncio.gather(
            *(lookup.fetch_scalar([index]) for index in range(100))
        )
        assert values == [f"value-{index}" for index in range(100)]

        update = conn.prepare("UPDATE values_table SET value = ? WHERE id = ?")
        async with conn.transaction():
            await update.execute(["uncommitted", 1])
            assert await lookup.fetch_scalar([1]) == "uncommitted"
        assert await lookup.fetch_scalar([1]) == "uncommitted"


@pytest.mark.asyncio
async def test_prepared_query_fails_after_close_and_new_connection_can_reprepare() -> (
    None
):
    conn = connect_memory(name="phase5-prepared-reopen")
    await conn.__aenter__()
    await conn.execute("CREATE TABLE values_table (value INTEGER)")
    await conn.execute("INSERT INTO values_table VALUES (7)")
    old_query = conn.prepare("SELECT value FROM values_table")
    assert await old_query.fetch_scalar() == 7
    await conn.close()

    with pytest.raises(InterfaceError):
        await old_query.fetch_scalar()

    async with connect_memory(name="phase5-prepared-reopen") as reopened:
        await reopened.execute("CREATE TABLE values_table (value INTEGER)")
        await reopened.execute("INSERT INTO values_table VALUES (9)")
        new_query = reopened.prepare("SELECT value FROM values_table")
        assert await new_query.fetch_scalar() == 9


@pytest.mark.asyncio
async def test_query_usage_tracking_is_opt_in() -> None:
    async with connect_memory() as conn:
        await conn.execute("SELECT 1")
        assert conn.query_usage() == {}
        assert conn.query_usage_dropped() == 0

        conn.query_usage_tracking = True
        await conn.fetch_all("SELECT  1")
        await conn.fetch_all("SELECT 1")
        assert conn.query_usage() == {"SELECT 1": 2}

        conn.clear_query_usage()
        assert conn.query_usage() == {}
        assert conn.query_usage_dropped() == 0
        conn.query_usage_tracking = False
        await conn.fetch_all("SELECT 1")
        assert conn.query_usage() == {}


@pytest.mark.asyncio
async def test_query_usage_tracking_counts_each_execute_many_statement() -> None:
    async with connect_memory() as conn:
        await conn.execute("CREATE TABLE usage_batch (value INTEGER)")
        conn.query_usage_tracking = True

        await conn.execute_many("INSERT INTO usage_batch VALUES (?)", [[1], [2], [3]])

        assert conn.query_usage() == {"INSERT INTO usage_batch VALUES (?)": 3}
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM usage_batch") == 3
        assert conn.query_usage()["SELECT COUNT(*) FROM usage_batch"] == 1


@pytest.mark.asyncio
async def test_query_usage_tracking_is_bounded_under_concurrency() -> None:
    distinct_queries = 1_100
    async with connect_memory(pool_size=4) as conn:
        conn.query_usage_tracking = True
        values = await asyncio.gather(
            *(conn.fetch_scalar(f"SELECT {index}") for index in range(distinct_queries))
        )

        assert values == list(range(distinct_queries))
        snapshot = conn.query_usage()
        assert len(snapshot) == 1_024
        assert sum(snapshot.values()) == 1_024
        assert conn.query_usage_dropped() == distinct_queries - 1_024

        # Existing keys keep counting even after the distinct-query cap is hit.
        await conn.fetch_scalar("SELECT 0")
        assert conn.query_usage()["SELECT 0"] == 2
        assert conn.query_usage_dropped() == distinct_queries - 1_024

        # Oversized SQL is executed normally but its text is never retained.
        oversized = f"/*{'x' * 2_100}*/ SELECT 1"
        assert await conn.fetch_scalar(oversized) == 1
        assert conn.query_usage_dropped() == distinct_queries - 1_024 + 1

        conn.clear_query_usage()
        assert conn.query_usage() == {}
        assert conn.query_usage_dropped() == 0


@pytest.mark.asyncio
async def test_query_feature_fast_paths_survive_enable_disable_transitions() -> None:
    async with connect_memory() as conn:
        assert await conn.fetch_scalar("SELECT 1") == 1

        traced: list[str] = []
        await conn.set_trace_callback(traced.append)
        assert await conn.fetch_scalar("SELECT 2") == 2
        await conn.set_trace_callback(None)
        assert await conn.fetch_scalar("SELECT 3") == 3
        assert traced == ["SELECT 2"]

        await conn.create_function("phase5_identity", 1, _identity)
        assert await conn.fetch_scalar("SELECT phase5_identity(?)", [4]) == 4
        await conn.create_function("phase5_identity", 1, None)
        assert await conn.fetch_scalar("SELECT 5") == 5


@pytest.mark.asyncio
async def test_transaction_state_fast_path_survives_transaction_transitions() -> None:
    async with connect_memory(pool_size=1) as conn:
        assert not conn.in_transaction
        assert await conn.fetch_scalar("SELECT 1") == 1

        await conn.begin()
        assert conn.in_transaction
        assert await conn.fetch_scalar("SELECT 2") == 2

        await conn.rollback()
        assert not conn.in_transaction
        assert await conn.fetch_scalar("SELECT 3") == 3


@pytest.mark.asyncio
async def test_raw_scalar_rejects_callback_connections() -> None:
    async with connect_memory() as conn:
        await conn.create_function("identity", 1, _identity)
        with pytest.raises(NotSupportedError, match="callbacks"):
            await conn.raw_fetch_scalar("SELECT identity(?)", [1])


@pytest.mark.asyncio
async def test_raw_scalar_rejects_callbacks_registered_by_init_hook() -> None:
    async def init_hook(conn: Any) -> None:
        await conn.create_function("identity", 1, _identity)

    async with Connection(":memory:", init_hook=init_hook) as conn:
        with pytest.raises(NotSupportedError, match="callbacks"):
            await conn.raw_fetch_scalar("SELECT identity(?)", [1])


@pytest.mark.asyncio
async def test_execute_many_rolls_back_after_raw_path_errors() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
        await conn.execute(
            "CREATE TABLE child (parent_id INTEGER REFERENCES parent(id) "
            "DEFERRABLE INITIALLY DEFERRED)"
        )

        with pytest.raises(DatabaseError):
            await conn.execute_many("INSERT INTO child VALUES (?)", [[999]])

        # A failed deferred-FK COMMIT must not poison the retained connection.
        await conn.execute_many("INSERT INTO parent VALUES (?)", [[999]])
        assert (
            await conn.fetch_scalar("SELECT id FROM parent WHERE id = ?", [999]) == 999
        )

        with pytest.raises(DatabaseError):
            await conn.execute_many("INSERT INTO parent VALUES (?)\x00", [[1000]])

        # Query validation happens before BEGIN, so a malformed query also
        # leaves the same retained connection usable.
        await conn.execute_many("INSERT INTO parent VALUES (?)", [[1001]])


@pytest.mark.asyncio
async def test_execute_many_validates_each_binding_set_and_statement_count() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE values_table (first INTEGER, second INTEGER)")

        with pytest.raises(DatabaseError, match="Incorrect number of bindings"):
            await conn.execute_many(
                "INSERT INTO values_table VALUES (?, ?)", [[1, 2], [3]]
            )
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM values_table") == 0

        with pytest.raises(DatabaseError, match="only one SQL statement"):
            await conn.execute_many(
                "INSERT INTO values_table VALUES (?, ?); DELETE FROM values_table",
                [[4, 5]],
            )
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM values_table") == 0


@pytest.mark.asyncio
async def test_execute_many_reports_actual_changes_not_input_rows() -> None:
    async with connect_memory(session_affinity=True) as conn:
        await conn.execute("CREATE TABLE values_table (value INTEGER UNIQUE)")

        await conn.execute_many(
            "INSERT OR IGNORE INTO values_table VALUES (?)", [[1], [1], [2]]
        )
        assert await conn.changes() == 2

        await conn.execute_many(
            "UPDATE values_table SET value = value WHERE value = ?", [[999]]
        )
        assert await conn.changes() == 0


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


@pytest.mark.asyncio
async def test_cancelled_raw_scalar_can_be_stopped_with_interrupt() -> None:
    async with connect_memory(session_affinity=True) as conn:
        task = asyncio.ensure_future(
            conn.raw_fetch_scalar(
                "WITH RECURSIVE counter(value) AS ("
                "SELECT 1 UNION ALL SELECT value + 1 FROM counter "
                "WHERE value < 100000000) SELECT max(value) FROM counter"
            )
        )
        await asyncio.sleep(0.01)
        task.cancel()
        await conn.interrupt()
        with pytest.raises(asyncio.CancelledError):
            await task
