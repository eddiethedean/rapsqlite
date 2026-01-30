"""Smoke tests for SQLAlchemy + rapsqlite (sqlite+rapsqlite dialect)."""

from __future__ import annotations

import pytest

pytest.importorskip("sqlalchemy")
from sqlalchemy import text
from sqlalchemy.ext.asyncio import create_async_engine


@pytest.mark.asyncio
async def test_sqlalchemy_engine_create():
    """create_async_engine(\"sqlite+rapsqlite:///:memory:\") builds an AsyncEngine."""
    import rapsqlite.sqlalchemy  # register dialect

    engine = create_async_engine("sqlite+rapsqlite:///:memory:")
    assert engine is not None
    assert str(engine.url).startswith("sqlite+rapsqlite")
    await engine.dispose()
