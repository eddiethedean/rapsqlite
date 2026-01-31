"""Smoke test for aiohttp + rapsqlite integration pattern."""

import pytest

pytest.importorskip("aiohttp")

from aiohttp import web

from rapsqlite import connect


def _make_app(db_path: str) -> web.Application:
    async def homepage(request):
        async with connect(db_path) as conn:
            rows = await conn.fetch_all("SELECT id, name FROM items")
            return web.json_response({"items": [list(r) for r in rows]})

    app = web.Application()
    app.router.add_get("/", homepage)
    return app


@pytest.mark.asyncio
async def test_aiohttp_rapsqlite_smoke(test_db: str) -> None:
    async with connect(test_db) as conn:
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (id, name) VALUES (1, 'foo')")
    app = _make_app(test_db)
    from aiohttp.test_utils import TestClient, TestServer

    async with TestClient(TestServer(app)) as client:
        r = await client.get("/")
        assert r.status == 200
        data = await r.json()
        assert data == {"items": [[1, "foo"]]}
