#!/usr/bin/env python3
"""Minimal SQL logic test runner for rapsqlite.

Runs simple-format test files (SQL statements, then "----", then expected rows)
against rapsqlite :memory: and compares results. Format:
  - Lines before "----" are SQL (one or more statements, semicolon-separated).
  - Lines after "----" are expected result rows (tab-separated values).
  - The last statement must be a SELECT; its result is compared to expected rows.

Usage:
  python scripts/run_sql_logic_tests.py [path_to_test.sql]
  If no path given, runs scripts/sql_logic_tests/*.sql
"""

import asyncio
import sys
from pathlib import Path

# Add project root for rapsqlite import
ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def parse_test_file(path: Path) -> tuple[list[str], list[list[str]]]:
    """Parse a simple-format test file. Returns (statements, expected_rows)."""
    lines = path.read_text().splitlines()
    # Separator is a line that is exactly "----" (avoids matching "----" in comments)
    try:
        sep_idx = next(i for i, line in enumerate(lines) if line.strip() == "----")
    except StopIteration:
        return [], []
    sql_lines = [line for line in lines[:sep_idx] if not line.strip().startswith("#")]
    sql_block = "\n".join(sql_lines)
    statements = [s.strip() for s in sql_block.split(";") if s.strip()]
    if not statements and sql_block.strip():
        stmt = sql_block.strip()
        if stmt:
            statements = [stmt]
    expected_lines = lines[sep_idx + 1 :]
    expected_rows = []
    for line in expected_lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        expected_rows.append([c.strip() for c in line.split("\t")])
    return statements, expected_rows


async def run_test(path: Path) -> bool:
    """Run one test file; return True if passed."""
    from rapsqlite import connect

    statements, expected_rows = parse_test_file(path)
    if not statements:
        print(f"  {path.name}: no statements (skip)")
        return True
    async with connect(":memory:") as db:  # type: ignore[attr-defined]
        for stmt in statements[:-1]:
            await db.execute(stmt)
        last = statements[-1]
        rows = await db.fetch_all(last)
    # Normalize to list of list of strings for comparison
    actual = [[str(c) for c in row] for row in rows]
    if actual != expected_rows:
        print(f"  {path.name}: FAILED")
        print(f"    expected: {expected_rows}")
        print(f"    actual:   {actual}")
        return False
    print(f"  {path.name}: ok")
    return True


async def main() -> int:
    if len(sys.argv) > 1:
        paths = [Path(p) for p in sys.argv[1:]]
    else:
        base = ROOT / "scripts" / "sql_logic_tests"
        if not base.exists():
            print("No scripts/sql_logic_tests directory; nothing to run.")
            return 0
        paths = sorted(base.glob("*.sql"))
    if not paths:
        print("No test files found.")
        return 0
    print("Running SQL logic tests (rapsqlite :memory:)...")
    results = await asyncio.gather(*[run_test(p) for p in paths])
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
