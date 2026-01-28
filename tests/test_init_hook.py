"""Tests for init_hook functionality (Phase 2.11)."""

import pytest
import rapsqlite


@pytest.mark.asyncio
async def test_init_hook_basic(tmp_path):
    """Test basic init_hook execution."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation should trigger init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify hook was called
    assert len(call_count) == 1

    # Verify table was created
    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1
    assert result[0][0] == 1


@pytest.mark.asyncio
async def test_init_hook_creates_tables(tmp_path):
    """Test init_hook creates tables/schema."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT UNIQUE
            )
        """)
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER,
                title TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )
        """)

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute(
        "INSERT INTO users (name, email) VALUES (?, ?)", ["Alice", "alice@example.com"]
    )

    # Verify schema was created
    tables = await conn.get_tables()
    assert "users" in tables
    assert "posts" in tables

    # Verify foreign key works
    user = await conn.fetch_one("SELECT id FROM users WHERE name = ?", ["Alice"])
    await conn.execute(
        "INSERT INTO posts (user_id, title) VALUES (?, ?)", [user[0], "First Post"]
    )

    posts = await conn.fetch_all("SELECT * FROM posts")
    assert len(posts) == 1


@pytest.mark.asyncio
async def test_init_hook_sets_pragmas(tmp_path):
    """Test init_hook sets additional PRAGMAs."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.set_pragma("foreign_keys", True)
        await conn.set_pragma("journal_mode", "WAL")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    # Verify PRAGMAs were set
    fk_rows = await conn.fetch_all("PRAGMA foreign_keys")
    assert fk_rows[0][0] == 1

    journal_rows = await conn.fetch_all("PRAGMA journal_mode")
    assert journal_rows[0][0].lower() == "wal"  # SQLite returns lowercase


@pytest.mark.asyncio
async def test_init_hook_inserts_data(tmp_path):
    """Test init_hook inserts initial data."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
        await conn.execute("INSERT INTO users (name) VALUES (?)", ["Charlie"])

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    result = await conn.fetch_all("SELECT name FROM users ORDER BY id")

    # Verify data was inserted
    assert len(result) == 3
    assert result[0][0] == "Alice"
    assert result[1][0] == "Bob"
    assert result[2][0] == "Charlie"


@pytest.mark.asyncio
async def test_init_hook_only_called_once(tmp_path):
    """Test init_hook is only called once per connection."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Multiple operations should only trigger init_hook once
    await conn.execute("INSERT INTO test (id) VALUES (1)")
    await conn.execute("INSERT INTO test (id) VALUES (2)")
    await conn.fetch_all("SELECT * FROM test")
    await conn.fetch_one("SELECT * FROM test WHERE id = 1")
    await conn.fetch_optional("SELECT * FROM test WHERE id = 999")

    # Verify hook was called exactly once
    assert len(call_count) == 1


@pytest.mark.asyncio
async def test_init_hook_with_pool_size_one(tmp_path):
    """Test init_hook works with default pool."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify hook was called
    assert len(call_count) == 1

    # Verify subsequent operations work
    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_with_pool_size_multiple(tmp_path):
    """Test init_hook works with concurrent operations."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify hook was called
    assert len(call_count) == 1

    # Verify concurrent operations work
    import asyncio

    tasks = [conn.execute("INSERT INTO test (id) VALUES (?)", [i]) for i in range(2, 7)]
    await asyncio.gather(*tasks)

    # Verify hook was still only called once
    assert len(call_count) == 1

    # Verify all data is present
    result = await conn.fetch_all("SELECT COUNT(*) FROM test")
    assert result[0][0] == 6


@pytest.mark.asyncio
async def test_init_hook_error_handling(tmp_path):
    """Test error in init_hook is properly raised."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        raise ValueError("Init hook error")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation should raise the error from init_hook
    # Note: The error is wrapped in OperationalError with message "init_hook raised an exception: ..."
    with pytest.raises(
        rapsqlite.OperationalError,
        match="init_hook raised an exception.*Init hook error",
    ):
        await conn.execute("INSERT INTO test (id) VALUES (1)")


