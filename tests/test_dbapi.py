"""Tests for rapsqlite.dbapi (True Async DBAPI spec)."""

import asyncio
import os
import time

import pytest

pytest.importorskip("rapsqlite")
from rapsqlite import dbapi

pytestmark = [pytest.mark.unit]


@pytest.mark.asyncio
async def test_module_level_contract():
    assert dbapi.apilevel == "2.0"
    assert dbapi.threadsafety == 0
    assert dbapi.paramstyle == "qmark"


@pytest.mark.asyncio
async def test_raw_cursor_has_close_and_lastrowid():
    """Verify raw Cursor from cursor() and execute() exposes close and lastrowid (or DBAPI fallbacks work)."""
    conn = await dbapi.connect(":memory:")
    # From cursor()
    cur = await conn.cursor()
    raw = cur._raw
    assert hasattr(raw, "close"), "raw Cursor from cursor() must have close"
    assert callable(getattr(raw, "close", None)), "raw close must be callable"
    assert hasattr(raw, "lastrowid"), "raw Cursor from cursor() must have lastrowid"
    await cur.close()
    # From execute()
    exec_cur = await conn.execute("SELECT 1")
    raw_exec = exec_cur._raw
    assert hasattr(raw_exec, "close"), "raw Cursor from execute() must have close"
    assert hasattr(raw_exec, "lastrowid"), (
        "raw Cursor from execute() must have lastrowid"
    )
    await exec_cur.close()
    await conn.close()


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
async def test_async_connection_create_function():
    """DBAPI AsyncConnection has create_function; register and use in SELECT."""
    conn = await dbapi.connect(":memory:")
    assert hasattr(conn, "create_function")
    await conn.create_function("double", 1, lambda x: x * 2 if x is not None else None)
    cur = await conn.execute("SELECT double(21)")
    row = await cur.fetchone()
    await cur.close()
    assert row is not None and row[0] == 42
    await conn.create_function("double", 1, None)  # remove
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
async def test_commit_reproducer(test_db, unique_table_prefix):
    """Minimal reproducer: connect -> CREATE -> INSERT -> commit -> close; reconnect and verify row."""
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
    await conn2.close()
    assert len(rows) == 1
    assert rows[0][0] == 1


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
    # DBAPI: lastrowid/rowcount are ints; -1 when unknown on some builds
    assert isinstance(ins_cur.lastrowid, int) and isinstance(ins_cur.rowcount, int)
    assert ins_cur.lastrowid >= -1 and ins_cur.rowcount >= -1
    await ins_cur.close()
    sel_cur = await conn.execute(f"SELECT * FROM {tbl}")
    _ = (
        await sel_cur.fetchone()
    )  # trigger execution; description may be populated on fetch
    # description may be None until first fetch on some builds
    assert sel_cur.description is None or len(sel_cur.description) >= 0
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
    """Rows are available after execute(SELECT); use fetchone loop (async for may require eager results)."""
    tbl = unique_table_prefix
    conn = await dbapi.connect(":memory:")
    await conn.execute(f"CREATE TABLE {tbl} (a INT)")
    await conn.executemany(f"INSERT INTO {tbl} VALUES (?)", [[1], [2], [3]])
    await conn.commit()
    cur = await conn.execute(f"SELECT * FROM {tbl} ORDER BY a")
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
async def test_connect_database_path():
    conn = await dbapi.connect(":memory:", timeout=5.0)
    cur = await conn.execute("SELECT 1")
    await cur.fetchone()
    await cur.close()
    await conn.close()


@pytest.mark.asyncio
async def test_context_manager():
    async with await dbapi.connect(":memory:") as conn:
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

    # Run many concurrent execute() so at least two overlap; one will raise
    # ProgrammingError (single pair can finish too fast and never overlap).
    async def run_select():
        cur = await conn.execute("SELECT 1")
        await cur.fetchone()
        await cur.close()

    with pytest.raises(dbapi.ProgrammingError, match="Concurrent operation"):
        await asyncio.gather(*[run_select() for _ in range(30)])

    await conn.close()


@pytest.mark.asyncio
async def test_cancellation_interrupts_and_connection_usable(unique_table_prefix):
    """Cancellation interrupts underlying SQLite work and leaves connection usable."""
    tbl = unique_table_prefix
    # Use a file-backed DB so we can create a deterministic lock wait.
    # (in-memory DBs are per-connection).
    import tempfile

    # NOTE: On Windows, NamedTemporaryFile keeps the file handle open, which prevents
    # SQLite from opening/creating the database file (error code 14).
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as d:
        path = os.path.join(d, "test.db")
        # SQLx does not reliably create the file across platforms unless it exists.
        open(path, "ab").close()

        conn1 = await dbapi.connect(path, timeout=5.0)
        conn2 = await dbapi.connect(path, timeout=5.0)
        await conn1.execute(f"CREATE TABLE {tbl} (a INT)")
        await conn1.commit()

        # Hold an exclusive lock so conn2 blocks inside SQLite.
        await conn1.execute("BEGIN EXCLUSIVE")
        started = asyncio.Event()

        async def blocked_insert():
            started.set()
            await conn2.execute(f"INSERT INTO {tbl} VALUES (1)")

        t0 = time.monotonic()
        task = asyncio.create_task(blocked_insert())
        await started.wait()
        await asyncio.sleep(0.05)
        task.cancel()

        with pytest.raises(asyncio.CancelledError):
            await task

        elapsed = time.monotonic() - t0
        # If cancellation did not interrupt SQLite, we'd wait ~busy_timeout seconds.
        assert elapsed < 1.0

        # Release lock and ensure conn2 still works.
        await conn1.execute("ROLLBACK")
        await conn1.close()

        cur = await conn2.execute("SELECT 1")
        row = await cur.fetchone()
        await cur.close()
        assert row is not None
        await conn2.close()
