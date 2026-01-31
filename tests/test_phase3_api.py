"""Tests for Phase 3.9 API additions: execute_fetchall, execute_insert, Cursor props, close."""

import pytest

from rapsqlite import (
    Connection,
    connect,
    execute_iter,
    paginate,
    analyze_query_plan,
    suggest_indexes,
    in_clause_query,
    rows_to_dicts,
    pool_metrics_gauges,
    timed_fetch_all,
    transaction_retry,
    transaction_with_timeout,
)

if not hasattr(Connection, "iter_chunk_size"):
    pytest.skip(
        "Phase 3 APIs (iter_chunk_size, etc.) not supported by this build",
        allow_module_level=True,
    )


@pytest.mark.asyncio
async def test_connect_iter_chunk_size(test_db):
    """connect(..., iter_chunk_size=N) stores value; default 64."""
    async with connect(test_db, iter_chunk_size=128) as db:
        assert db.iter_chunk_size == 128
    async with connect(test_db) as db:
        assert db.iter_chunk_size == 64


@pytest.mark.asyncio
async def test_connect_loop_noop(test_db):
    """connect(..., loop=...) accepted and ignored (aiosqlite compat)."""
    async with connect(test_db, loop=None) as db:
        await db.execute("SELECT 1")
    async with connect(test_db, loop=object()) as db:
        await db.execute("SELECT 1")


