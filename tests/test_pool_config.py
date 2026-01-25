"""Robust tests for Phase 2.4 pool configuration (pool_size, connection_timeout)."""

import os
import sys
import tempfile

import pytest

from rapsqlite import connect


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


# ---- Validation: negative values ----


@pytest.mark.asyncio
async def test_pool_size_rejects_negative(test_db):
    """Setting pool_size to a negative value raises ValueError."""
    async with connect(test_db) as db:
        with pytest.raises(ValueError, match="pool_size must be >= 0"):
            db.pool_size = -1
        assert db.pool_size is None


@pytest.mark.asyncio
async def test_connection_timeout_rejects_negative(test_db):
    """Setting connection_timeout to a negative value raises ValueError."""
    async with connect(test_db) as db:
        with pytest.raises(ValueError, match="connection_timeout must be >= 0"):
            db.connection_timeout = -1
        assert db.connection_timeout is None


# ---- Validation: invalid types ----


@pytest.mark.asyncio
async def test_pool_size_rejects_non_int(test_db):
    """Setting pool_size to a non-int (e.g. str) raises TypeError."""
    async with connect(test_db) as db:
        with pytest.raises((TypeError, ValueError)):
            db.pool_size = "10"


@pytest.mark.asyncio
async def test_connection_timeout_rejects_non_int(test_db):
    """Setting connection_timeout to a non-int (e.g. str) raises TypeError."""
    async with connect(test_db) as db:
        with pytest.raises((TypeError, ValueError)):
            db.connection_timeout = "30"


# ---- Config applied before first use ----


@pytest.mark.asyncio
async def test_pool_config_before_execute(test_db):
    """Set pool_size and connection_timeout before any DB op; execute works."""
    async with connect(test_db) as db:
        db.pool_size = 2
        db.connection_timeout = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1
        assert db.pool_size == 2
        assert db.connection_timeout == 5


@pytest.mark.asyncio
async def test_pool_config_before_fetch(test_db):
    """Set config before any op; fetch_* creates pool with config."""
    async with connect(test_db) as db:
        db.pool_size = 3
        db.connection_timeout = 10
        rows = await db.fetch_all("SELECT 1 AS a, 2 AS b")
        assert rows == [[1, 2]]
        assert db.pool_size == 3
        assert db.connection_timeout == 10


# ---- Config + transaction ----


@pytest.mark.asyncio
async def test_pool_config_with_transaction(test_db):
    """Pool config set; transaction() uses it when creating pool."""
    async with connect(test_db) as db:
        db.pool_size = 4
        db.connection_timeout = 15
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        async with db.transaction():
            await db.execute("INSERT INTO t (v) VALUES ('a')")
            await db.execute("INSERT INTO t (v) VALUES ('b')")
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 2
        assert rows[0][1] == "a" and rows[1][1] == "b"


# ---- Config + cursor ----


@pytest.mark.asyncio
async def test_pool_config_with_cursor(test_db):
    """Pool config set; cursor execute/fetch use it."""
    async with connect(test_db) as db:
        db.pool_size = 5
        db.connection_timeout = 20
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x INTEGER)")
        await db.execute("INSERT INTO t (x) VALUES (1), (2), (3)")
        cur = db.cursor()
        await cur.execute("SELECT * FROM t WHERE x > 1")
        out = await cur.fetchall()
        assert len(out) == 2
        assert [r[1] for r in out] == [2, 3]


# ---- Config + set_pragma ----


@pytest.mark.asyncio
async def test_pool_config_with_set_pragma(test_db):
    """set_pragma triggers pool creation; pool config is used."""
    async with connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 3
        await db.set_pragma("journal_mode", "DELETE")
        rows = await db.fetch_all("PRAGMA journal_mode")
        assert len(rows) == 1
        assert db.pool_size == 1
        assert db.connection_timeout == 3


# ---- execute_many ----


@pytest.mark.asyncio
async def test_pool_config_with_execute_many(test_db):
    """execute_many (no transaction) uses pool config."""
    async with connect(test_db) as db:
        db.pool_size = 2
        db.connection_timeout = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        await db.execute_many("INSERT INTO t (v) VALUES (?)", [["a"], ["b"], ["c"]])
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 3


