import pytest

from rapsqlite import connect


pytestmark = [pytest.mark.unit, pytest.mark.asyncio]


async def test_executescript_ignores_semicolons_in_strings(test_db) -> None:
    async with connect(test_db) as conn:
        await conn.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, body TEXT)")

        script = """
        INSERT INTO messages (body) VALUES ('hello;world');
        INSERT INTO messages (body) VALUES ("foo;bar");
        """

        cursor = conn.cursor()
        await cursor.executescript(script)

        rows = await conn.fetch_all("SELECT body FROM messages ORDER BY id")
        assert [row[0] for row in rows] == ["hello;world", "foo;bar"]
