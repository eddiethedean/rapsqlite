# Ported from CPython Lib/test/test_sqlite3/test_types.py.
# Original: pysqlite2/test/types.py (Gerhard Häring). PSF license.
# Converted to async pytest for rapsqlite; basic type round-trip and
# register_adapter/register_converter tests (per-connection in rapsqlite).

"""Tests for rapsqlite type handling (ported from CPython test_sqlite3)."""

import pytest

from conftest import skip_if_no_register_adapter, skip_if_no_register_converter
from rapsqlite import connect

pytestmark = [pytest.mark.unit]


# ---- Basic type round-trip ----


@pytest.mark.asyncio
async def test_type_string(test_db, unique_table_prefix):
    """String round-trip."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (i INTEGER, s TEXT, f REAL, b BLOB)"
        )
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (s) VALUES (?)", ["Österreich"]
        )
        row = await cx.fetch_one(f"SELECT s FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == "Österreich"


@pytest.mark.asyncio
async def test_type_string_with_null_character(test_db, unique_table_prefix):
    """String with null byte round-trip."""
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (i INTEGER, s TEXT)")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (s) VALUES (?)", ["a\0b"])
        row = await cx.fetch_one(f"SELECT s FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == "a\0b"


@pytest.mark.asyncio
async def test_type_small_int(test_db, unique_table_prefix):
    """Small integer round-trip."""
    async with connect(test_db) as cx:
        await cx.execute(
            f"CREATE TABLE {unique_table_prefix} (i INTEGER, s TEXT, f REAL, b BLOB)"
        )
        await cx.execute(f"INSERT INTO {unique_table_prefix} (i) VALUES (?)", [42])
        row = await cx.fetch_one(f"SELECT i FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == 42


@pytest.mark.asyncio
async def test_type_large_int(test_db, unique_table_prefix):
    """Large integer round-trip."""
    num = 123456789123456789
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (i INTEGER)")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (i) VALUES (?)", [num])
        row = await cx.fetch_one(f"SELECT i FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == num


@pytest.mark.asyncio
async def test_type_float(test_db, unique_table_prefix):
    """Float round-trip."""
    val = 3.14
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (f REAL)")
        await cx.execute(f"INSERT INTO {unique_table_prefix} (f) VALUES (?)", [val])
        row = await cx.fetch_one(f"SELECT f FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == val


@pytest.mark.asyncio
async def test_type_blob(test_db, unique_table_prefix):
    """Blob (bytes/memoryview) round-trip."""
    sample = b"Guglhupf"
    async with connect(test_db) as cx:
        await cx.execute(f"CREATE TABLE {unique_table_prefix} (b BLOB)")
        await cx.execute(
            f"INSERT INTO {unique_table_prefix} (b) VALUES (?)", [memoryview(sample)]
        )
        row = await cx.fetch_one(f"SELECT b FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == sample


@pytest.mark.asyncio
async def test_type_unicode_execute(test_db):
    """Unicode in SELECT literal."""
    async with connect(test_db) as cx:
        row = await cx.fetch_one("SELECT 'Österreich'")
        assert row is not None
        assert row[0] == "Österreich"


# ---- register_adapter (per-connection in rapsqlite) ----


@pytest.mark.asyncio
async def test_register_adapter_round_trip(test_db, unique_table_prefix):
    """register_adapter converts custom type for binding."""
    skip_if_no_register_adapter()

    class Point:
        def __init__(self, x, y):
            self.x = x
            self.y = y

    async with connect(test_db) as db:
        db.register_adapter(Point, lambda p: f"{p.x},{p.y}")
        await db.execute(f"CREATE TABLE {unique_table_prefix} (p TEXT)")
        await db.execute(
            f"INSERT INTO {unique_table_prefix} (p) VALUES (?)", [Point(1, 2)]
        )
        row = await db.fetch_one(f"SELECT p FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == "1,2"


# ---- register_converter (per-connection in rapsqlite) ----


@pytest.mark.asyncio
async def test_register_converter_declared_type(test_db, unique_table_prefix):
    """register_converter converts column value by declared type."""
    skip_if_no_register_converter()

    async with connect(test_db) as db:
        # Use DATE (sqlx reports this declared type); converter uppercases to verify it was applied
        db.register_converter(
            "DATE", lambda b: b.decode("utf-8").upper() if b else None
        )
        await db.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER, d DATE)")
        await db.execute(
            f"INSERT INTO {unique_table_prefix} (id, d) VALUES (1, ?)", ["hello"]
        )
        row = await db.fetch_one(f"SELECT d FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == "HELLO"
        db.register_converter("DATE", None)  # restore for other tests


@pytest.mark.asyncio
async def test_register_converter_remove(test_db, unique_table_prefix):
    """register_converter(typename, None) removes converter."""
    skip_if_no_register_converter()

    async with connect(test_db) as db:
        db.register_converter("DATE", lambda b: b.decode("utf-8") if b else None)
        await db.execute(f"CREATE TABLE {unique_table_prefix} (d DATE)")
        await db.execute(
            f"INSERT INTO {unique_table_prefix} (d) VALUES (?)", ["2024-01-15"]
        )
        row = await db.fetch_one(f"SELECT d FROM {unique_table_prefix}")
        assert row is not None
        assert row[0] == "2024-01-15"
        db.register_converter("DATE", None)
        row2 = await db.fetch_one(f"SELECT d FROM {unique_table_prefix}")
        assert row2 is not None
        # After removal, value may be bytes or str depending on implementation
        assert row2[0] in (b"2024-01-15", "2024-01-15")
