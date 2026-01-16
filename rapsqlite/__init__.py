"""True async SQLite — no fake async, no GIL stalls.

rapsqlite provides true async SQLite operations for Python, backed by Rust,
Tokio, and sqlx. Unlike libraries that wrap blocking database calls in async
syntax, rapsqlite guarantees that all database operations execute outside the
Python GIL, ensuring event loops never stall under load.

Example:
    Basic usage::

        import asyncio
        from rapsqlite import Connection

        async def main():
            async with Connection("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                await conn.execute("INSERT INTO test (value) VALUES ('hello')")
                rows = await conn.fetch_all("SELECT * FROM test")
                print(rows)

        asyncio.run(main())

    Using the connect() function (aiosqlite-compatible)::

        import asyncio
        from rapsqlite import connect

        async def main():
            async with connect("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                rows = await conn.fetch_all("SELECT * FROM test")
                print(rows)

        asyncio.run(main())

    Transactions::

        async with Connection("example.db") as conn:
            await conn.begin()
            try:
                await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
                await conn.commit()
            except Exception:
                await conn.rollback()
"""

from typing import Any, List

try:
    from _rapsqlite import (
        Connection,
        Cursor,
        Error,
        Warning,
        DatabaseError,
        OperationalError,
        ProgrammingError,
        IntegrityError,
    )  # type: ignore[import-not-found]
except ImportError:
    try:
        from rapsqlite._rapsqlite import (
            Connection,
            Cursor,
            Error,
            Warning,
            DatabaseError,
            OperationalError,
            ProgrammingError,
            IntegrityError,
        )
    except ImportError:
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        )

__version__: str = "0.1.1"
__all__: List[str] = [
    "Connection",
    "Cursor",
    "connect",
    "Error",
    "Warning",
    "DatabaseError",
    "OperationalError",
    "ProgrammingError",
    "IntegrityError",
]


def connect(path: str, **kwargs: Any) -> Connection:
    """Connect to a SQLite database.

    This function matches the aiosqlite.connect() API for compatibility,
    allowing seamless migration from aiosqlite to rapsqlite.

    Args:
        path: Path to the SQLite database file. Can be ":memory:" for an
            in-memory database, or a file path.
        **kwargs: Additional arguments (currently ignored, reserved for future use)

    Returns:
        Connection: An async SQLite connection object that can be used as an
            async context manager.

    Raises:
        ValueError: If the database path is invalid (empty or contains null bytes)
        OperationalError: If the database connection cannot be established

    Example:
        Basic usage::

            async with connect("example.db") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
                await conn.execute("INSERT INTO test DEFAULT VALUES")
                rows = await conn.fetch_all("SELECT * FROM test")

        In-memory database::

            async with connect(":memory:") as conn:
                await conn.execute("CREATE TABLE test (id INTEGER)")
                # Database exists only for the duration of the connection

    Note:
        The connection object supports async context manager protocol. It's
        recommended to use `async with` to ensure proper resource cleanup.
    """
    return Connection(path)
