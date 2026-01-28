"""Property-based tests using Hypothesis for rapsqlite.

Tests invariants and properties that should always hold.
"""

import pytest
from hypothesis import given, strategies as st, settings, assume, HealthCheck

from rapsqlite import connect


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=50,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(
    value=st.one_of(
        st.integers(
            min_value=-(2**63), max_value=2**63 - 1
        ),  # Limit to SQLite INTEGER range
        st.floats(allow_nan=False, allow_infinity=False),
        st.text(),
        st.binary(),
    )
)
async def test_parameter_round_trip(test_db, value):
    """Test that parameter values survive round-trip (insert → select)."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, value BLOB)"
        )

        # Insert value
        await db.execute("INSERT INTO t (value) VALUES (?)", [value])

        # Retrieve value
        rows = await db.fetch_all("SELECT value FROM t ORDER BY id DESC LIMIT 1")

        retrieved = rows[0][0]

        if isinstance(value, bytes):
            assert retrieved == value
        elif isinstance(value, str):
            assert retrieved == value
        elif isinstance(value, int):
            # Integers within SQLite INTEGER range should be preserved exactly
            assert retrieved == value
        elif isinstance(value, float):
            # Allow some float precision differences
            assert abs(retrieved - value) < 1e-10


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=30,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(
    values=st.lists(
        st.one_of(
            st.integers(
                min_value=-(2**63), max_value=2**63 - 1
            ),  # Limit to SQLite INTEGER range
            st.text(max_size=100),
        ),
        min_size=1,
        max_size=10,
    )
)
async def test_multiple_parameters_round_trip(test_db, values):
    """Test that multiple parameters survive round-trip."""
    async with connect(test_db) as db:
        # Drop and create table with correct columns (handle table schema changes)
        await db.execute("DROP TABLE IF EXISTS t")
        columns = ", ".join([f"c{i} TEXT" for i in range(len(values))])
        await db.execute(f"CREATE TABLE t (id INTEGER PRIMARY KEY, {columns})")

        # Build insert query
        placeholders = ", ".join(["?" for _ in values])
        await db.execute(
            f"INSERT INTO t ({', '.join([f'c{i}' for i in range(len(values))])}) VALUES ({placeholders})",
            values,
        )

        # Retrieve
        rows = await db.fetch_all("SELECT * FROM t ORDER BY id DESC LIMIT 1")
        retrieved = list(rows[0][1:])  # Skip id column

        # Compare (handle type conversions)
        assert len(retrieved) == len(values)
        for r, v in zip(retrieved, values):
            if isinstance(v, int):
                # Integers might be stored as text for very large values
                # Compare as integers if possible, otherwise as strings
                try:
                    assert int(r) == v or str(r) == str(v)
                except (ValueError, TypeError):
                    assert str(r) == str(v)
            else:
                # For other types, compare as strings
                assert str(r) == str(v)


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=20,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(
    table_name=st.text(
        alphabet=st.characters(min_codepoint=97, max_codepoint=122),
        min_size=1,
        max_size=20,
    ),
    count=st.integers(min_value=1, max_value=100),
)
async def test_transaction_atomicity(test_db, table_name, count):
    """Test that transactions are atomic - all or nothing."""
    # Avoid SQL keywords and ensure valid table name
    sql_keywords = {
        "as",
        "select",
        "from",
        "where",
        "insert",
        "update",
        "delete",
        "create",
        "table",
        "drop",
    }
    assume(" " not in table_name)  # Avoid spaces in table names
    assume(table_name.isalnum())  # Only alphanumeric
    assume(table_name.lower() not in sql_keywords)  # Avoid SQL keywords

    async with connect(test_db) as db:
        await db.execute(
            f"CREATE TABLE IF NOT EXISTS {table_name} (id INTEGER PRIMARY KEY, value INTEGER)"
        )

        # Count before
        rows_before = await db.fetch_all(f"SELECT COUNT(*) FROM {table_name}")
        count_before = rows_before[0][0] if rows_before else 0

        # Start transaction
        await db.begin()
        try:
            # Insert rows
            for i in range(count):
                await db.execute(f"INSERT INTO {table_name} (value) VALUES (?)", [i])

            # Rollback
            await db.rollback()
        except Exception:
            await db.rollback()
            raise

        # Count after - should be same as before
        rows_after = await db.fetch_all(f"SELECT COUNT(*) FROM {table_name}")
        count_after = rows_after[0][0] if rows_after else 0
        assert count_after == count_before


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=20,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(
    pool_size=st.integers(min_value=1, max_value=10),
    num_operations=st.integers(min_value=1, max_value=20),
)
async def test_pool_size_invariant(test_db, pool_size, num_operations):
    """Test that pool size invariant is maintained."""
    async with connect(test_db) as db:
        db.pool_size = pool_size
        await db.execute(
            "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, value INTEGER)"
        )

        # Perform operations
        for i in range(num_operations):
            await db.execute("INSERT INTO t (value) VALUES (?)", [i])

        # Pool size should still be set
        assert db.pool_size == pool_size


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=30,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(text_value=st.text(max_size=1000))
async def test_text_round_trip(test_db, text_value):
    """Test that text values survive round-trip."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, value TEXT)"
        )

        await db.execute("INSERT INTO t (value) VALUES (?)", [text_value])
        rows = await db.fetch_all("SELECT value FROM t ORDER BY id DESC LIMIT 1")

        assert rows[0][0] == text_value


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=20,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(int_value=st.integers(min_value=-(2**63), max_value=2**63 - 1))
async def test_integer_round_trip(test_db, int_value):
    """Test that integer values survive round-trip."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, value INTEGER)"
        )

        await db.execute("INSERT INTO t (value) VALUES (?)", [int_value])
        rows = await db.fetch_all("SELECT value FROM t ORDER BY id DESC LIMIT 1")

        assert rows[0][0] == int_value


@pytest.mark.property
@pytest.mark.asyncio
@settings(
    max_examples=20,
    deadline=5000,
    suppress_health_check=[HealthCheck.function_scoped_fixture],
)
@given(blob_value=st.binary(max_size=10000))
async def test_blob_round_trip(test_db, blob_value):
    """Test that BLOB values survive round-trip."""
    async with connect(test_db) as db:
        await db.execute(
            "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, value BLOB)"
        )

        await db.execute("INSERT INTO t (value) VALUES (?)", [blob_value])
        rows = await db.fetch_all("SELECT value FROM t ORDER BY id DESC LIMIT 1")

        assert rows[0][0] == blob_value