@pytest.mark.asyncio
async def test_execute_fetchall(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES ('a'), ('b')")
        rows = await db.execute_fetchall("SELECT * FROM t ORDER BY id")
    assert rows == [[1, "a"], [2, "b"]]


@pytest.mark.asyncio
async def test_execute_fetchall_dict_factory(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES ('a')")
        db.row_factory = "dict"
        rows = await db.execute_fetchall("SELECT * FROM t")
    assert rows == [{"id": 1, "x": "a"}]


@pytest.mark.asyncio
async def test_execute_insert(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        r1 = await db.execute_insert("INSERT INTO t (x) VALUES (?)", ["a"])
        r2 = await db.execute_insert("INSERT INTO t (x) VALUES (?)", ["b"])
    assert r1 == 1
    assert r2 == 2


@pytest.mark.asyncio
async def test_execute_insert_rejects_select(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        with pytest.raises(Exception) as exc:
            await db.execute_insert("SELECT 1")
        assert "SELECT" in str(exc.value) or "execute_insert" in str(exc.value).lower()


@pytest.mark.asyncio
async def test_cursor_arraysize(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        for i in range(5):
            await db.execute("INSERT INTO t (id) VALUES (?)", [i + 1])
        cur = await db.execute("SELECT * FROM t")
        assert cur.arraysize == 1
        cur.arraysize = 2
        assert cur.arraysize == 2
        many = await cur.fetchmany()
        assert len(many) == 2


@pytest.mark.asyncio
async def test_cursor_iter_chunk_size_alias(test_db):
    """iter_chunk_size is aiosqlite alias for arraysize."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t (id) VALUES (1)")
        cur = await db.execute("SELECT * FROM t")
        assert cur.iter_chunk_size == 1
        cur.iter_chunk_size = 3
        assert cur.iter_chunk_size == 3
        assert cur.arraysize == 3


@pytest.mark.asyncio
async def test_cursor_connection(test_db):
    async with connect(test_db) as db:
        cur = await db.execute("SELECT 1")
        assert cur.connection is db


@pytest.mark.asyncio
async def test_cursor_description(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (a INT, b TEXT)")
        await db.execute("INSERT INTO t (a, b) VALUES (1, 'x')")
        cur = await db.execute("SELECT a, b FROM t")
        assert cur.description is None
        await cur.fetchall()
        d = cur.description
        assert d is not None
        assert len(d) == 2
        assert d[0][0] == "a" and d[1][0] == "b"


@pytest.mark.asyncio
async def test_cursor_lastrowid_rowcount(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        cur = await db.execute("INSERT INTO t (x) VALUES (?)", ["a"])
        assert cur.lastrowid == 1
        assert cur.rowcount == 1
        cur2 = await db.execute("SELECT 1")
        await cur2.fetchone()
        assert cur2.lastrowid == -1
        assert cur2.rowcount == -1


@pytest.mark.asyncio
async def test_cursor_row_factory_override(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES ('a')")
        cur = await db.execute("SELECT * FROM t")
        cur.row_factory = "tuple"
        row = await cur.fetchone()
        assert row == (1, "a")


@pytest.mark.asyncio
async def test_pool_health(test_db):
    async with connect(test_db) as db:
        ok = await db.pool_health()
        assert ok is True


@pytest.mark.asyncio
async def test_pool_metrics(test_db):
    async with connect(test_db) as db:
        m = await db.pool_metrics()
        assert "size" in m
        assert "num_idle" in m
        assert "in_use" in m
        assert m["size"] >= 0
        assert m["num_idle"] >= 0
        assert m["in_use"] >= 0
        assert m["size"] == m["num_idle"] + m["in_use"]


@pytest.mark.asyncio
async def test_pool_metrics_gauges(test_db):
    """pool_metrics_gauges(conn) returns dict of gauge names for Prometheus/custom metrics."""
    async with connect(test_db) as db:
        gauges = await pool_metrics_gauges(db)
    assert "rapsqlite_pool_size" in gauges
    assert "rapsqlite_pool_num_idle" in gauges
    assert "rapsqlite_pool_in_use" in gauges
    assert (
        gauges["rapsqlite_pool_size"]
        == gauges["rapsqlite_pool_num_idle"] + gauges["rapsqlite_pool_in_use"]
    )


@pytest.mark.asyncio
async def test_execute_iter(test_db):
    """execute_iter yields rows in chunks; conn.execute_iter(...) works; respects chunk_size."""
    async with connect(test_db, iter_chunk_size=2) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES ('a'),('b'),('c'),('d'),('e')")
        collected = []
        async for chunk in execute_iter(
            db, "SELECT * FROM t ORDER BY id", chunk_size=2
        ):
            collected.extend(chunk)
        assert collected == [[1, "a"], [2, "b"], [3, "c"], [4, "d"], [5, "e"]]
        # Connection method form and default chunk_size from iter_chunk_size
        collected2 = []
        async for chunk in db.execute_iter("SELECT * FROM t ORDER BY id"):
            collected2.extend(chunk)
        assert collected2 == [[1, "a"], [2, "b"], [3, "c"], [4, "d"], [5, "e"]]


@pytest.mark.asyncio
async def test_timed_fetch_all(test_db):
    """timed_fetch_all returns (rows, duration) or rows and calls on_timing when given."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES ('a'), ('b')")
        rows, duration = await timed_fetch_all(db, "SELECT * FROM t ORDER BY id")
    assert rows == [[1, "a"], [2, "b"]]
    assert isinstance(duration, (int, float)) and duration >= 0
    called = []

    async with connect(test_db) as db:
        rows2 = await timed_fetch_all(
            db, "SELECT * FROM t", on_timing=lambda sec, sql: called.append((sec, sql))
        )
    assert rows2 == [[1, "a"], [2, "b"]]
    assert len(called) == 1 and len(called[0]) == 2


@pytest.mark.asyncio
async def test_transaction_retry(test_db):
    """transaction_retry runs a transaction with retry on transient errors."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")

        async def do_work():
            await db.execute("INSERT INTO t (x) VALUES (?)", ["a"])

        await transaction_retry(db, do_work, max_retries=2)
        rows = await db.fetch_all("SELECT * FROM t")
    assert rows == [[1, "a"]]


@pytest.mark.asyncio
async def test_idle_timeout(test_db):
    """idle_timeout can be set; pool is created with idle_timeout when set before first use."""
    async with connect(test_db, idle_timeout=60) as db:
        assert db.idle_timeout == 60
        await db.execute("SELECT 1")
    # Set on connection after creation (before first use)
    db = connect(test_db)
    db.idle_timeout = 30
    async with db:
        await db.execute("SELECT 1")
    db.idle_timeout = None
    assert db.idle_timeout is None


@pytest.mark.asyncio
async def test_explain_query_plan(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
        rows = await db.explain_query_plan("SELECT * FROM t")
        assert isinstance(rows, list)
        assert all(isinstance(r, (list, tuple)) for r in rows)


@pytest.mark.asyncio
async def test_analyze_query_plan(test_db):
    """analyze_query_plan returns structured dict with uses_index, table_scan, details."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
        analysis = await analyze_query_plan(db, "SELECT * FROM t WHERE id = ?", [1])
        assert "rows" in analysis
        assert "details" in analysis
        assert "uses_index" in analysis
        assert "table_scan" in analysis
        assert isinstance(analysis["rows"], list)
        assert isinstance(analysis["details"], list)


@pytest.mark.asyncio
async def test_suggest_indexes(test_db):
    """suggest_indexes returns list of suggestions when table_scan without index."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
        # Full table scan (no index on x) - should suggest
        suggestions = await suggest_indexes(db, "SELECT * FROM t WHERE x = ?", ["a"])
        assert isinstance(suggestions, list)
        if suggestions:
            assert all("table" in s and "suggestion" in s for s in suggestions)
            assert any("t" in str(s.get("table", "")) for s in suggestions)
        # Query using primary key - uses index, no suggestion
        suggestions_pk = await suggest_indexes(db, "SELECT * FROM t WHERE id = ?", [1])
        assert suggestions_pk == []


@pytest.mark.asyncio
async def test_in_clause_query(test_db):
    """in_clause_query expands IN (?) to IN (?,?,...) with flattened params."""
    sql, params = in_clause_query("SELECT * FROM t WHERE id IN (?)", [1, 2, 3])
    assert "IN (?,?,?)" in sql or "IN (?, ?, ?)" in sql
    assert params == [1, 2, 3]
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t (id) VALUES (1), (2), (3)")
        rows = await db.fetch_all(sql, params)
        assert len(rows) == 3
        assert {r[0] for r in rows} == {1, 2, 3}
    with pytest.raises(ValueError, match="at least one value"):
        in_clause_query("SELECT * FROM t WHERE id IN (?)", [])
    with pytest.raises(ValueError, match="IN \\(\\?\\)"):
        in_clause_query("SELECT * FROM t WHERE id = ?", [1])


def test_rows_to_dicts():
    """rows_to_dicts converts list-of-list rows to list-of-dicts using columns."""
    rows = [[1, "a"], [2, "b"]]
    cols = ["id", "name"]
    dicts = rows_to_dicts(rows, cols)
    assert dicts == [{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]
    assert rows_to_dicts([], ["a"]) == []
    assert rows_to_dicts(rows, None) == []


@pytest.mark.asyncio
async def test_paginate(test_db):
    """paginate returns one page of rows with LIMIT/OFFSET."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        for i in range(5):
            await db.execute("INSERT INTO t (id, x) VALUES (?, ?)", [i + 1, f"v{i}"])
        page0 = await paginate(db, "SELECT * FROM t ORDER BY id", page_size=2, offset=0)
        page1 = await paginate(db, "SELECT * FROM t ORDER BY id", page_size=2, offset=2)
        page2 = await paginate(db, "SELECT * FROM t ORDER BY id", page_size=2, offset=4)
        page3 = await paginate(
            db, "SELECT * FROM t ORDER BY id", page_size=2, offset=10
        )
        assert len(page0) == 2
        assert len(page1) == 2
        assert len(page2) == 1
        assert len(page3) == 0
        assert page0[0][0] == 1 and page0[1][0] == 2
        assert page1[0][0] == 3 and page1[1][0] == 4
        assert page2[0][0] == 5


@pytest.mark.asyncio
async def test_interrupt(test_db):
    """interrupt() no-ops without callbacks; with callbacks, interrupts callback connection."""
    async with connect(test_db) as db:
        await db.interrupt()
    async with connect(test_db) as db:
        await db.create_function("f", 1, lambda x: x)
        await db.interrupt()
        await db.create_function("f", 1, None)


@pytest.mark.asyncio
async def test_connection_await(test_db):
    conn = connect(test_db)
    db = await conn
    try:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t (id) VALUES (1)")
        rows = await db.fetch_all("SELECT * FROM t")
        assert rows == [[1]]
    finally:
        await db.close()


@pytest.mark.asyncio
async def test_isolation_level(test_db):
    async with connect(test_db) as db:
        assert db.isolation_level is None
        db.isolation_level = "DEFERRED"
        assert db.isolation_level == "DEFERRED"
        db.isolation_level = "IMMEDIATE"
        assert db.isolation_level == "IMMEDIATE"
        db.isolation_level = "EXCLUSIVE"
        assert db.isolation_level == "EXCLUSIVE"
        db.isolation_level = None
        assert db.isolation_level is None
        with pytest.raises(Exception, match="DEFERRED|IMMEDIATE|EXCLUSIVE"):
            db.isolation_level = "INVALID"  # type: ignore[assignment]


@pytest.mark.asyncio
async def test_cursor_close(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t (id) VALUES (1)")
        cur = await db.execute("SELECT * FROM t")
        await cur.fetchall()
        assert cur.description is not None
        await cur.close()
        assert cur.description is None
        assert cur.lastrowid == -1
        assert cur.rowcount == -1


@pytest.mark.asyncio
async def test_savepoint_inside_transaction(test_db):
    """savepoint() context manager inside transaction(); rollback to savepoint on exception."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        async with db.transaction():
            await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
            async with db.savepoint("sp1"):
                await db.execute("INSERT INTO t (id, x) VALUES (2, 'b')")
                await db.execute("INSERT INTO t (id, x) VALUES (3, 'c')")
            # sp1 released; 1,2,3 committed with outer transaction
        rows = await db.fetch_all("SELECT id, x FROM t ORDER BY id")
    assert rows == [[1, "a"], [2, "b"], [3, "c"]]


@pytest.mark.asyncio
async def test_set_slow_query_threshold(test_db):
    """set_slow_query_threshold invokes callback when queries exceed threshold."""
    slow_calls: list[tuple[float, str]] = []

    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        # Disabled (0): callback never called
        db.set_slow_query_threshold(0)
        await db.fetch_all("SELECT 1")
        assert len(slow_calls) == 0

        # Enabled with very low threshold (1ns): callback should fire for any query
        db.set_slow_query_threshold(1e-9, lambda d, s: slow_calls.append((d, s)))
        await db.fetch_all("SELECT 1")
        assert len(slow_calls) >= 1
        assert "SELECT 1" in slow_calls[-1][1]

        # Disable again
        db.set_slow_query_threshold(0)
        slow_calls.clear()
        await db.fetch_all("SELECT 1")
        assert len(slow_calls) == 0


@pytest.mark.asyncio
async def test_transaction_with_timeout(test_db):
    """transaction_with_timeout runs work in a transaction with timeout."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")

        async def do_work():
            await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")

        await transaction_with_timeout(db, do_work, timeout_secs=10)
        rows = await db.fetch_all("SELECT * FROM t")
        assert rows == [[1, "a"]]


@pytest.mark.asyncio
async def test_savepoint_rollback(test_db):
    """Rollback to savepoint on exception."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        async with db.transaction():
            await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
            try:
                async with db.savepoint("sp1"):
                    await db.execute("INSERT INTO t (id, x) VALUES (2, 'b')")
                    raise ValueError("abort sp1")
            except ValueError:
                pass
            await db.execute("INSERT INTO t (id, x) VALUES (3, 'c')")
        rows = await db.fetch_all("SELECT id, x FROM t ORDER BY id")
    assert rows == [[1, "a"], [3, "c"]]


@pytest.mark.asyncio
async def test_savepoint_no_name(test_db):
    """savepoint() with no name uses generated name."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        async with db.transaction():
            async with db.savepoint():
                await db.execute("INSERT INTO t (id) VALUES (1)")
        rows = await db.fetch_all("SELECT * FROM t")
    assert rows == [[1]]


@pytest.mark.asyncio
async def test_savepoint_requires_transaction(test_db):
    """savepoint() without active transaction raises."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        with pytest.raises(Exception, match="active transaction|Savepoint"):
            async with db.savepoint("sp1"):
                await db.execute("INSERT INTO t (id) VALUES (1)")


@pytest.mark.asyncio
async def test_tuple_parameter_supported(test_db):
    """Tuple as parameter is converted to text for aiosqlite compatibility."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES (?)", [("a", 1)])
        row = await db.fetch_one("SELECT x FROM t")
        assert row is not None
        assert row[0] == "('a', 1)"