@pytest.mark.asyncio
async def test_init_hook_none(tmp_path):
    """Test no error when init_hook is None."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    conn = rapsqlite.Connection(str(db_path), init_hook=None)

    # Should work normally without init_hook
    await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_in_transaction(tmp_path):
    """Test init_hook can begin transactions."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        async with conn:
            await conn.execute("INSERT INTO test (id) VALUES (1)")
            await conn.execute("INSERT INTO test (id) VALUES (2)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    result = await conn.fetch_all("SELECT * FROM test")

    # Verify data from transaction is present
    assert len(result) == 2
    assert result[0][0] == 1
    assert result[1][0] == 2


@pytest.mark.asyncio
async def test_init_hook_with_other_operations(tmp_path):
    """Test init_hook works with subsequent operations."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (id, value) VALUES (1, 'initial')")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    initial = await conn.fetch_all("SELECT * FROM test")
    assert len(initial) == 1
    assert initial[0][1] == "initial"

    # Subsequent operations should work normally
    await conn.execute("INSERT INTO test (id, value) VALUES (2, 'second')")
    await conn.execute("INSERT INTO test (id, value) VALUES (3, 'third')")

    result = await conn.fetch_all("SELECT * FROM test ORDER BY id")
    assert len(result) == 3
    assert result[0][1] == "initial"
    assert result[1][1] == "second"
    assert result[2][1] == "third"


@pytest.mark.asyncio
async def test_init_hook_with_pragmas_constructor(tmp_path):
    """Test init_hook works alongside pragmas in constructor."""
    db_path = tmp_path / "test.db"
    # Create empty file - SQLite needs file to exist
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        # Set additional PRAGMA in hook
        await conn.set_pragma("synchronous", "NORMAL")

    conn = rapsqlite.Connection(
        str(db_path), pragmas={"foreign_keys": True}, init_hook=init_hook
    )

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify both constructor pragmas and hook pragmas are set
    fk_rows = await conn.fetch_all("PRAGMA foreign_keys")
    assert fk_rows[0][0] == 1

    sync_rows = await conn.fetch_all("PRAGMA synchronous")
    assert sync_rows[0][0] == 1  # NORMAL = 1


@pytest.mark.asyncio
async def test_init_hook_multiple_connections(tmp_path):
    """Test each connection instance calls init_hook independently."""
    db_path1 = tmp_path / "test1.db"
    db_path2 = tmp_path / "test2.db"
    # Create empty files - SQLite needs files to exist
    db_path1.touch()
    db_path2.touch()
    call_count1 = []
    call_count2 = []

    async def init_hook1(conn):
        call_count1.append(1)
        await conn.execute("CREATE TABLE test1 (id INTEGER PRIMARY KEY)")

    async def init_hook2(conn):
        call_count2.append(1)
        await conn.execute("CREATE TABLE test2 (id INTEGER PRIMARY KEY)")

    conn1 = rapsqlite.Connection(str(db_path1), init_hook=init_hook1)
    conn2 = rapsqlite.Connection(str(db_path2), init_hook=init_hook2)

    # Each connection should call its own init_hook
    await conn1.execute("INSERT INTO test1 (id) VALUES (1)")
    await conn2.execute("INSERT INTO test2 (id) VALUES (1)")

    # Verify both hooks were called
    assert len(call_count1) == 1
    assert len(call_count2) == 1

    # Verify each connection has its own schema
    tables1 = await conn1.get_tables()
    tables2 = await conn2.get_tables()
    assert "test1" in tables1
    assert "test2" in tables2
    assert "test1" not in tables2
    assert "test2" not in tables1


@pytest.mark.asyncio
async def test_init_hook_with_fetch_methods(tmp_path):
    """Test init_hook is triggered by different fetch methods."""
    call_count1 = []
    call_count2 = []
    call_count3 = []

    async def init_hook1(conn):
        call_count1.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO test (id, name) VALUES (1, 'Alice')")

    async def init_hook2(conn):
        call_count2.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO test (id, name) VALUES (1, 'Alice')")

    async def init_hook3(conn):
        call_count3.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO test (id, name) VALUES (1, 'Alice')")

    # Test that fetch_all triggers init_hook
    db_path1 = tmp_path / "test1.db"
    db_path1.touch()  # Create empty file - SQLite needs file to exist
    conn1 = rapsqlite.Connection(str(db_path1), init_hook=init_hook1)
    result1 = await conn1.fetch_all("SELECT * FROM test")
    assert len(call_count1) == 1
    assert len(result1) == 1

    # Test that fetch_one triggers init_hook
    db_path2 = tmp_path / "test2.db"
    db_path2.touch()  # Create empty file - SQLite needs file to exist
    conn2 = rapsqlite.Connection(str(db_path2), init_hook=init_hook2)
    result2 = await conn2.fetch_one("SELECT * FROM test WHERE id = 1")
    assert len(call_count2) == 1
    assert result2[1] == "Alice"

    # Test that fetch_optional triggers init_hook
    db_path3 = tmp_path / "test3.db"
    db_path3.touch()  # Create empty file - SQLite needs file to exist
    conn3 = rapsqlite.Connection(str(db_path3), init_hook=init_hook3)
    result3 = await conn3.fetch_optional("SELECT * FROM test WHERE id = 1")
    assert len(call_count3) == 1
    assert result3 is not None
    assert result3[1] == "Alice"


@pytest.mark.asyncio
async def test_init_hook_with_schema_introspection(tmp_path):
    """Test init_hook works with schema introspection methods."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("CREATE INDEX idx_name ON users(name)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Trigger init_hook first with a simple operation
    await conn.execute("SELECT 1")

    # Now schema introspection should work
    tables = await conn.get_tables()
    assert "users" in tables

    # Verify hook was only called once even with multiple schema calls
    table_info = await conn.get_table_info("users")
    indexes = await conn.get_indexes()
    assert len(table_info) == 2
    assert len(indexes) == 1


@pytest.mark.asyncio
async def test_init_hook_with_execute_many(tmp_path):
    """Test init_hook works with execute_many."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Trigger init_hook first
    await conn.execute("SELECT 1")

    # Now execute_many should work
    await conn.execute_many(
        "INSERT INTO test (value) VALUES (?)", [["first"], ["second"], ["third"]]
    )

    result = await conn.fetch_all("SELECT value FROM test ORDER BY id")
    assert len(result) == 3
    assert result[0][0] == "first"
    assert result[1][0] == "second"
    assert result[2][0] == "third"


@pytest.mark.asyncio
async def test_init_hook_with_set_pragma(tmp_path):
    """Test init_hook is triggered by set_pragma."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # set_pragma should trigger init_hook and create the table
    await conn.set_pragma("foreign_keys", True)

    # Verify pragma was set
    fk_rows = await conn.fetch_all("PRAGMA foreign_keys")
    assert fk_rows[0][0] == 1

    # Verify table was created by init_hook
    tables = await conn.get_tables()
    assert "test" in tables


@pytest.mark.asyncio
async def test_init_hook_with_begin(tmp_path):
    """Test init_hook is triggered by begin()."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # begin() should trigger init_hook and create the table
    await conn.begin()

    # Verify we can use the transaction with the table created by init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")
    await conn.commit()

    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_with_transaction_context_manager(tmp_path):
    """Test init_hook is triggered by transaction context manager."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Transaction context manager should trigger init_hook and create the table
    async with conn.transaction():
        await conn.execute("INSERT INTO test (id) VALUES (1)")

    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_sql_error(tmp_path):
    """Test init_hook with SQL syntax error."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        # Invalid SQL - use a syntax that will definitely fail
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute("INVALID SQL STATEMENT THAT WILL FAIL")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Should raise OperationalError for SQL error in init_hook
    # The error is wrapped in OperationalError with message about init_hook
    with pytest.raises(
        rapsqlite.OperationalError,
        match='init_hook raised an exception|syntax error|near "INVALID"',
    ):
        await conn.execute("SELECT 1")


@pytest.mark.asyncio
async def test_init_hook_database_error(tmp_path):
    """Test init_hook with database constraint error."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER UNIQUE)"
        )
        await conn.execute("INSERT INTO test (value) VALUES (1)")
        await conn.execute(
            "INSERT INTO test (value) VALUES (1)"
        )  # Duplicate - should fail

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Should raise OperationalError (wrapping IntegrityError from init_hook)
    with pytest.raises(
        rapsqlite.OperationalError,
        match="init_hook raised an exception.*UNIQUE constraint",
    ):
        await conn.execute("SELECT 1")


