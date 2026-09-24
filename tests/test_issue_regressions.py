"""Regression coverage for verified GitHub issue reproductions."""

import pytest

from rapsqlite import connect

pytestmark = [pytest.mark.unit, pytest.mark.asyncio]


async def test_unicode_named_parameter_uses_utf8_byte_offsets(test_db):
    async with connect(test_db) as db:
        assert await db.fetch_all("SELECT :é", {"é": 17}) == [[17]]


async def test_named_parameter_parser_ignores_quoted_sql_and_comments(test_db):
    async with connect(test_db) as db:
        rows = await db.fetch_all(
            "SELECT ':ghost' AS literal, :value AS value -- :comment\n",
            {"value": 1},
        )
    assert rows == [[":ghost", 1]]


async def test_include_query_in_errors_false_hides_failed_sql(test_db):
    async with connect(test_db) as db:
        db.include_query_in_errors = False
        with pytest.raises(Exception) as exc_info:
            await db.fetch_all("SELECT 'TOPSECRET_LITERAL' FROM missing_table")
    assert "TOPSECRET_LITERAL" not in str(exc_info.value)


async def test_sanitizing_multiple_sensitive_literals_does_not_panic(test_db):
    async with connect(test_db) as db:
        with pytest.raises(Exception) as exc_info:
            await db.fetch_all(
                "SELECT * FROM missing_table "
                "WHERE password='long-secret-value' AND token='other-secret'"
            )
    message = str(exc_info.value)
    assert "long-secret-value" not in message
    assert "other-secret" not in message
    assert "RustPanic" not in type(exc_info.value).__name__


async def test_leading_sql_comments_do_not_hide_select_results(test_db):
    async with connect(test_db) as db:
        cursor = await db.execute("-- leading comment\nSELECT 42 AS answer")
        assert await cursor.fetchall() == [[42]]
