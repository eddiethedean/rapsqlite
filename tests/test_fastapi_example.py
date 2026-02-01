"""Smoke test for FastAPI + rapsqlite integration pattern."""

import sys
from typing import Any, AsyncIterator

import pytest

if sys.version_info >= (3, 14):
    pytest.skip(
        "FastAPI/Pydantic not yet compatible with Python 3.14",
        allow_module_level=True,
    )
pytest.importorskip("fastapi")

from fastapi import Depends, FastAPI
from fastapi.testclient import TestClient

from rapsqlite import connect

pytestmark = [pytest.mark.integration]


def _make_app(db_path: str) -> FastAPI:
    app = FastAPI()

    async def get_db() -> AsyncIterator[Any]:
        async with connect(db_path) as conn:  # type: ignore[attr-defined]
            yield conn

    @app.get("/")
    async def root(db: Any = Depends(get_db)) -> dict:
        rows = await db.fetch_all("SELECT id, name FROM items")
        return {"items": [list(r) for r in rows]}

    return app


@pytest.mark.asyncio
async def test_fastapi_rapsqlite_smoke(test_db: str) -> None:
    async with connect(test_db) as conn:  # type: ignore[attr-defined]
        await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO items (id, name) VALUES (1, 'foo')")
    app = _make_app(test_db)
    with TestClient(app) as client:
        r = client.get("/")
    assert r.status_code == 200
    assert r.json() == {"items": [[1, "foo"]]}
