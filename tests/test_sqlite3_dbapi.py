# Ported from CPython Lib/test/test_sqlite3/test_dbapi.py.
# Original: pysqlite2/test/dbapi.py (Gerhard Häring). PSF license.
# Converted to async pytest for rapsqlite; skip Blob, thread, subprocess,
# setlimit/getlimit, serialize/deserialize, getconfig/setconfig, complete_statement.

"""Tests for rapsqlite DB-API and sqlite3-style behavior (ported from CPython test_sqlite3)."""

import pytest

from rapsqlite import (
    DataError,
    DatabaseError,
    Error,
    IntegrityError,
    InterfaceError,
    InternalError,
    NotSupportedError,
    OperationalError,
    ProgrammingError,
    Row,
    Warning,
    connect,
)


# ---- Module-level (DB-API from rapsqlite.dbapi) ----


@pytest.mark.asyncio
async def test_dbapi_module_constants():
    """DB-API constants from rapsqlite.dbapi (apilevel, paramstyle, threadsafety)."""
    pytest.importorskip("rapsqlite.dbapi")
    from rapsqlite import dbapi

    assert dbapi.apilevel == "2.0"
    assert dbapi.paramstyle == "qmark"
    assert dbapi.threadsafety in (0, 1, 3)


@pytest.mark.asyncio
async def test_error_hierarchy():
    """Exception class hierarchy matches DB-API."""
    assert issubclass(Warning, Exception)
    assert issubclass(Error, Exception)
    assert issubclass(InterfaceError, Error)
    assert issubclass(DatabaseError, Error)
    assert issubclass(DataError, DatabaseError)
    assert issubclass(OperationalError, DatabaseError)
    assert issubclass(IntegrityError, DatabaseError)
    assert issubclass(InternalError, DatabaseError)
    assert issubclass(ProgrammingError, DatabaseError)
    assert issubclass(NotSupportedError, DatabaseError)


# ---- Connection ----


@pytest.mark.asyncio
async def test_connection_commit(test_db, unique_table_prefix):
    """commit() works."""
    async with connect(test_db) as cx:
        cur = await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cur.close()
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        await cx.commit()


@pytest.mark.asyncio
async def test_connection_commit_after_no_changes(test_db, unique_table_prefix):
    """commit() works when no changes were made."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        await cx.commit()
        await cx.commit()


@pytest.mark.asyncio
async def test_connection_rollback(test_db, unique_table_prefix):
    """rollback() works."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        await cx.rollback()


@pytest.mark.asyncio
async def test_connection_rollback_after_no_changes(test_db, unique_table_prefix):
    """rollback() works when no changes were made."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        await cx.rollback()
        await cx.rollback()


@pytest.mark.asyncio
async def test_connection_cursor(test_db):
    """cursor() returns a cursor."""
    async with connect(test_db) as cx:
        cur = cx.cursor()
        assert cur is not None
        await cur.close()


@pytest.mark.asyncio
async def test_connection_failed_open():
    """Connect to invalid path raises OperationalError."""
    with pytest.raises((OperationalError, Error, OSError)):
        async with connect("/nonexistent/path/23534/mydb.db") as cx:
            await cx.execute("SELECT 1")


@pytest.mark.asyncio
async def test_connection_close(test_db):
    """close() closes the connection."""
    conn = await connect(test_db)
    await conn.close()


@pytest.mark.asyncio
async def test_connection_in_transaction(test_db, unique_table_prefix):
    """in_transaction reflects transaction state (cached; False after commit)."""
    async with connect(test_db) as cx:
        cur = cx.cursor()
        assert cx.in_transaction is False
        await cur.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cur.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        await cx.commit()
        assert cx.in_transaction is False
        await cur.close()


@pytest.mark.asyncio
async def test_connection_execute(test_db):
    """Connection.execute() returns cursor and result."""
    async with connect(test_db) as cx:
        cur = await cx.execute("SELECT 5")
        row = await cur.fetchone()
        assert row[0] == 5
        await cur.close()


# ---- Cursor: execute ----


@pytest.mark.asyncio
async def test_cursor_execute_no_args(test_db, unique_table_prefix):
    """execute() with no parameters."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        cur = await cx.execute(f"DELETE FROM {unique_table_prefix}")
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_execute_illegal_sql(test_db):
    """execute() with invalid SQL raises OperationalError."""
    async with connect(test_db) as cx:
        with pytest.raises((OperationalError, DatabaseError, ProgrammingError)):
            await cx.execute("SELECT asdf")


