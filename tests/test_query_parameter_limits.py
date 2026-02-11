import pytest

from rapsqlite import connect


pytestmark = [pytest.mark.unit, pytest.mark.asyncio]


async def test_execute_and_fetch_support_50_parameters(test_db) -> None:
    values = list(range(1, 51))

    async with connect(test_db) as conn:
        await conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")

        # Insert 50 rows using executemany with 1 parameter each.
        await conn.execute_many("INSERT INTO t (v) VALUES (?)", [[v] for v in values])

        # Build a SELECT with 50 bound parameters and ensure it succeeds.
        placeholders = ",".join("?" for _ in values)
        sql = f"SELECT v FROM t WHERE v IN ({placeholders}) ORDER BY v"

        rows = await conn.fetch_all(sql, values)
        assert [row[0] for row in rows] == values
