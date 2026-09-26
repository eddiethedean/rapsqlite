"""Correctness coverage for the Phase 0.5 low-latency APIs."""

from typing import Any

import asyncio
import sqlite3

import pytest

from rapsqlite import (
    Connection,
    DatabaseError,
    InterfaceError,
    NotSupportedError,
    OperationalError,
    ProgrammingError,
    connect_memory,
)

pytestmark = [pytest.mark.unit]


def _identity(value: Any) -> Any:
    return value


@pytest.mark.asyncio
async def test_scalar_preflight_prepares_through_sqlx_once_for_authorizer() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        authorizer_actions: list[int] = []

        def allow_all(
            action: int,
            arg1: str | None,
            arg2: str | None,
            arg3: str | None,
            arg4: str | None,
        ) -> int:
            del arg1, arg2, arg3, arg4
            authorizer_actions.append(action)
            return sqlite3.SQLITE_OK

        await conn.set_authorizer(allow_all)

        assert await conn.fetch_scalar("SELECT 123") == 123
        assert authorizer_actions == [sqlite3.SQLITE_SELECT]

        authorizer_actions.clear()
        assert await conn.fetch_blob("SELECT X'2A'") == b"*"
        assert authorizer_actions == [sqlite3.SQLITE_SELECT]


@pytest.mark.asyncio
@pytest.mark.parametrize("transaction_entry", ["begin", "context"])
async def test_failed_transaction_init_hook_rolls_back_writes(
    transaction_entry: str,
) -> None:
    name = f"phase5-failed-init-hook-transaction-{transaction_entry}"
    async with connect_memory(name=name, pool_size=2) as keeper:
        await keeper.execute("CREATE TABLE state (value INTEGER)")
        await keeper.execute("INSERT INTO state VALUES (1)")
        await keeper.commit()

        async def failing_init_hook(conn: Connection) -> None:
            await conn.execute("UPDATE state SET value = 2")
            raise ValueError("init hook failed after writing")

        connection = Connection(keeper.path, init_hook=failing_init_hook)
        connection.pool_size = 2
        async with connection:
            with pytest.raises(OperationalError, match="init_hook raised an exception"):
                if transaction_entry == "begin":
                    await connection.begin()
                else:
                    async with connection.transaction():
                        pass

            assert not connection.in_transaction
            assert await keeper.fetch_scalar("SELECT value FROM state") == 1


@pytest.mark.asyncio
@pytest.mark.parametrize("transaction_api", ["manual", "context"])
async def test_failed_deferred_commit_keeps_transaction_rollbackable(
    transaction_api: str,
) -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
        await conn.execute(
            "CREATE TABLE child (parent_id INTEGER REFERENCES parent(id) "
            "DEFERRABLE INITIALLY DEFERRED)"
        )
        await conn.commit()

        if transaction_api == "manual":
            await conn.begin()
            await conn.execute("INSERT INTO child VALUES (999)")
            with pytest.raises(DatabaseError):
                await conn.commit()
        else:
            with pytest.raises(DatabaseError):
                async with conn.transaction():
                    await conn.execute("INSERT INTO child VALUES (999)")

        assert conn.in_transaction
        await conn.rollback()
        assert not conn.in_transaction
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM child") == 0


@pytest.mark.asyncio
async def test_failed_rollback_keeps_transaction_connection_available() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE values_table (value INTEGER)")

        def deny_rollback(
            action: int,
            arg1: str | None,
            arg2: str | None,
            arg3: str | None,
            arg4: str | None,
        ) -> int:
            del arg2, arg3, arg4
            if action == sqlite3.SQLITE_TRANSACTION and arg1 == "ROLLBACK":
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK

        await conn.set_authorizer(deny_rollback)
        await conn.begin()
        await conn.execute("INSERT INTO values_table VALUES (1)")

        with pytest.raises(DatabaseError, match="not authorized"):
            await conn.rollback()

        assert conn.in_transaction
        await conn.set_authorizer(None)
        await conn.rollback()
        assert not conn.in_transaction
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM values_table") == 0


