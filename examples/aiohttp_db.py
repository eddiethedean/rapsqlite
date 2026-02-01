"""Minimal aiohttp example with rapsqlite.

Run: pip install aiohttp rapsqlite && python examples/aiohttp_db.py

Then: GET http://127.0.0.1:8080/
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from aiohttp import web
from rapsqlite import connect

DB_PATH = str(Path(__file__).resolve().parent / "aiohttp_example.db")


async def init_db(app: web.Application):
    """Initialize database schema on startup."""
    async with connect(DB_PATH) as conn:  # type: ignore[attr-defined]
        await conn.execute(
            "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await conn.execute("INSERT OR IGNORE INTO items (id, name) VALUES (1, 'baz')")


async def homepage(request: web.Request) -> web.Response:
    async with connect(DB_PATH) as conn:  # type: ignore[attr-defined]
        rows = await conn.fetch_all("SELECT id, name FROM items")
        return web.json_response({"items": [list(r) for r in rows]})


def create_app() -> web.Application:
    app = web.Application()
    app.on_startup.append(lambda app: init_db(app))
    app.router.add_get("/", homepage)
    return app


if __name__ == "__main__":
    app = create_app()
    web.run_app(app, host="127.0.0.1", port=8080)
