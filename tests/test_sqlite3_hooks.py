# Ported from CPython Lib/test/test_sqlite3/test_hooks.py.
# Converted to async pytest for rapsqlite.

"""Tests for rapsqlite authorizer and progress handler (ported from CPython test_sqlite3)."""

import pytest

from rapsqlite import connect


@pytest.mark.asyncio
async def test_set_authorizer_allow_and_invoked(test_db, unique_table_prefix):
    """set_authorizer callback is invoked; allow all then clear."""
    async with connect(test_db) as db:
        calls = []

        def authorizer(action, arg1, arg2, arg3, arg4):
            calls.append(action)
            return 0  # SQLITE_OK

        await db.set_authorizer(authorizer)
        await db.execute(f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY)")
        await db.execute(f"INSERT INTO {unique_table_prefix} (id) VALUES (1)")
        assert len(calls) > 0
        await db.set_authorizer(None)


@pytest.mark.asyncio
async def test_set_progress_handler_invoked(test_db, unique_table_prefix):
    """set_progress_handler callback is invoked during long operations."""
    async with connect(test_db) as db:
        await db.execute(
            f"CREATE TABLE {unique_table_prefix} (id INTEGER PRIMARY KEY, x INTEGER)"
        )
        cur = db.cursor()
        await cur.executemany(
            f"INSERT INTO {unique_table_prefix} (x) VALUES (?)",
            [[i] for i in range(200)],
        )
        await cur.close()
        progress_calls = []

        def progress():
            progress_calls.append(1)
            return 0

        await db.set_progress_handler(progress, 10)
        rows = await db.fetch_all(f"SELECT SUM(x) FROM {unique_table_prefix}")
        assert len(rows) == 1
        assert len(progress_calls) >= 1
        await db.set_progress_handler(10, None)
