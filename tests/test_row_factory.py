"""Robust tests for Connection.row_factory and Cursor row_factory behavior."""

import os
import sys
import tempfile

import pytest

from rapsqlite import DatabaseError, Row, connect


def _cleanup(path: str) -> None:
    if os.path.exists(path):
        try:
            os.unlink(path)
        except (PermissionError, OSError):
            if sys.platform != "win32":
                raise


@pytest.fixture
def test_db():
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        path = f.name
    try:
        yield path
    finally:
        _cleanup(path)


async def _ensure_table(db, table="t", cols="id INTEGER PRIMARY KEY, a TEXT, b REAL"):
    await db.execute(f"CREATE TABLE IF NOT EXISTS {table} ({cols})")
    await db.execute(f"DELETE FROM {table}")


# ---- fetch_one / fetch_optional ----


@pytest.mark.asyncio
async def test_row_factory_fetch_one(test_db):
    """fetch_one respects row_factory (None, dict, tuple, callable)."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('x', 1.5)")

        db.row_factory = None
        row = await db.fetch_one("SELECT * FROM t")
        assert row is not None
        assert isinstance(row, list)
        assert row[0] == 1 and row[1] == "x" and row[2] == 1.5

        db.row_factory = "dict"
        row = await db.fetch_one("SELECT * FROM t")
        assert row is not None
        assert isinstance(row, dict)
        assert row["id"] == 1 and row["a"] == "x" and row["b"] == 1.5

        db.row_factory = "tuple"
        row = await db.fetch_one("SELECT * FROM t")
        assert row is not None
        assert isinstance(row, tuple)
        assert row[0] == 1 and row[1] == "x" and row[2] == 1.5

        def fac(r):
            return (r[0] * 10, r[1].upper())

        db.row_factory = fac
        row = await db.fetch_one("SELECT * FROM t")
        assert row == (10, "X")


@pytest.mark.asyncio
async def test_row_factory_fetch_one_no_rows(test_db):
    """fetch_one raises when no rows; fetch_optional returns None."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        # no inserts

        for factory in (None, "dict", "tuple"):
            db.row_factory = factory
            with pytest.raises(DatabaseError, match="no rows returned"):
                await db.fetch_one("SELECT * FROM t")
            opt = await db.fetch_optional("SELECT * FROM t")
            assert opt is None

        def fac(r):
            return {"k": r[0]}

        db.row_factory = fac
        with pytest.raises(DatabaseError, match="no rows returned"):
            await db.fetch_one("SELECT * FROM t")
        opt = await db.fetch_optional("SELECT * FROM t")
        assert opt is None


@pytest.mark.asyncio
async def test_row_factory_fetch_optional_some(test_db):
    """fetch_optional returns factory-shaped row when a row exists."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('y', 2.0)")

        db.row_factory = "dict"
        opt = await db.fetch_optional("SELECT * FROM t")
        assert opt is not None
        assert isinstance(opt, dict)
        assert opt["a"] == "y"

        db.row_factory = "tuple"
        opt = await db.fetch_optional("SELECT * FROM t")
        assert opt is not None
        assert isinstance(opt, tuple)
        assert opt[1] == "y"


# ---- fetch_all: empty and multiple rows ----


@pytest.mark.asyncio
async def test_row_factory_fetch_all_empty(test_db):
    """fetch_all returns [] with each factory when no rows."""
    async with connect(test_db) as db:
        await _ensure_table(db)

        for factory in (None, "dict", "tuple"):
            db.row_factory = factory
            rows = await db.fetch_all("SELECT * FROM t")
            assert rows == []

        db.row_factory = lambda r: r[0]
        rows = await db.fetch_all("SELECT * FROM t")
        assert rows == []


@pytest.mark.asyncio
async def test_row_factory_fetch_all_multiple_rows(test_db):
    """All rows are transformed, not just the first."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('a', 1.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('b', 2.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('c', 3.0)")

        db.row_factory = None
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 3
        assert (
            rows[0] == [1, "a", 1.0]
            and rows[1] == [2, "b", 2.0]
            and rows[2] == [3, "c", 3.0]
        )

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 3
        assert rows[0]["a"] == "a" and rows[1]["a"] == "b" and rows[2]["a"] == "c"

        db.row_factory = "tuple"
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 3
        assert rows[0][1] == "a" and rows[1][1] == "b" and rows[2][1] == "c"

        def fac(r):
            return r[1] + str(r[2])

        db.row_factory = fac
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert rows == ["a1.0", "b2.0", "c3.0"]


# ---- NULLs and duplicate column names ----


