"""Script to verify and update code examples in documentation.

This script runs all code examples from README.md and captures their real outputs.
"""

import asyncio
import tempfile
import os
import sys
from pathlib import Path

# Add parent directory to path to import rapsqlite
sys.path.insert(0, str(Path(__file__).parent.parent))

from rapsqlite import Connection, connect, IntegrityError


async def example_basic_usage():
    """Run basic usage example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name

    try:
        async with Connection(db_path) as conn:
            await conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
            )
            await conn.execute(
                "INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com')"
            )
            await conn.execute(
                "INSERT INTO users (name, email) VALUES ('Bob', 'bob@example.com')"
            )

            rows = await conn.fetch_all("SELECT * FROM users")
            print("rows =", rows)

            user = await conn.fetch_one("SELECT * FROM users WHERE name = 'Alice'")
            print("user =", user)

            user = await conn.fetch_optional(
                "SELECT * FROM users WHERE name = 'Charlie'"
            )
            print("user =", user)
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_connect():
    """Run connect() example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with connect(db_path) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            await conn.execute("INSERT INTO test (value) VALUES ('hello')")
            rows = await conn.fetch_all("SELECT * FROM test")
            print("rows =", rows)
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_transactions():
    """Run transactions example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with Connection(db_path) as conn:
            await conn.execute(
                "CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER)"
            )
            await conn.execute("INSERT INTO accounts (balance) VALUES (1000)")

            await conn.begin()
            try:
                await conn.execute(
                    "UPDATE accounts SET balance = balance - 100 WHERE id = 1"
                )
                await conn.execute(
                    "UPDATE accounts SET balance = balance + 100 WHERE id = 2"
                )
                await conn.commit()
                print("Transaction committed")
            except Exception:
                await conn.rollback()
                print("Transaction rolled back")
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_cursors():
    """Run cursors example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with Connection(db_path) as conn:
            await conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)")
            await conn.execute("INSERT INTO items (name) VALUES ('item1')")
            await conn.execute("INSERT INTO items (name) VALUES ('item2')")

            cursor = conn.cursor()
            await cursor.execute("SELECT * FROM items")

            row = await cursor.fetchone()
            print("row =", row)

            rows = await cursor.fetchall()
            print("rows =", rows)

            await cursor.execute("SELECT * FROM items")
            rows = await cursor.fetchmany(1)
            print("rows =", rows)
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_concurrent():
    """Run concurrent operations example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with Connection(db_path) as conn:
            await conn.execute(
                "CREATE TABLE data (id INTEGER PRIMARY KEY, value INTEGER)"
            )

            tasks = [
                conn.execute(f"INSERT INTO data (value) VALUES ({i})")
                for i in range(100)
            ]
            await asyncio.gather(*tasks)

            rows = await conn.fetch_all("SELECT * FROM data")
            print(f"Inserted {len(rows)} rows")
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_error_handling():
    """Run error handling example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with Connection(db_path) as conn:
            await conn.execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE)"
            )
            await conn.execute("INSERT INTO users (email) VALUES ('alice@example.com')")

            try:
                await conn.execute(
                    "INSERT INTO users (email) VALUES ('alice@example.com')"
                )
            except IntegrityError as e:
                print(f"Integrity constraint violation: {e}")
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_backup():
    """Run backup example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        source_path = f.name
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        backup_path = f.name
    try:
        async with Connection(source_path) as source:
            await source.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)"
            )
            await source.execute("INSERT INTO test (name) VALUES ('Alice')")
            await source.execute("INSERT INTO test (name) VALUES ('Bob')")

            async with Connection(backup_path) as target:
                await source.backup(target)

                rows = await target.fetch_all("SELECT * FROM test")
                print("rows =", rows)
    finally:
        for path in [source_path, backup_path]:
            if os.path.exists(path):
                os.unlink(path)


async def example_schema():
    """Run schema operations example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        async with Connection(db_path) as conn:
            await conn.execute("""
                CREATE TABLE users (
                    id INTEGER PRIMARY KEY,
                    email TEXT UNIQUE NOT NULL
                )
            """)
            await conn.execute("""
                CREATE TABLE posts (
                    id INTEGER PRIMARY KEY,
                    user_id INTEGER,
                    title TEXT,
                    FOREIGN KEY (user_id) REFERENCES users(id)
                )
            """)
            await conn.execute("CREATE INDEX idx_posts_user ON posts(user_id)")

            tables = await conn.get_tables()
            print("tables =", tables)

            columns = await conn.get_table_info("users")
            print("columns =", columns)

            indexes = await conn.get_indexes(table_name="posts")
            print("indexes =", indexes)

            foreign_keys = await conn.get_foreign_keys("posts")
            print("foreign_keys =", foreign_keys)

            schema = await conn.get_schema(table_name="posts")
            print("schema keys =", list(schema.keys()))
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def example_init_hook():
    """Run init_hook example."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:

        async def init_hook(conn):
            await conn.execute("""
                CREATE TABLE IF NOT EXISTS users (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    email TEXT UNIQUE
                )
            """)
            await conn.execute(
                "INSERT OR IGNORE INTO users (name, email) VALUES ('Admin', 'admin@example.com')"
            )
            await conn.set_pragma("foreign_keys", True)

        async with Connection(db_path, init_hook=init_hook) as conn:
            users = await conn.fetch_all("SELECT * FROM users")
            print("users =", users)
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


async def main():
    """Run all examples."""
    print("=" * 60)
    print("Example: Basic Usage")
    print("=" * 60)
    await example_basic_usage()
    print()

    print("=" * 60)
    print("Example: connect() Function")
    print("=" * 60)
    await example_connect()
    print()

    print("=" * 60)
    print("Example: Transactions")
    print("=" * 60)
    await example_transactions()
    print()

    print("=" * 60)
    print("Example: Cursors")
    print("=" * 60)
    await example_cursors()
    print()

    print("=" * 60)
    print("Example: Concurrent Operations")
    print("=" * 60)
    await example_concurrent()
    print()

    print("=" * 60)
    print("Example: Error Handling")
    print("=" * 60)
    await example_error_handling()
    print()

    print("=" * 60)
    print("Example: Backup")
    print("=" * 60)
    await example_backup()
    print()

    print("=" * 60)
    print("Example: Schema Operations")
    print("=" * 60)
    await example_schema()
    print()

    print("=" * 60)
    print("Example: Init Hook")
    print("=" * 60)
    await example_init_hook()
    print()


if __name__ == "__main__":
    asyncio.run(main())
