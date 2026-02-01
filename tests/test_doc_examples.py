"""Tests that run the documentation code examples to ensure they execute correctly.

These mirror docs/quickstart.rst, docs/installation.rst, docs/index.rst,
docs/api-reference/row.rst, and docs/guides/migration-guide.rst. Run with:
  pytest tests/test_doc_examples.py -v
Or run the script: python scripts/run_doc_examples.py
"""

import os
import tempfile

import pytest

from rapsqlite import Row, connect

from conftest import cleanup_db

pytestmark = [pytest.mark.integration]


def _temp_db():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


@pytest.mark.asyncio
async def test_quickstart_basic_connection():
    """Docs quickstart: Basic Connection - output [[1, 'Alice']]."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
            rows = await conn.fetch_all("SELECT * FROM users")
            assert rows == [[1, "Alice"]]
    finally:
        cleanup_db(path)


@pytest.mark.asyncio
async def test_installation_verify():
    """Docs installation: Verifying Installation - output [[1]]."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER)")
            await conn.execute("INSERT INTO test VALUES (1)")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert rows == [[1]]
    finally:
        cleanup_db(path)


@pytest.mark.asyncio
async def test_index_quick_example():
    """Docs index: Quick Example - output [[1, 'hello']]."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert rows == [[1, "hello"]]
    finally:
        cleanup_db(path)


@pytest.mark.asyncio
async def test_quickstart_cursor_iteration():
    """Docs quickstart: Using Cursors async for - output [1,'Alice'], [2,'Bob']."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
            await conn.execute("INSERT INTO users (name) VALUES ('Bob')")
            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM users")
            collected = []
            async for row in cursor:
                collected.append(row)
            assert collected == [[1, "Alice"], [2, "Bob"]]
    finally:
        cleanup_db(path)


@pytest.mark.asyncio
async def test_row_api_example():
    """Docs api-reference/row.rst: Row access by name, index, keys()."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
            conn.row_factory = Row
            rows = await conn.fetch_all("SELECT id, name FROM users")
            assert rows[0]["name"] == "Alice"
            assert rows[0][0] == 1
            assert list(rows[0].keys()) == ["id", "name"]
    finally:
        cleanup_db(path)


@pytest.mark.asyncio
async def test_migration_basic_connection():
    """Docs migration-guide: Basic Connection with rapsqlite as aiosqlite."""
    import rapsqlite as aiosqlite

    path = _temp_db()
    try:
        async with aiosqlite.connect(path) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await db.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await db.fetch_all("SELECT * FROM test")
            assert rows == [[1, "hello"]]
    finally:
        cleanup_db(path)
