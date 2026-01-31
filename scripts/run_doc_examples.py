"""Run the code examples from the documentation to verify they execute correctly.

These examples mirror the snippets in docs/quickstart.rst, docs/installation.rst,
docs/index.rst, and docs/api-reference/row.rst. Use temp files for isolation
(rapsqlite may share :memory: across connections). Exit 0 only if all run successfully.
"""

import asyncio
import os
import sys
import tempfile
from pathlib import Path

# Allow importing rapsqlite from project root when run as script
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from rapsqlite import connect, Row


def _temp_db():
    """Return a path to a temporary database; caller must unlink when done."""
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


async def quickstart_basic():
    """Docs quickstart: Basic Connection (output: [[1, 'Alice']])."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
            rows = await conn.fetch_all("SELECT * FROM users")
            assert rows == [[1, "Alice"]], f"Expected [[1, 'Alice']], got {rows}"
    finally:
        os.unlink(path)


async def installation_verify():
    """Docs installation: Verifying Installation (output: [[1]])."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER)")
            await conn.execute("INSERT INTO test VALUES (1)")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert rows == [[1]], f"Expected [[1]], got {rows}"
    finally:
        os.unlink(path)


async def index_quick_example():
    """Docs index: Quick Example (output: [[1, 'hello']])."""
    path = _temp_db()
    try:
        async with connect(path) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await conn.fetch_all("SELECT * FROM test")
            assert rows == [[1, "hello"]], f"Expected [[1, 'hello']], got {rows}"
    finally:
        os.unlink(path)


async def quickstart_cursor_iteration():
    """Docs quickstart: Using Cursors / async for (output: [1,'Alice'], [2,'Bob'])."""
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
            assert collected == [[1, "Alice"], [2, "Bob"]], f"Got {collected}"
    finally:
        os.unlink(path)


async def row_api_example():
    """Docs api-reference/row.rst: Row access (name, index, keys)."""
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
        os.unlink(path)


async def migration_basic_connection():
    """Docs migration-guide: Basic Connection (output: [[1, 'hello']])."""
    import rapsqlite as aiosqlite
    path = _temp_db()
    try:
        async with aiosqlite.connect(path) as db:
            await db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await db.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await db.fetch_all("SELECT * FROM test")
            assert rows == [[1, "hello"]], f"Expected [[1, 'hello']], got {rows}"
    finally:
        os.unlink(path)


async def run_all():
    """Run all doc examples."""
    await quickstart_basic()
    await installation_verify()
    await index_quick_example()
    await quickstart_cursor_iteration()
    await row_api_example()
    await migration_basic_connection()


def main():
    asyncio.run(run_all())
    print("All doc examples ran successfully.")


if __name__ == "__main__":
    main()
