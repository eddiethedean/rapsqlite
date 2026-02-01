"""Smoke test for aiohttp + rapsqlite integration pattern."""

from typing import Any

import pytest

pytest.importorskip("aiohttp")

from rapsqlite import connect

pytestmark = [pytest.mark.integration]


async def _fetch_items(db_path: str) -> dict[str, Any]:
    """Simulate an aiohttp handler that fetches items from the database.

    In a real aiohttp app, this would be called from a request handler
    and the result would be passed to web.json_response().
    """
    async with connect(db_path) as conn:  # type: ignore[attr-defined]
        rows = await conn.fetch_all("SELECT id, name FROM items")
        return {"items": [list(r) for r in rows]}


@pytest.mark.filterwarnings("ignore::pytest.PytestUnraisableExceptionWarning")
@pytest.mark.asyncio
async def test_aiohttp_rapsqlite_smoke(test_db: str) -> None:
    """Test aiohttp handler pattern with rapsqlite.

    This tests the database access pattern used in aiohttp handlers
    without involving aiohttp's request/response machinery which
    requires network sockets.
    """
    # Set up the database
    async with connect(test_db) as conn:  # type: ignore[attr-defined]
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (id, name) VALUES (1, 'foo')")

    # Simulate calling a handler function
    result = await _fetch_items(test_db)

    assert result == {"items": [[1, "foo"]]}