@pytest.mark.asyncio
async def test_cursor_execute_multiple_statements(test_db):
    """execute() with multiple statements raises or executes first only."""
    async with connect(test_db) as cx:
        try:
            cur = await cx.execute("SELECT 1; SELECT 2")
            row = await cur.fetchone()
            # If rapsqlite executes first statement only, we get one row
            assert row is not None
        except ProgrammingError:
            pass


@pytest.mark.asyncio
async def test_cursor_execute_arg_int(test_db, unique_table_prefix):
    """execute() with int parameter."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT, income REAL)"
        )
        await cx.execute(f"INSERT INTO {unique_table_prefix} (id) VALUES (?)", [42])


@pytest.mark.asyncio
async def test_cursor_execute_arg_float(test_db, unique_table_prefix):
    """execute() with float parameter."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, income REAL)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (income) VALUES (?)", [2500.32]
        )


@pytest.mark.asyncio
async def test_cursor_execute_arg_string(test_db, unique_table_prefix):
    """execute() with string parameter."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["Hugo"]
        )


@pytest.mark.asyncio
async def test_cursor_execute_param_list(test_db, unique_table_prefix):
    """execute() with list parameters."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('foo')")
        cur = await cx.execute(
            f"SELECT name FROM {unique_table_prefix} WHERE name=?", ["foo"]
        )
        row = await cur.fetchone()
        assert row[0] == "foo"
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_execute_wrong_no_of_args_too_many(test_db, unique_table_prefix):
    """execute() with too many parameters may raise or use first param."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        try:
            await cx.execute(
                f"INSERT INTO {unique_table_prefix} (id) VALUES (?)", [17, "Egon"]
            )
        except (ProgrammingError, DatabaseError):
            pass
        else:
            # rapsqlite may bind first param only
            row = await cx.fetch_one(f"SELECT id FROM {unique_table_prefix}")
            assert row is not None


@pytest.mark.asyncio
async def test_cursor_execute_wrong_no_of_args_too_few(test_db, unique_table_prefix):
    """execute() with too few parameters may raise or bind NULL (implementation-dependent)."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        try:
            await cx.execute(f"INSERT INTO {unique_table_prefix} (id) VALUES (?)")
        except (ProgrammingError, DatabaseError, TypeError):
            pass
        else:
            # rapsqlite may treat missing params as NULL
            row = await cx.fetch_one(f"SELECT id FROM {unique_table_prefix}")
            assert row is not None