@pytest.mark.asyncio
async def test_authorizer_routing_is_published_before_install_finishes() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        assert await conn.fetch_scalar("SELECT 1") == 1
        authorizer_calls: list[int] = []

        def deny_all(
            action: int,
            arg1: str | None,
            arg2: str | None,
            arg3: str | None,
            arg4: str | None,
        ) -> int:
            del arg1, arg2, arg3, arg4
            authorizer_calls.append(action)
            return sqlite3.SQLITE_DENY

        # The Python method publishes callback configuration synchronously but
        # its installation future has not been awaited yet. Routing must already
        # avoid the ordinary session handle, which has no SQLite authorizer.
        install = conn.set_authorizer(deny_all)
        with pytest.raises(DatabaseError, match="not authorized"):
            await conn.fetch_scalar("SELECT 42")
        await install
        assert authorizer_calls


@pytest.mark.asyncio
async def test_callback_setup_reuses_retained_session_in_single_slot_pool() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        assert await conn.fetch_scalar("SELECT 1") == 1
        await conn.execute("CREATE TABLE retained_data (value INTEGER)")
        await conn.execute("INSERT INTO retained_data VALUES (7)")

        await asyncio.wait_for(
            conn.create_function("identity", 1, _identity), timeout=2
        )
        assert await conn.fetch_scalar("SELECT identity(42)") == 42
        assert await conn.fetch_scalar("SELECT value FROM retained_data") == 7
        assert conn.session_affinity is True

        await conn.create_function("identity", 1, None)
        assert await conn.fetch_scalar("SELECT value FROM retained_data") == 7


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
        with pytest.raises(TypeError, match="BLOB or NULL"):
            await conn.raw_fetch_scalar(
                "SELECT text_value FROM values_table WHERE id = ?", [1], blob=True
            )
        with pytest.raises(ProgrammingError, match="returns rows"):
            await conn.raw_fetch_scalar("UPDATE values_table SET id = id")

        assert await conn.raw_fetch_scalar("PRAGMA user_version") == 0
        assert await conn.raw_fetch_scalar("VALUES (42)") == 42


@pytest.mark.asyncio
async def test_fetch_scalar_rejects_non_scalar_dml_before_execution() -> None:
    async with connect_memory() as conn:
        await conn.execute("CREATE TABLE scalar_side_effect (value INTEGER)")

        with pytest.raises(ProgrammingError, match="exactly one result column"):
            await conn.fetch_scalar("INSERT INTO scalar_side_effect VALUES (1)")
        with pytest.raises(ProgrammingError, match="returns rows"):
            await conn.raw_fetch_scalar("INSERT INTO scalar_side_effect VALUES (1)")
        with pytest.raises(ProgrammingError, match="exactly one result column"):
            await conn.raw_fetch_scalar("SELECT 1, 2")

        assert await conn.fetch_scalar("SELECT COUNT(*) FROM scalar_side_effect") == 0
        assert (
            await conn.fetch_scalar(
                "INSERT INTO scalar_side_effect VALUES (2) RETURNING value"
            )
            == 2
        )
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM scalar_side_effect") == 1


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
async def test_callback_handoff_waits_for_in_flight_cached_raw_statement() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        query = (
            "WITH RECURSIVE counter(value) AS ("
            "VALUES (1) UNION ALL SELECT value + 1 FROM counter "
            "WHERE value < 500000) SELECT MAX(value) FROM counter"
        )
        assert await conn.raw_fetch_scalar(query) == 500000

        raw_task = asyncio.ensure_future(conn.raw_fetch_scalar(query))
        await asyncio.sleep(0.001)
        await conn.create_function("identity", 1, _identity)

        assert await raw_task == 500000
        assert await conn.fetch_scalar("SELECT identity(7)") == 7
        await conn.create_function("identity", 1, None)