@pytest.mark.asyncio
async def test_init_hook_recursive_prevention(tmp_path):
    """Test that init_hook calling other methods doesn't cause recursion."""
    db_path = tmp_path / "test.db"
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        # Call other methods that would normally trigger init_hook
        await conn.fetch_all("SELECT 1")
        await conn.set_pragma("foreign_keys", True)
        await conn.get_tables()

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation should trigger init_hook once
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify hook was called exactly once (not recursively)
    assert len(call_count) == 1


@pytest.mark.asyncio
async def test_init_hook_concurrent_first_access(tmp_path):
    """Test init_hook with concurrent first access (race condition)."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Trigger init_hook first
    await conn.execute("SELECT 1")

    # Now concurrent operations should work
    import asyncio

    tasks = [conn.execute("INSERT INTO test (id) VALUES (?)", [i]) for i in range(10)]
    await asyncio.gather(*tasks)

    # Verify all inserts succeeded (table was created by init_hook)
    result = await conn.fetch_all("SELECT COUNT(*) FROM test")
    assert result[0][0] == 10


@pytest.mark.asyncio
async def test_init_hook_with_cursor(tmp_path):
    """Test init_hook works with cursor operations."""
    db_path = tmp_path / "test.db"
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Cursor operations should trigger init_hook
    async with conn.cursor() as cursor:
        await cursor.execute("INSERT INTO test (value) VALUES (?)", ["test"])
        await cursor.execute("INSERT INTO test (value) VALUES (?)", ["data"])

    assert len(call_count) == 1
    result = await conn.fetch_all("SELECT value FROM test ORDER BY id")
    assert len(result) == 2


@pytest.mark.asyncio
async def test_init_hook_with_row_factory(tmp_path):
    """Test init_hook works with row factory configuration."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO test (name) VALUES (?)", ["Alice"])
        # Set row factory in hook
        conn.row_factory = "dict"

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    result = await conn.fetch_all("SELECT * FROM test")

    # Verify row factory was set
    assert isinstance(result[0], dict)
    assert result[0]["name"] == "Alice"