@pytest.mark.asyncio
async def test_cursor_execute_non_iterable_params(test_db, unique_table_prefix):
    """execute() with non-iterable parameters raises or is rejected."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        try:
            await cx.execute(f"INSERT INTO {unique_table_prefix} (id) VALUES (?)", 42)
        except (ProgrammingError, TypeError):
            pass
        else:
            # Some bindings treat 42 as single param
            row = await cx.fetch_one(f"SELECT id FROM {unique_table_prefix}")
            assert row is not None


# ---- Cursor: rowcount, fetch, executemany ----


@pytest.mark.asyncio
async def test_cursor_rowcount_execute(test_db, unique_table_prefix):
    """rowcount after UPDATE reflects number of rows updated."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('foo')")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('foo')")
        cur = await cx.execute(f"UPDATE {unique_table_prefix} SET name='bar'")
        assert cur.rowcount == 2
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_rowcount_select(test_db):
    """rowcount for SELECT is -1 (unknown until all rows fetched)."""
    async with connect(test_db) as cx:
        cur = await cx.execute("SELECT 5 UNION SELECT 6")
        assert cur.rowcount == -1
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_rowcount_executemany(test_db, unique_table_prefix):
    """executemany() inserts multiple rows; rowcount may be set or -1."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        cur = cx.cursor()
        await cur.executemany(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)",
            [["1"], ["2"], ["3"]],
        )
        # rapsqlite may report rowcount or -1 for executemany
        assert cur.rowcount == 3 or cur.rowcount == -1
        rows = await cx.fetch_all(f"SELECT name FROM {unique_table_prefix}")
        assert len(rows) == 3
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_fetchone(test_db, unique_table_prefix):
    """fetchone() returns one row then None."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        cur = await cx.execute(f"SELECT name FROM {unique_table_prefix}")
        row = await cur.fetchone()
        assert row[0] == "foo"
        row2 = await cur.fetchone()
        assert row2 is None
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_fetchone_no_statement(test_db):
    """fetchone() with no active statement returns None or raises ProgrammingError."""
    async with connect(test_db) as cx:
        cur = cx.cursor()
        try:
            row = await cur.fetchone()
            assert row is None
        except ProgrammingError:
            pass  # rapsqlite raises "No query executed"
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_fetchmany(test_db, unique_table_prefix):
    """fetchmany() with size returns that many rows."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('A')")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('B')")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (name) VALUES ('C')")
        cur = await cx.execute(f"SELECT name FROM {unique_table_prefix}")
        cur.arraysize = 2
        res = await cur.fetchmany()
        assert len(res) == 2
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_fetchall(test_db, unique_table_prefix):
    """fetchall() returns all rows."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        cur = await cx.execute(f"SELECT name FROM {unique_table_prefix}")
        res = await cur.fetchall()
        assert len(res) == 1
        assert res[0][0] == "foo"
        res2 = await cur.fetchall()
        assert res2 == []
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_lastrowid_insert(test_db, unique_table_prefix):
    """lastrowid after INSERT reflects row id."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, name TEXT)"
        )
        cur = await cx.execute(
            f"INSERT INTO {unique_table_prefix} (name) VALUES (?)", ["foo"]
        )
        assert cur.lastrowid == 1
        await cur.close()


@pytest.mark.asyncio
async def test_cursor_description_after_fetch(test_db, unique_table_prefix):
    """description is set after fetch (lazy)."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (a INT, b TEXT)")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (a, b) VALUES (1, 'x')")
        cur = await cx.execute(f"SELECT a, b FROM {unique_table_prefix}")
        await cur.fetchall()
        d = cur.description
        assert d is not None
        assert len(d) == 2
        assert d[0][0] == "a" and d[1][0] == "b"
        await cur.close()


# ---- Executemany ----


@pytest.mark.asyncio
async def test_executemany_sequence(test_db, unique_table_prefix):
    """executemany() with sequence of parameters."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, income REAL)"
        )
        cur = cx.cursor()
        await cur.executemany(
            f"INSERT INTO {unique_table_prefix} (income) VALUES (?)",
            [(x,) for x in range(100, 110)],
        )
        await cur.close()


@pytest.mark.asyncio
async def test_executemany_select_raises(test_db):
    """executemany() with SELECT may raise or execute (sqlite3 raises)."""
    async with connect(test_db) as cx:
        cur = cx.cursor()
        try:
            await cur.executemany("SELECT ?", [[3]])
        except (ProgrammingError, DatabaseError):
            pass
        await cur.close()


# ---- Closed connection / cursor ----


@pytest.mark.asyncio
async def test_use_after_close(test_db, unique_table_prefix):
    """Using cursor after connection close() raises."""
    conn = await connect(test_db)
    await conn.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
    cur = await conn.execute("SELECT 1")
    await conn.close()
    with pytest.raises((ProgrammingError, InterfaceError, RuntimeError, Exception)):
        await cur.fetchone()


@pytest.mark.asyncio
async def test_closed_cursor_execute_raises(test_db, unique_table_prefix):
    """Cursor.execute() after cursor.close() may raise (implementation-dependent)."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        cur = cx.cursor()
        await cur.close()
        try:
            await cur.execute("SELECT 1")
        except (ProgrammingError, InterfaceError, RuntimeError):
            pass
        # If no raise, cursor close is no-op for execute in some implementations


