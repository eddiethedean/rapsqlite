import os
import subprocess
import sys
import tempfile

import pytest


pytestmark = [pytest.mark.unit]


def _run_in_subprocess(code: str) -> subprocess.CompletedProcess[bytes]:
    env = os.environ.copy()
    # Ensure repo root is importable for the child process (tests run from repo root).
    env["PYTHONPATH"] = os.getcwd() + (
        os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else ""
    )
    env.setdefault("PYTHONFAULTHANDLER", "1")
    env.setdefault("RUST_BACKTRACE", "1")
    return subprocess.run(
        [sys.executable, "-X", "faulthandler", "-c", code],
        env=env,
        capture_output=True,
        timeout=60,
        check=False,
    )


def test_create_aggregate_subprocess_smoke():
    # If create_aggregate triggers a native crash, we want a non-zero return code.
    # On Windows, NamedTemporaryFile keeps the file handle open, which prevents SQLite
    # from opening/creating the database file (error code 14).
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "test.db")
        code = f"""
import asyncio
from rapsqlite import connect
import sys

class SumAggregate:
    def __init__(self):
        self.total = 0
    def step(self, x):
        if x is not None:
            self.total += int(x)
    def finalize(self):
        return self.total

async def main():
    open({path!r}, "ab").close()
    async with connect({path!r}) as db:
        print("connected", file=sys.stderr, flush=True)
        await db.execute("CREATE TABLE t (x INT)")
        print("table", file=sys.stderr, flush=True)
        await db.executemany("INSERT INTO t VALUES (?)", [[1],[2],[3]])
        print("inserted", file=sys.stderr, flush=True)
        await db.create_aggregate("mysum", 1, SumAggregate)
        print("aggregate-set", file=sys.stderr, flush=True)
        row = await db.fetch_one("SELECT mysum(x) FROM t")
        print("selected", file=sys.stderr, flush=True)
        assert row[0] == 6
        # Intentionally do not clear here: historically, callback cleanup has been a crash vector.
        # The subprocess boundary ensures native crashes are surfaced in CI.

asyncio.run(main())
"""
        p = _run_in_subprocess(code)
        assert p.returncode == 0, p.stderr.decode("utf-8", "replace")


def test_create_collation_subprocess_smoke():
    # If create_collation triggers a native crash, we want a non-zero return code.
    # On Windows, NamedTemporaryFile keeps the file handle open, which prevents SQLite
    # from opening/creating the database file (error code 14).
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "test.db")
        code = f"""
import asyncio
from rapsqlite import connect
import sys

def reverse_collation(s1: str, s2: str) -> int:
    if s1 == s2:
        return 0
    return -1 if s1 > s2 else 1

async def main():
    open({path!r}, "ab").close()
    async with connect({path!r}) as db:
        print("connected", file=sys.stderr, flush=True)
        await db.create_collation("reverse", reverse_collation)
        print("collation-set", file=sys.stderr, flush=True)
        await db.execute("CREATE TABLE t (name TEXT)")
        print("table", file=sys.stderr, flush=True)
        await db.executemany("INSERT INTO t VALUES (?)", [["alpha"],["beta"],["gamma"]])
        print("inserted", file=sys.stderr, flush=True)
        rows = await db.fetch_all("SELECT name FROM t ORDER BY name COLLATE reverse")
        print("selected", file=sys.stderr, flush=True)
        assert [r[0] for r in rows] == ["gamma", "beta", "alpha"]
        # Intentionally do not clear here: historically, callback cleanup has been a crash vector.
        # The subprocess boundary ensures native crashes are surfaced in CI.

asyncio.run(main())
"""
        p = _run_in_subprocess(code)
        assert p.returncode == 0, p.stderr.decode("utf-8", "replace")
