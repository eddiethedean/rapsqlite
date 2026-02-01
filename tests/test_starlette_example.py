"""Smoke test for Starlette + rapsqlite integration pattern."""

import pytest

pytest.importorskip("starlette")
pytest.importorskip("httpx")  # starlette.testclient requires httpx

from starlette.applications import Starlette
from starlette.routing import Route
from starlette.testclient import TestClient

from rapsqlite import connect


def _make_app(db_path: str) -> Starlette:
    async def homepage(request):
        from starlette.responses import JSONResponse

        async with connect(db_path) as conn:
            rows = await conn.fetch_all("SELECT id, name FROM items")
            return JSONResponse({"items": [list(r) for r in rows]})

    return Starlette(routes=[Route("/", homepage)])


@pytest.mark.asyncio
async def test_starlette_rapsqlite_smoke(test_db: str) -> None:
    async with connect(test_db) as conn:  # type: ignore[attr-defined]
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (id, name) VALUES (1, 'foo')")
    app = _make_app(test_db)
    with TestClient(app) as client:
        r = client.get("/")
    assert r.status_code == 200
    assert r.json() == {"items": [[1, "foo"]]}