@pytest.mark.asyncio
async def test_row_factory_null_values(test_db):
    """NULLs are represented as None in list/dict/tuple/callable."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES (NULL, 1.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('x', NULL)")

        db.row_factory = None
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert rows[0][1] is None and rows[0][2] == 1.0
        assert rows[1][1] == "x" and rows[1][2] is None

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert rows[0]["a"] is None and rows[0]["b"] == 1.0
        assert rows[1]["a"] == "x" and rows[1]["b"] is None

        db.row_factory = "tuple"
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert rows[0][1] is None and rows[0][2] == 1.0
        assert rows[1][1] == "x" and rows[1][2] is None

        def fac(r):
            return (r[1], r[2])

        db.row_factory = fac
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert rows[0] == (None, 1.0) and rows[1] == ("x", None)


@pytest.mark.asyncio
async def test_row_factory_duplicate_column_names(test_db):
    """With duplicate column names, dict uses last occurrence (overwrites)."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('v', 1.0)")

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT a AS x, a AS x, b FROM t")
        assert len(rows) == 1
        assert "x" in rows[0]
        assert rows[0]["b"] == 1.0
        # Second 'x' overwrites first; value is still 'v'
        assert rows[0]["x"] == "v"


# ---- Cursor ----


@pytest.mark.asyncio
async def test_row_factory_cursor_fetchone_fetchall_fetchmany(test_db):
    """Cursor fetchone/fetchall/fetchmany use Connection row_factory at creation."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('a', 1.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('b', 2.0)")

        db.row_factory = "dict"
        async with db.cursor() as cur:
            await cur.execute("SELECT * FROM t ORDER BY id")
            one = await cur.fetchone()
            assert isinstance(one, dict)
            assert one["a"] == "a"
            rest = await cur.fetchall()
            assert len(rest) == 1 and isinstance(rest[0], dict)
            assert rest[0]["a"] == "b"

        db.row_factory = "tuple"
        async with db.cursor() as cur:
            await cur.execute("SELECT * FROM t ORDER BY id")
            one = await cur.fetchone()
            assert isinstance(one, tuple)
            assert one[1] == "a"
            many = await cur.fetchmany(1)
            assert len(many) == 1 and isinstance(many[0], tuple)
            assert many[0][1] == "b"


@pytest.mark.asyncio
async def test_row_factory_cursor_uses_connection_factory(test_db):
    """Cursor uses Connection's row_factory (shared state)."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('x', 1.0)")

        db.row_factory = "dict"
        async with db.cursor() as cur:
            await cur.execute("SELECT * FROM t")
            row = await cur.fetchone()
            assert isinstance(row, dict)
            assert row["a"] == "x"


@pytest.mark.asyncio
async def test_row_factory_cursor_empty_and_multiple(test_db):
    """Cursor fetchone returns None when no rows; fetchall returns []."""
    async with connect(test_db) as db:
        await _ensure_table(db)

        db.row_factory = "dict"
        async with db.cursor() as cur:
            await cur.execute("SELECT * FROM t")
            assert await cur.fetchone() is None
            assert await cur.fetchall() == []
            assert await cur.fetchmany(5) == []


# ---- Parameterized queries and transactions ----


@pytest.mark.asyncio
async def test_row_factory_parameterized_queries(test_db):
    """fetch_all/fetch_one/fetch_optional with parameters respect row_factory."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('a', 1.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('b', 2.0)")
        await db.execute("INSERT INTO t (a, b) VALUES ('c', 3.0)")

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT * FROM t WHERE id > ?", [1])
        assert len(rows) == 2
        assert rows[0]["id"] == 2 and rows[1]["id"] == 3

        row = await db.fetch_one("SELECT * FROM t WHERE a = ?", ["b"])
        assert row is not None and row["a"] == "b"

        opt = await db.fetch_optional("SELECT * FROM t WHERE a = ?", ["z"])
        assert opt is None


@pytest.mark.asyncio
async def test_row_factory_in_transaction(test_db):
    """fetch_* inside transaction() respect row_factory."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('a', 1.0)")

        db.row_factory = "tuple"
        async with db.transaction():
            rows = await db.fetch_all("SELECT * FROM t")
            assert len(rows) == 1 and isinstance(rows[0], tuple)
            assert rows[0][1] == "a"
            one = await db.fetch_one("SELECT * FROM t")
            assert isinstance(one, tuple)


# ---- Callable edge cases ----


