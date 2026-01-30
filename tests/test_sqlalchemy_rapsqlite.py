"""Smoke tests for SQLAlchemy + rapsqlite (sqlite+rapsqlite dialect)."""

from __future__ import annotations

import pytest

pytest.importorskip("sqlalchemy")
import rapsqlite.sqlalchemy  # noqa: F401 -- register dialect before create_async_engine
from sqlalchemy import text
from sqlalchemy.ext.asyncio import create_async_engine


@pytest.mark.asyncio
async def test_sqlalchemy_engine_create():
    """create_async_engine(\"sqlite+rapsqlite:///:memory:\") builds an AsyncEngine."""
    engine = create_async_engine("sqlite+rapsqlite:///:memory:")
    assert engine is not None
    assert str(engine.url).startswith("sqlite+rapsqlite")
    await engine.dispose()


@pytest.mark.asyncio
async def test_sqlalchemy_alembic_style_migration(test_db: str) -> None:
    """Validate sqlite+rapsqlite for Alembic-style migrations (create table, add column)."""
    from rapsqlite import connect

    # Dialect already registered by module-level import
    url = f"sqlite+rapsqlite:///{test_db}"
    engine = create_async_engine(url)
    try:
        async with engine.begin() as conn:
            await conn.execute(
                text("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            )
            await conn.execute(text("INSERT INTO users (id, name) VALUES (1, 'alice')"))
        async with engine.begin() as conn:
            await conn.execute(text("ALTER TABLE users ADD COLUMN email TEXT"))
            await conn.execute(text("UPDATE users SET email = 'a@b.com' WHERE id = 1"))
        await engine.dispose()
    finally:
        await engine.dispose()

    async with connect(test_db) as conn:
        row = await conn.fetch_one("SELECT id, name, email FROM users WHERE id = 1")
    assert row is not None
    assert row[0] == 1 and row[1] == "alice" and row[2] == "a@b.com"