@pytest.mark.asyncio
async def test_pool_config_with_execute_many_in_transaction(test_db):
    """execute_many inside transaction uses pool config."""
    async with connect(test_db) as db:
        db.pool_size = 2
        db.connection_timeout = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        async with db.transaction():
            await db.execute_many("INSERT INTO t (v) VALUES (?)", [["x"], ["y"]])
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id")
        assert len(rows) == 2


# ---- begin() ----


@pytest.mark.asyncio
async def test_pool_config_with_begin(test_db):
    """begin() creates pool; pool config is used."""
    async with connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 2
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.begin()
        await db.execute("INSERT INTO t DEFAULT VALUES")
        await db.commit()
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


# ---- Edge: both zero ----


@pytest.mark.asyncio
async def test_pool_config_both_zero_stored(test_db):
    """pool_size=0 and connection_timeout=0 are stored and returned by getters."""
    async with connect(test_db) as db:
        db.pool_size = 0
        db.connection_timeout = 0
        assert db.pool_size == 0
        assert db.connection_timeout == 0
    # Note: connection_timeout=0 yields acquire_timeout(0); pool acquire can
    # timeout immediately. We only assert storage/getter here.


@pytest.mark.asyncio
async def test_pool_config_pool_size_zero_ops_succeed(test_db):
    """pool_size=0 (stored) with non-zero timeout; DB ops succeed."""
    async with connect(test_db) as db:
        db.pool_size = 0
        db.connection_timeout = 5
        assert db.pool_size == 0
        assert db.connection_timeout == 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        rows = await db.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


# ---- Config switch mid-session ----


@pytest.mark.asyncio
async def test_pool_config_switch_mid_session(test_db):
    """Changing config after pool exists updates getter; stored value persists."""
    async with connect(test_db) as db:
        db.pool_size = 2
        db.connection_timeout = 10
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        assert db.pool_size == 2
        assert db.connection_timeout == 10
        db.pool_size = 10
        db.connection_timeout = 60
        assert db.pool_size == 10
        assert db.connection_timeout == 60
        await db.execute("INSERT INTO t DEFAULT VALUES")
        assert db.pool_size == 10
        assert db.connection_timeout == 60


# ---- fetch_one / fetch_optional with config ----


@pytest.mark.asyncio
async def test_pool_config_fetch_one_optional(test_db):
    """fetch_one and fetch_optional with config set before any use."""
    async with connect(test_db) as db:
        db.pool_size = 1
        db.connection_timeout = 5
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
        await db.execute("INSERT INTO t (v) VALUES ('only')")
        one = await db.fetch_one("SELECT * FROM t")
        assert one is not None
        assert one[1] == "only"
        none_row = await db.fetch_optional("SELECT * FROM t WHERE 1=0")
        assert none_row is None


# ---- Multiple connections independent config ----


@pytest.mark.asyncio
async def test_pool_config_multiple_connections_independent(test_db):
    """Two connections can have different pool config; both work."""
    async with connect(test_db) as db1:
        await db1.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        db1.pool_size = 2
        db1.connection_timeout = 5
        await db1.execute("INSERT INTO t DEFAULT VALUES")

    async with connect(test_db) as db2:
        db2.pool_size = 10
        db2.connection_timeout = 60
        rows = await db2.fetch_all("SELECT * FROM t")
        assert len(rows) == 1
        assert db2.pool_size == 10
        assert db2.connection_timeout == 60

    async with connect(test_db) as db3:
        assert db3.pool_size is None
        assert db3.connection_timeout is None
        rows = await db3.fetch_all("SELECT * FROM t")
        assert len(rows) == 1


# ---- Large values ----


@pytest.mark.asyncio
async def test_pool_config_large_values(test_db):
    """Large pool_size and connection_timeout are accepted and persist."""
    async with connect(test_db) as db:
        db.pool_size = 1000
        db.connection_timeout = 86400
        assert db.pool_size == 1000
        assert db.connection_timeout == 86400
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        await db.execute("INSERT INTO t DEFAULT VALUES")
        assert db.pool_size == 1000
        assert db.connection_timeout == 86400
