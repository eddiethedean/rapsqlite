"""Tests for Phase 3.9 API additions: execute_fetchall, execute_insert, Cursor props, close."""

import pytest

from rapsqlite import connect


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
async def test_explain_query_plan(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (id, x) VALUES (1, 'a')")
        rows = await db.explain_query_plan("SELECT * FROM t")
        assert isinstance(rows, list)
        assert all(isinstance(r, (list, tuple)) for r in rows)


@pytest.mark.asyncio
async def test_interrupt_stub(test_db):
    async with connect(test_db) as db:
        with pytest.raises(NotImplementedError, match="interrupt"):
            await db.interrupt()


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
