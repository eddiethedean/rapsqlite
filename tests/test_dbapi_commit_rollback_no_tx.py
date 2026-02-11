import pytest

from rapsqlite import dbapi


pytestmark = [pytest.mark.unit, pytest.mark.asyncio]


async def test_dbapi_commit_rollback_no_transaction_is_noop(tmp_path) -> None:
    db_path = tmp_path / "test_dbapi_commit_rollback_no_tx.db"

    conn = await dbapi.connect(str(db_path))
    try:
        # No explicit transaction started; these should behave as no-ops,
        # not raising exceptions, matching aiosqlite/DB-API expectations.
        await conn.commit()
        await conn.rollback()
    finally:
        await conn.close()