@pytest.mark.asyncio
@pytest.mark.parametrize("transaction_mode", ["explicit", "implicit"])
async def test_raw_statement_cache_is_cleared_before_transaction_handoff(
    transaction_mode: str, capfd: pytest.CaptureFixture[str]
) -> None:
    name = f"phase5-raw-cache-transaction-handoff-{transaction_mode}"
    async with connect_memory(
        name=name, pool_size=2, idle_timeout=1, session_affinity=True
    ) as keeper:
        await keeper.execute("CREATE TABLE values_table (value INTEGER)")
        await keeper.execute("INSERT INTO values_table VALUES (1)")
        await keeper.commit()

        # Keep the named in-memory database alive while the other pooled
        # connection is returned and closed by SQLx's idle reaper.
        async with connect_memory(
            name=name, pool_size=2, idle_timeout=1, session_affinity=True
        ) as conn:
            query = "SELECT value FROM values_table"
            assert await conn.raw_fetch_scalar(query) == 1

            if transaction_mode == "explicit":
                await conn.execute("BEGIN")
                await conn.execute("UPDATE values_table SET value = 2")
                await conn.execute("COMMIT")
            else:
                await conn.execute("UPDATE values_table SET value = 2")
                await conn.commit()

            # SQLx's idle reaper must be able to close the returned connection;
            # a cached raw sqlite3_stmt would otherwise make SQLite reject close.
            await asyncio.sleep(1.5)
            assert await conn.raw_fetch_scalar(query) == 2

    assert "unable to close due to unfinalized statements" not in capfd.readouterr().err


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
async def test_query_usage_tracking_includes_raw_scalar_queries() -> None:
    async with connect_memory() as conn:
        conn.query_usage_tracking = True

        assert await conn.raw_fetch_scalar("SELECT  2") == 2
        assert conn.query_usage() == {"SELECT 2": 1}


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
async def test_execute_many_with_callbacks_is_atomic() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE values_table (value INTEGER UNIQUE)")
        await conn.execute("INSERT INTO values_table VALUES (1)")
        await conn.commit()
        await conn.create_function("identity", 1, _identity)

        with pytest.raises(DatabaseError):
            await conn.execute_many("INSERT INTO values_table VALUES (?)", [[2], [1]])

        assert await conn.fetch_all(
            "SELECT value FROM values_table ORDER BY value"
        ) == [[1]]
        await conn.execute_many("INSERT INTO values_table VALUES (?)", [[2], [3]])
        assert await conn.fetch_all(
            "SELECT value FROM values_table ORDER BY value"
        ) == [
            [1],
            [2],
            [3],
        ]


@pytest.mark.asyncio
async def test_failed_first_implicit_dml_remains_rollbackable() -> None:
    async def initialize(conn: Connection) -> None:
        await conn.execute("CREATE TABLE values_table (value INTEGER UNIQUE)")
        await conn.execute("INSERT INTO values_table VALUES (1)")
        await conn.commit()

    conn = Connection(":memory:", init_hook=initialize)
    conn.pool_size = 1
    conn.session_affinity = True
    async with conn:
        with pytest.raises(DatabaseError):
            await conn.execute("INSERT INTO values_table VALUES (1)")

        await conn.rollback()
        await conn.execute("INSERT INTO values_table VALUES (2)")
        await conn.commit()
        assert await conn.fetch_all(
            "SELECT value FROM values_table ORDER BY value"
        ) == [
            [1],
            [2],
        ]


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
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        assert await conn.fetch_scalar("SELECT 1") == 1
        assert (await conn.pool_metrics())["in_use"] == 1
        assert conn.session_affinity is True
        conn.session_affinity = False
        metrics = await conn.pool_metrics()
        for _ in range(100):
            if metrics["in_use"] == 0:
                break
            await asyncio.sleep(0.01)
            metrics = await conn.pool_metrics()
        assert metrics["in_use"] == 0
        assert await conn.fetch_scalar("SELECT 1") == 1
        assert (await conn.pool_metrics())["in_use"] == 0
        assert conn.session_affinity is False


