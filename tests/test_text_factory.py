import pytest

import rapsqlite


@pytest.mark.asyncio
async def test_text_factory_callable_applies_to_text_columns(test_db):
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (v TEXT, b BLOB)")
        await db.execute("INSERT INTO t (v, b) VALUES (?, ?)", ["hello", b"bytes"])

        seen = {"arg_type": None}

        def tf(raw: bytes):
            seen["arg_type"] = type(raw)
            return raw.decode("utf-8").upper()

        db.text_factory = tf

        row = await db.fetch_one("SELECT v, b FROM t")
        assert row[0] == "HELLO"
        assert row[1] == b"bytes"  # BLOB should not be routed through text_factory
        assert seen["arg_type"] is bytes


@pytest.mark.asyncio
async def test_text_factory_none_uses_default_utf8(test_db):
    async with rapsqlite.connect(test_db) as db:
        await db.execute("CREATE TABLE t (v TEXT)")
        await db.execute("INSERT INTO t (v) VALUES (?)", ["hello"])

        db.text_factory = None
        row = await db.fetch_one("SELECT v FROM t")
        assert row[0] == "hello"