@pytest.mark.asyncio
async def test_row_factory_callable_returns_tuple(test_db):
    """Custom factory can return tuple (or any type), not only dict."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('x', 2.5)")

        db.row_factory = lambda r: (r[0], r[1].upper(), r[2] * 2)
        row = await db.fetch_one("SELECT * FROM t")
        assert row == (1, "X", 5.0)

        db.row_factory = lambda r: r[2]
        rows = await db.fetch_all("SELECT * FROM t")
        assert rows == [2.5]


@pytest.mark.asyncio
async def test_row_factory_callable_receives_list(test_db):
    """Custom factory receives a list; row[0], row[1] etc. work."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('hi', 3.14)")

        seen = []

        def fac(row):
            seen.append(list(row))
            return row[0] + row[2]

        db.row_factory = fac
        out = await db.fetch_one("SELECT * FROM t")
        assert out == 1 + 3.14
        assert len(seen) == 1
        assert seen[0] == [1, "hi", 3.14]


# ---- Switching factory mid-session ----


@pytest.mark.asyncio
async def test_row_factory_switch_mid_session(test_db):
    """Switching row_factory between fetches works correctly."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('q', 1.0)")

        db.row_factory = None
        r1 = await db.fetch_one("SELECT * FROM t")
        assert isinstance(r1, list)

        db.row_factory = "dict"
        r2 = await db.fetch_one("SELECT * FROM t")
        assert isinstance(r2, dict)

        db.row_factory = "tuple"
        r3 = await db.fetch_one("SELECT * FROM t")
        assert isinstance(r3, tuple)

        db.row_factory = None
        r4 = await db.fetch_one("SELECT * FROM t")
        assert isinstance(r4, list)


@pytest.mark.asyncio
async def test_row_factory_getter_setter_roundtrip(test_db):
    """Getter returns what we set; setter accepts None and values."""
    async with connect(test_db) as db:
        assert db.row_factory is None

        db.row_factory = "dict"
        assert db.row_factory == "dict"

        db.row_factory = "tuple"
        assert db.row_factory == "tuple"

        def f(row):
            return row

        db.row_factory = f
        assert db.row_factory is f

        db.row_factory = None
        assert db.row_factory is None


@pytest.mark.asyncio
async def test_row_factory_single_column(test_db):
    """Single-column SELECT works with each factory."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('x', 1.0)")

        db.row_factory = None
        rows = await db.fetch_all("SELECT id FROM t")
        assert rows == [[1]]

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT id FROM t")
        assert rows == [{"id": 1}]

        db.row_factory = "tuple"
        rows = await db.fetch_all("SELECT id FROM t")
        assert rows == [(1,)]

        db.row_factory = lambda r: r[0] * 2
        rows = await db.fetch_all("SELECT id FROM t")
        assert rows == [2]


@pytest.mark.asyncio
async def test_row_factory_rapsqlite_row_mixed_access(test_db):
    """rapsqlite.Row supports both index and key access with keys()/values()/items()."""
    async with connect(test_db) as db:
        await _ensure_table(db)
        await db.execute("INSERT INTO t (a, b) VALUES ('x', 1.5)")

        db.row_factory = Row
        row = await db.fetch_one("SELECT * FROM t")
        # Index access
        assert row[0] == 1
        assert row[1] == "x"
        assert row[2] == 1.5
        # Key access
        assert row["id"] == 1
        assert row["a"] == "x"
        assert row["b"] == 1.5
        # Mapping-style helpers
        keys = list(row.keys())
        assert "id" in keys and "a" in keys and "b" in keys
        values = list(row.values())
        assert 1 in values and "x" in values and 1.5 in values
        items = dict(row.items())
        assert items["id"] == 1
        assert items["a"] == "x"
        assert items["b"] == 1.5

        # __contains__ and __len__
        assert "id" in row
        assert "missing" not in row
        assert 0 in row
        assert 2 in row
        assert 3 not in row
        assert len(row) == 3


@pytest.mark.asyncio
async def test_row_factory_blob(test_db):
    """BLOB columns work with dict/tuple/list factories."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE bin (id INTEGER PRIMARY KEY, data BLOB)")
        await db.execute("INSERT INTO bin (data) VALUES (?)", [b"\x00\x01\x02"])

        db.row_factory = "dict"
        rows = await db.fetch_all("SELECT * FROM bin")
        assert rows[0]["data"] == b"\x00\x01\x02"

        db.row_factory = "tuple"
        rows = await db.fetch_all("SELECT * FROM bin")
        assert rows[0][1] == b"\x00\x01\x02"

        db.row_factory = None
        rows = await db.fetch_all("SELECT * FROM bin")
        assert rows[0][1] == b"\x00\x01\x02"
