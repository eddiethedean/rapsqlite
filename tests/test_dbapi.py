"""Tests for rapsqlite.dbapi (True Async DBAPI spec)."""

from __future__ import annotations

import asyncio

import pytest

pytest.importorskip("rapsqlite")
from rapsqlite import dbapi


@pytest.mark.asyncio
async def test_module_level_contract():
    assert dbapi.apilevel == "2.0"
    assert dbapi.threadsafety == 0
    assert dbapi.paramstyle == "qmark"


@pytest.mark.asyncio
async def test_exception_hierarchy():
    assert issubclass(dbapi.InterfaceError, dbapi.Error)
    assert issubclass(dbapi.DataError, dbapi.DatabaseError)
    assert issubclass(dbapi.OperationalError, dbapi.DatabaseError)
    assert issubclass(dbapi.IntegrityError, dbapi.DatabaseError)
    assert issubclass(dbapi.InternalError, dbapi.DatabaseError)
    assert issubclass(dbapi.ProgrammingError, dbapi.DatabaseError)
    assert issubclass(dbapi.NotSupportedError, dbapi.DatabaseError)


@pytest.mark.asyncio
async def test_async_connect_returns_connection():
    conn = await dbapi.connect(":memory:")
    assert conn is not None
    assert hasattr(conn, "cursor")
    assert hasattr(conn, "execute")
    assert hasattr(conn, "executemany")
    assert hasattr(conn, "commit")
    assert hasattr(conn, "rollback")
    assert hasattr(conn, "close")
    await conn.close()


@pytest.mark.asyncio
async def test_async_cursor():
    conn = await dbapi.connect(":memory:")
    cur = await conn.cursor()
    assert cur is not None
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_execute_returns_cursor():
    conn = await dbapi.connect(":memory:")
    cur = await conn.execute("SELECT 1")
    assert cur is not None
    row = await cur.fetchone()
    assert row is not None
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_executemany(unique_table_prefix):
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.executemany(f"INSERT INTO {tbl} VALUES (?)", [[1], [2], [3]])
    cur = await conn.execute(f"SELECT * FROM {tbl} ORDER BY a")
    rows = await cur.fetchall()
    assert len(rows) == 3
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_commit_rollback(test_db, unique_table_prefix):
    tbl = unique_table_prefix
    path = test_db
    conn = await dbapi.connect(path)
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.execute(f"INSERT INTO {tbl} VALUES (1)")
    await conn.commit()
    await conn.close()

    conn2 = await dbapi.connect(path)
    cur = await conn2.execute(f"SELECT * FROM {tbl}")
    rows = await cur.fetchall()
    await cur.close()
    assert len(rows) == 1
    await conn2.close()


@pytest.mark.asyncio
async def test_cursor_description_rowcount_lastrowid_arraysize(unique_table_prefix):
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (id INTEGER PRIMARY KEY, x TEXT)")
    ins_cur = await conn.execute(f"INSERT INTO {tbl} (x) VALUES (?)", ["hello"])
    assert ins_cur.lastrowid >= 0
    assert ins_cur.rowcount >= 0
    await ins_cur.close()
    sel_cur = await conn.execute(f"SELECT * FROM {tbl}")
    _ = await sel_cur.fetchone()  # trigger execution; description populated on fetch
    assert sel_cur.description is not None
    assert sel_cur.arraysize >= 1
    sel_cur.arraysize = 10
    assert sel_cur.arraysize == 10
    await sel_cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_cursor_async_iteration(unique_table_prefix):
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.executemany(f"INSERT INTO {tbl} VALUES (?)", [[1], [2], [3]])
    await conn.commit()
    cur = await conn.execute(f"SELECT * FROM {tbl} ORDER BY a")
    # Equivalent to async for: fetchone until None (lazy execution on first fetch)
    fetched = []
    while True:
        row = await cur.fetchone()
        if row is None:
            break
        fetched.append(row)
    assert len(fetched) == 3
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_cursor_async_for_after_execute(unique_table_prefix):
    """async for row in cursor works after execute(SELECT) (eager execution)."""
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.executemany(f"INSERT INTO {tbl} VALUES (?)", [[1], [2], [3]])
    await conn.commit()
    cur = await conn.execute(f"SELECT * FROM {tbl} ORDER BY a")
    fetched = []
    async for row in cur:
        fetched.append(row)
    assert len(fetched) == 3
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_connect_database_path():
    conn = await dbapi.connect(":memory:", timeout=5.0)
    cur = await conn.execute("SELECT 1")
    await cur.fetchone()
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_context_manager():
    async with (await dbapi.connect(":memory:")) as conn:
        cur = await conn.execute("SELECT 1")
        row = await cur.fetchone()
        assert row is not None
        await cur.close()
    # Exiting context manager closes connection; no explicit close needed


@pytest.mark.asyncio
async def test_connect_positional():
    conn = await dbapi.connect(":memory:")
    cur = await conn.execute("SELECT 1")
    await cur.fetchone()
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_connect_keyword():
    conn = await dbapi.connect(database=":memory:")
    cur = await conn.execute("SELECT 1")
    await cur.fetchone()
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_connect_missing_database_raises():
    with pytest.raises(dbapi.InterfaceError, match="database"):
        await dbapi.connect()


@pytest.mark.asyncio
async def test_concurrent_connection_usage_raises(unique_table_prefix):
    """Concurrent operations on same connection must raise ProgrammingError."""
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")

    async def run_select():
        cur = await conn.execute("SELECT 1")
        await cur.fetchone()
        await cur.close()

    # Run two selects concurrently; one should raise ProgrammingError
    with pytest.raises(dbapi.ProgrammingError, match="Concurrent operation"):
        await asyncio.gather(run_select(), run_select())

    await conn.close()


@pytest.mark.asyncio
async def test_cancellation_interrupts_and_connection_usable(unique_table_prefix):
    """Cancellation aborts the query and leaves the connection usable."""
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.executemany(f"INSERT INTO {tbl} VALUES (?)", [[i] for i in range(50)])
    await conn.commit()

    async def long_select():
        cur = await conn.execute(f"SELECT * FROM {tbl}")
        async for _ in cur:
            await asyncio.sleep(0.01)

    t = asyncio.create_task(long_select())
    await asyncio.sleep(0.05)
    t.cancel()
    with pytest.raises(asyncio.CancelledError):
        await t

    # Connection still usable
    cur = await conn.execute("SELECT 1")
    row = await cur.fetchone()
    await cur.close()
    assert row is not None
    await conn.close()