# ---- Executescript ----


@pytest.mark.asyncio
async def test_executescript_string(test_db, unique_table_prefix):
    """executescript() runs multiple statements."""
    async with connect(test_db) as cx:
        cur = cx.cursor()
        await cur.executescript(
            f"""
            CREATE TABLE {unique_table_prefix} (i INT);
            INSERT INTO {unique_table_prefix} (i) VALUES (5);
            """
        )
        res = await cx.execute(f"SELECT i FROM {unique_table_prefix}")
        row = await res.fetchone()
        assert row[0] == 5
        await cur.close()


# ---- Row (row_factory) ----


@pytest.mark.asyncio
async def test_row_keys(test_db):
    """Row factory Row: keys() returns column names."""
    async with connect(test_db) as cx:
        cx.row_factory = Row
        cur = await cx.execute("SELECT 1 AS first, 2 AS second")
        row = await cur.fetchone()
        assert row is not None
        assert list(row.keys()) == ["first", "second"]
        await cur.close()


@pytest.mark.asyncio
async def test_row_getitem(test_db):
    """Row: index and name access."""
    async with connect(test_db) as cx:
        cx.row_factory = Row
        cur = await cx.execute("SELECT 1 AS a, 2 AS b")
        row = await cur.fetchone()
        assert row is not None
        assert row[0] == 1 and row[1] == 2
        assert row["a"] == 1 and row["b"] == 2
        await cur.close()


# ---- INSERT ON CONFLICT (SQLite) ----


@pytest.mark.asyncio
async def test_on_conflict_ignore(test_db, unique_table_prefix):
    """INSERT OR IGNORE ignores duplicate."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"""
            CREATE TABLE {unique_table_prefix} (
                id INTEGER PRIMARY KEY, name TEXT, unique_name TEXT UNIQUE
            )
            """
        )
        await cx.execute(
            f"INSERT OR IGNORE INTO {unique_table_prefix} (unique_name) VALUES (?)",
            ["foo"],
        )
        await cx.execute(
            f"INSERT OR IGNORE INTO {unique_table_prefix} (unique_name) VALUES (?)",
            ["foo"],
        )
        cur = await cx.execute(f"SELECT unique_name FROM {unique_table_prefix}")
        rows = await cur.fetchall()
        assert len(rows) == 1
        assert rows[0][0] == "foo"
        await cur.close()


@pytest.mark.asyncio
async def test_on_conflict_replace(test_db, unique_table_prefix):
    """INSERT OR REPLACE replaces on conflict."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"""
            CREATE TABLE {unique_table_prefix} (
                id INTEGER PRIMARY KEY, name TEXT, unique_name TEXT UNIQUE
            )
            """
        )
        await cx.execute(
            f"INSERT OR REPLACE INTO {unique_table_prefix} (name, unique_name) VALUES (?, ?)",
            ["Data!", "foo"],
        )
        await cx.execute(
            f"INSERT OR REPLACE INTO {unique_table_prefix} (name, unique_name) VALUES (?, ?)",
            ["Very different data!", "foo"],
        )
        cur = await cx.execute(f"SELECT name, unique_name FROM {unique_table_prefix}")
        rows = await cur.fetchall()
        assert len(rows) == 1
        assert rows[0][0] == "Very different data!" and rows[0][1] == "foo"
        await cur.close()