@pytest.mark.asyncio
async def test_disabling_session_affinity_keeps_memory_callback_keeper() -> None:
    name = "phase5-disable-affinity-with-callback-memory-keeper"
    async with connect_memory(name=name, pool_size=2, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE retained_data (value INTEGER)")
        await conn.execute("INSERT INTO retained_data VALUES (9)")
        await conn.commit()
        await conn.create_function("identity", 1, _identity)
        assert (await conn.pool_metrics())["in_use"] == 1

        conn.session_affinity = False

        metrics = await conn.pool_metrics()
        assert metrics["in_use"] == 1
        async with connect_memory(name=name, pool_size=2) as other:
            assert (
                await asyncio.wait_for(
                    other.fetch_scalar("SELECT value FROM retained_data"), timeout=1
                )
                == 9
            )
        assert await conn.fetch_scalar("SELECT identity(value) FROM retained_data") == 9
        await conn.create_function("identity", 1, None)
        assert await conn.fetch_scalar("SELECT value FROM retained_data") == 9


@pytest.mark.asyncio
async def test_callbacks_preserve_named_memory_database_without_session_affinity() -> (
    None
):
    name = "phase5-callback-memory-default-affinity"
    async with connect_memory(name=name, pool_size=1) as conn:
        await conn.execute("CREATE TABLE retained_data (value INTEGER)")
        await conn.execute("INSERT INTO retained_data VALUES (11)")
        await conn.commit()

        await conn.create_function("identity", 1, _identity)
        assert (
            await conn.fetch_scalar("SELECT identity(value) FROM retained_data") == 11
        )

        await conn.create_function("identity", 1, None)
        assert await conn.fetch_scalar("SELECT value FROM retained_data") == 11


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "transaction_exit",
    [
        "context_commit",
        "manual_commit",
        "manual_rollback",
        "sql_commit",
        "sql_rollback",
    ],
)
async def test_callback_transaction_keeps_named_memory_database_alive(
    transaction_exit: str,
) -> None:
    name = f"phase5-callback-transaction-retention-{transaction_exit}"
    async with connect_memory(name=name, pool_size=1, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE retained_data (value INTEGER)")
        await conn.execute("INSERT INTO retained_data VALUES (1)")
        await conn.commit()
        await conn.create_function("identity", 1, _identity)

        if transaction_exit == "context_commit":
            async with conn.transaction():
                await conn.execute("INSERT INTO retained_data VALUES (2)")
        else:
            await conn.begin()
            await conn.execute("INSERT INTO retained_data VALUES (2)")
            if transaction_exit == "manual_commit":
                await conn.commit()
            elif transaction_exit == "manual_rollback":
                await conn.rollback()
            elif transaction_exit == "sql_commit":
                await conn.execute("COMMIT")
            else:
                await conn.execute("ROLLBACK")

        expected = (
            [1, 2]
            if transaction_exit in {"context_commit", "manual_commit", "sql_commit"}
            else [1]
        )
        assert await conn.fetch_all(
            "SELECT value FROM retained_data ORDER BY value"
        ) == [[value] for value in expected]
        assert await conn.fetch_scalar("SELECT identity(value) FROM retained_data") == 1


@pytest.mark.asyncio
async def test_close_sanitizes_retained_callback_handle_before_pool_reuse() -> None:
    name = "phase5-close-sanitizes-callback-handle"
    first = connect_memory(name=name, pool_size=1, session_affinity=True)
    second = connect_memory(name=name, pool_size=1, session_affinity=True)
    await first.execute("CREATE TABLE retained_data (value INTEGER)")
    await first.execute("INSERT INTO retained_data VALUES (1)")
    await first.commit()
    await first.create_function("connection_only", 0, lambda: 731)

    assert await first.fetch_scalar("SELECT connection_only()") == 731
    await first.close()

    # Reusing the sanitized physical handle must preserve the named in-memory
    # database, but must not expose the closed Connection's Python callback.
    assert await second.fetch_scalar("SELECT COUNT(*) FROM retained_data") == 1
    with pytest.raises(DatabaseError, match="no such function: connection_only"):
        await second.fetch_scalar("SELECT connection_only()")
    await second.close()


@pytest.mark.asyncio
async def test_context_exit_sanitizes_retained_callback_handle_before_pool_reuse() -> (
    None
):
    name = "phase5-context-exit-sanitizes-callback-handle"
    first = connect_memory(name=name, pool_size=1, session_affinity=True)
    second = connect_memory(name=name, pool_size=1, session_affinity=True)

    async with first:
        await first.create_function("context_only", 0, lambda: 731)
        assert await first.fetch_scalar("SELECT context_only()") == 731

    with pytest.raises(DatabaseError, match="no such function: context_only"):
        await second.fetch_scalar("SELECT context_only()")
    await first.close()
    await second.close()


@pytest.mark.asyncio
async def test_context_exit_propagates_failed_deferred_commit_and_keeps_memory_db() -> (
    None
):
    conn = connect_memory(pool_size=1, session_affinity=True)

    with pytest.raises(DatabaseError, match="FOREIGN KEY constraint failed"):
        async with conn:
            await conn.execute("PRAGMA foreign_keys = ON")
            await conn.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
            await conn.execute(
                "CREATE TABLE child (parent_id INTEGER REFERENCES parent(id) "
                "DEFERRABLE INITIALLY DEFERRED)"
            )
            await conn.commit()
            await conn.execute("INSERT INTO child VALUES (999)")

    assert not conn.in_transaction
    assert await conn.fetch_scalar("SELECT COUNT(*) FROM child") == 0
    await conn.close()


@pytest.mark.asyncio
@pytest.mark.parametrize("transaction_entry", ["begin", "context_manager"])
async def test_failed_implicit_commit_remains_rollbackable(
    transaction_entry: str,
) -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
        await conn.execute(
            "CREATE TABLE child (parent_id INTEGER REFERENCES parent(id) "
            "DEFERRABLE INITIALLY DEFERRED)"
        )
        await conn.execute("INSERT INTO child VALUES (999)")

        with pytest.raises(DatabaseError, match="FOREIGN KEY constraint failed"):
            if transaction_entry == "begin":
                await conn.begin()
            else:
                async with conn.transaction():
                    pytest.fail("the transaction body must not run after failed commit")

        await conn.rollback()
        assert await conn.fetch_scalar("SELECT COUNT(*) FROM child") == 0

        # The physical connection remains usable after recovering the failed
        # implicit transaction.
        await conn.execute("INSERT INTO parent VALUES (999)")
        await conn.commit()
        assert await conn.fetch_scalar("SELECT id FROM parent") == 999


@pytest.mark.asyncio
@pytest.mark.parametrize("pool_size", [1, 2])
async def test_set_pragma_updates_retained_callback_connection(pool_size: int) -> None:
    async with connect_memory(pool_size=pool_size, session_affinity=True) as conn:
        await conn.fetch_scalar("SELECT 1")
        await conn.create_function("identity", 1, _identity)
        assert await conn.fetch_scalar("PRAGMA foreign_keys") == 1

        await asyncio.wait_for(conn.set_pragma("foreign_keys", False), timeout=1)

        assert await conn.fetch_scalar("PRAGMA foreign_keys") == 0


@pytest.mark.asyncio
async def test_schema_introspection_uses_retained_session_connection() -> None:
    async with connect_memory(pool_size=1, session_affinity=True) as conn:
        await conn.execute("CREATE TABLE affinity_schema (value INTEGER)")

        assert await asyncio.wait_for(conn.get_tables(), timeout=1) == [
            "affinity_schema"
        ]


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
