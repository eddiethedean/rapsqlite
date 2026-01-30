"""Minimal FastAPI example with a rapsqlite connection dependency.

Run: pip install "fastapi[standard]" "uvicorn[standard]" && \
     python -m uvicorn examples.fastapi_db:app --reload

Then: GET http://127.0.0.1:8000/
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contextlib import asynccontextmanager
from typing import Any, AsyncIterator

from fastapi import FastAPI, Depends
from rapsqlite import connect

DB_PATH = str(Path(__file__).resolve().parent / "fastapi_example.db")


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    async with connect(DB_PATH) as conn:
        await conn.execute(
            "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await conn.execute("INSERT OR IGNORE INTO items (id, name) VALUES (1, 'foo')")
    yield


async def get_db() -> AsyncIterator[Any]:
    async with connect(DB_PATH) as conn:
        yield conn  # FastAPI injects the yielded connection into route params


app = FastAPI(lifespan=lifespan)


@app.get("/")
async def root(db: Any = Depends(get_db)) -> dict:
    rows = await db.fetch_all("SELECT id, name FROM items")
    return {"items": [list(r) for r in rows]}
