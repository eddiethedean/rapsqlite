"""Smoke test for FastAPI + rapsqlite integration pattern."""

from typing import Any, AsyncIterator

import pytest

pytest.importorskip("fastapi")

from fastapi import Depends, FastAPI
from fastapi.testclient import TestClient

from rapsqlite import connect


def _make_app(db_path: str) -> FastAPI:
    app = FastAPI()

    async def get_db() -> AsyncIterator[Any]:
        async with connect(db_path) as conn:
            yield conn

    @app.get("/")
    async def root(db: Any = Depends(get_db)) -> dict:
        rows = await db.fetch_all("SELECT id, name FROM items")
        return {"items": [list(r) for r in rows]}

    return app


@pytest.mark.asyncio
async def test_fastapi_rapsqlite_smoke(test_db: str) -> None:
    async with connect(test_db) as conn:
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (id, name) VALUES (1, 'foo')")
    app = _make_app(test_db)
    with TestClient(app) as client:
        r = client.get("/")
    assert r.status_code == 200
    assert r.json() == {"items": [[1, "foo"]]}
