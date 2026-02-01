"""Minimal Starlette example with rapsqlite.

Run: pip install starlette "uvicorn[standard]" rapsqlite && \
     python -m uvicorn examples.starlette_db:app --reload

Then: GET http://127.0.0.1:8000/
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contextlib import asynccontextmanager

from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import JSONResponse
from starlette.routing import Route

from rapsqlite import connect

DB_PATH = str(Path(__file__).resolve().parent / "starlette_example.db")


@asynccontextmanager
async def lifespan(app: Starlette):
    async with connect(DB_PATH) as conn:  # type: ignore[attr-defined]
        await conn.execute(
            "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await conn.execute("INSERT OR IGNORE INTO items (id, name) VALUES (1, 'bar')")
    yield


async def homepage(request: Request) -> JSONResponse:
    async with connect(DB_PATH) as conn:  # type: ignore[attr-defined]
        rows = await conn.fetch_all("SELECT id, name FROM items")
        return JSONResponse({"items": [list(r) for r in rows]})


routes = [Route("/", homepage)]
app = Starlette(routes=routes, lifespan=lifespan)
