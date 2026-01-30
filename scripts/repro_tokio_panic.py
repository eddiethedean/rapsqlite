"""Minimal repro for Tokio panic: Connection dropped without close() then GC.

Run: python scripts/repro_tokio_panic.py

Expected (when panic occurs): PytestUnraisableExceptionWarning or panic
  "this functionality requires a Tokio context" during gc.collect().
"""

import asyncio
import gc
import tempfile
import os

# Ensure we import rapsqlite (extension must be built)
from rapsqlite import Connection


def cleanup_db(path: str) -> None:
    if os.path.exists(path):
        try:
            os.unlink(path)
        except OSError:
            pass


async def main():
    fd, db_path = tempfile.mkstemp(suffix=".db", prefix="repro_tokio_")
    os.close(fd)
    try:
        # Create connection, use it once (creates pool + acquires connection), do NOT close
        conn = Connection(db_path)
        await conn.execute("CREATE TABLE t (id INTEGER)")
        await conn.execute("INSERT INTO t VALUES (1)")
        # Do not call await conn.close() and do not use async with
        # Drop our reference so Connection can be collected
        del conn
        # Force GC so PyO3 drops the Rust Connection -> drops pool/PoolConnection
        gc.collect()
        print("gc.collect() completed (panic may have occurred in another thread/warning)")
    finally:
        cleanup_db(db_path)


if __name__ == "__main__":
    asyncio.run(main())
