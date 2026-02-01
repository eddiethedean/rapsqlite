"""Tests for SQLite FTS5 (Full-Text Search) with rapsqlite."""

import pytest

from rapsqlite import connect

pytestmark = [pytest.mark.unit]


@pytest.mark.asyncio
async def test_fts5_create_and_query(test_db):
    """FTS5 virtual table creation and MATCH queries work."""
    async with connect(test_db) as db:
        await db.execute("CREATE VIRTUAL TABLE docs USING fts5(title, content)")
        await db.execute(
            "INSERT INTO docs(title, content) VALUES (?, ?)",
            ["First doc", "Hello world and SQLite FTS"],
        )
        await db.execute(
            "INSERT INTO docs(title, content) VALUES (?, ?)",
            ["Second doc", "Full text search is powerful"],
        )
        rows = await db.fetch_all("SELECT * FROM docs WHERE docs MATCH 'world'")
        assert len(rows) >= 1
        assert any("Hello" in str(row) or "world" in str(row).lower() for row in rows)


@pytest.mark.asyncio
async def test_fts5_bm25(test_db):
    """FTS5 bm25() ranking works."""
    async with connect(test_db) as db:
        await db.execute("CREATE VIRTUAL TABLE fts_bm25 USING fts5(a, b)")
        await db.execute("INSERT INTO fts_bm25 VALUES ('x y z', 'a b c')")
        await db.execute("INSERT INTO fts_bm25 VALUES ('x y', 'a b')")
        rows = await db.fetch_all(
            "SELECT bm25(fts_bm25) FROM fts_bm25 WHERE fts_bm25 MATCH 'x' ORDER BY bm25(fts_bm25)"
        )
        assert len(rows) == 2
