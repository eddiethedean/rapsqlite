"""Tests for SQLite JSON1 extension with rapsqlite."""

import pytest

from rapsqlite import connect


@pytest.mark.asyncio
async def test_json_extract(test_db):
    """json_extract returns values from JSON text."""
    async with connect(test_db) as db:
        rows = await db.fetch_all(
            "SELECT json_extract('{\"a\":1,\"b\":2}', '$.a') as a, json_extract('{\"a\":1}', '$.b') as b"
        )
        assert rows == [[1, None]]


@pytest.mark.asyncio
async def test_json_object(test_db):
    """json_object builds JSON from key-value pairs."""
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT json_object('name', 'alice', 'age', 30)")
        assert len(rows) == 1
        assert "alice" in rows[0][0] and "30" in rows[0][0]


@pytest.mark.asyncio
async def test_json_each(test_db):
    """json_each expands JSON array/object to rows."""
    async with connect(test_db) as db:
        rows = await db.fetch_all("SELECT key, value FROM json_each('[1,2,3]')")
        assert len(rows) == 3
        assert [r[1] for r in rows] == [1, 2, 3]


@pytest.mark.asyncio
async def test_json_arrow_operators(test_db):
    """JSON -> and ->> operators work (SQLite 3.38+)."""
    async with connect(test_db) as db:
        rows = await db.fetch_all(
            "SELECT json('{\"x\":10}') -> 'x' as arrow, json('{\"x\":10}') ->> 'x' as arrow2"
        )
        assert len(rows) == 1
        # ->> returns text; -> returns JSON (may decode as int)
        assert rows[0][1] in ("10", 10)
