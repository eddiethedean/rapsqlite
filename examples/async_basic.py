"""Minimal async rapsqlite example (no extra deps).

Run: python examples/async_basic.py
"""

import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from rapsqlite import connect


async def main() -> None:
    async with connect(":memory:") as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT)")
        await db.execute("INSERT INTO t (x) VALUES (?)", ["hello"])
        rows = await db.fetch_all("SELECT * FROM t")
    print("rows:", rows)


if __name__ == "__main__":
    asyncio.run(main())