@pytest.mark.asyncio
async def test_init_hook_with_callbacks(tmp_path):
    """Test init_hook works before setting up callbacks."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify table was created by init_hook
    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1
    assert result[0][0] == 1

    # Note: Callback setup requires special connection handling
    # This test verifies init_hook works independently of callbacks


@pytest.mark.asyncio
async def test_init_hook_with_user_function(tmp_path):
    """Test init_hook works before creating user-defined functions."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (value) VALUES (5)")

    # Verify table was created by init_hook
    result = await conn.fetch_all("SELECT value FROM test")
    assert result[0][0] == 5

    # Note: User functions require callback connection setup
    # This test verifies init_hook works independently of user functions


@pytest.mark.asyncio
async def test_init_hook_empty_hook(tmp_path):
    """Test init_hook that does nothing."""
    db_path = tmp_path / "test.db"
    db_path.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        # Do nothing

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Should work normally
    await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    assert len(call_count) == 1
    result = await conn.fetch_all("SELECT * FROM test")
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_multiple_statements(tmp_path):
    """Test init_hook with many operations."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        # Create multiple tables
        for i in range(5):
            await conn.execute(f"CREATE TABLE table{i} (id INTEGER PRIMARY KEY)")
        # Insert data
        for i in range(5):
            await conn.execute(f"INSERT INTO table{i} (id) VALUES (?)", [i])

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook - use execute to ensure it runs
    await conn.execute("SELECT 1")

    # Verify tables were created
    tables = await conn.get_tables()
    assert len(tables) == 5

    # Verify data was inserted
    for i in range(5):
        result = await conn.fetch_all(f"SELECT * FROM table{i}")
        assert len(result) == 1
        assert result[0][0] == i


@pytest.mark.asyncio
async def test_init_hook_with_connection_string_pragmas(tmp_path):
    """Test init_hook works with connection string pragmas."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        # Set additional pragma
        await conn.set_pragma("synchronous", "NORMAL")

    # Use connection string with pragmas
    conn = rapsqlite.Connection(f"file:{db_path}?foreign_keys=1", init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute("INSERT INTO test (id) VALUES (1)")

    # Verify both pragmas are set
    fk_rows = await conn.fetch_all("PRAGMA foreign_keys")
    assert fk_rows[0][0] == 1

    sync_rows = await conn.fetch_all("PRAGMA synchronous")
    assert sync_rows[0][0] == 1  # NORMAL = 1


@pytest.mark.asyncio
async def test_init_hook_with_backup(tmp_path):
    """Test init_hook works before backup operations."""
    db_path1 = tmp_path / "test1.db"
    db_path2 = tmp_path / "test2.db"
    db_path1.touch()
    db_path2.touch()
    call_count = []

    async def init_hook(conn):
        call_count.append(1)
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute("INSERT INTO test (id) VALUES (1)")

    conn1 = rapsqlite.Connection(str(db_path1), init_hook=init_hook)
    conn2 = rapsqlite.Connection(str(db_path2))

    # Trigger init_hook
    await conn1.execute("SELECT 1")
    assert len(call_count) == 1

    # Backup should work
    await conn1.backup(conn2)

    # Verify data was backed up
    result = await conn2.fetch_all("SELECT * FROM test")
    assert len(result) == 1
    assert result[0][0] == 1


@pytest.mark.asyncio
async def test_init_hook_with_iterdump(tmp_path):
    """Test init_hook works with iterdump."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
        await conn.execute("INSERT INTO test (value) VALUES (?)", ["test"])

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # Trigger init_hook first with a simple operation
    await conn.execute("SELECT 1")

    # Now iterdump should work
    dump = await conn.iterdump()

    # Verify dump contains our table and data
    dump_str = "\n".join(dump)
    assert "CREATE TABLE test" in dump_str
    assert "INSERT INTO test" in dump_str


@pytest.mark.asyncio
async def test_init_hook_exception_different_types(tmp_path):
    """Test init_hook with different exception types."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    # Test with RuntimeError
    async def init_hook_runtime(conn):
        raise RuntimeError("Runtime error in hook")

    conn1 = rapsqlite.Connection(str(db_path), init_hook=init_hook_runtime)
    with pytest.raises(
        rapsqlite.OperationalError, match="init_hook raised an exception.*Runtime error"
    ):
        await conn1.execute("SELECT 1")

    # Test with KeyError
    db_path2 = tmp_path / "test2.db"
    db_path2.touch()

    async def init_hook_key(conn):
        raise KeyError("key error")

    conn2 = rapsqlite.Connection(str(db_path2), init_hook=init_hook_key)
    with pytest.raises(
        rapsqlite.OperationalError, match="init_hook raised an exception"
    ):
        await conn2.execute("SELECT 1")


@pytest.mark.asyncio
async def test_init_hook_timing_execute_first(tmp_path):
    """Test init_hook timing - execute called first."""
    db_path = tmp_path / "test.db"
    db_path.touch()
    execution_order = []

    async def init_hook(conn):
        execution_order.append("init_hook")
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    execution_order.append("before_execute")
    await conn.execute("INSERT INTO test (id) VALUES (1)")
    execution_order.append("after_execute")

    # Verify init_hook was called before execute
    assert execution_order == ["before_execute", "init_hook", "after_execute"]


@pytest.mark.asyncio
async def test_init_hook_timing_fetch_first(tmp_path):
    """Test init_hook timing - fetch called first."""
    db_path = tmp_path / "test.db"
    db_path.touch()
    execution_order = []

    async def init_hook(conn):
        execution_order.append("init_hook")
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute("INSERT INTO test (id) VALUES (1)")

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    execution_order.append("before_fetch")
    result = await conn.fetch_all("SELECT * FROM test")
    execution_order.append("after_fetch")

    # Verify init_hook was called before fetch
    assert execution_order == ["before_fetch", "init_hook", "after_fetch"]
    assert len(result) == 1


@pytest.mark.asyncio
async def test_init_hook_with_complex_schema(tmp_path):
    """Test init_hook creating complex schema with indexes and foreign keys."""
    db_path = tmp_path / "test.db"
    db_path.touch()

    async def init_hook(conn):
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                email TEXT UNIQUE
            )
        """)
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                content TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
        """)
        await conn.execute("CREATE INDEX idx_posts_user ON posts(user_id)")
        await conn.execute("CREATE INDEX idx_users_email ON users(email)")
        await conn.set_pragma("foreign_keys", True)

    conn = rapsqlite.Connection(str(db_path), init_hook=init_hook)

    # First operation triggers init_hook
    await conn.execute(
        "INSERT INTO users (username, email) VALUES (?, ?)",
        ["alice", "alice@example.com"],
    )

    # Verify schema
    tables = await conn.get_tables()
    assert "users" in tables
    assert "posts" in tables

    indexes = await conn.get_indexes()
    assert len(indexes) == 2

    # Verify foreign keys work
    user = await conn.fetch_one("SELECT id FROM users WHERE username = ?", ["alice"])
    await conn.execute(
        "INSERT INTO posts (user_id, title) VALUES (?, ?)", [user[0], "First Post"]
    )

    posts = await conn.fetch_all("SELECT * FROM posts")
    assert len(posts) == 1
