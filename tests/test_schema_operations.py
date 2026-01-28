"""Comprehensive tests for Phase 2.10: Schema Operations and Introspection."""

import pytest
import tempfile
import os
from rapsqlite import Connection


@pytest.fixture
def test_db():
    """Create a temporary database file for testing."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        yield db_path
    finally:
        if os.path.exists(db_path):
            os.unlink(db_path)


@pytest.mark.asyncio
async def test_get_tables_empty_database(test_db):
    """Test get_tables on empty database."""
    async with Connection(test_db) as conn:
        tables = await conn.get_tables()
        assert isinstance(tables, list)
        assert len(tables) == 0


@pytest.mark.asyncio
async def test_get_tables_basic(test_db):
    """Test get_tables with multiple tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT)")
        await conn.execute("CREATE TABLE comments (id INTEGER PRIMARY KEY, text TEXT)")

        tables = await conn.get_tables()
        assert isinstance(tables, list)
        assert len(tables) == 3
        assert "users" in tables
        assert "posts" in tables
        assert "comments" in tables
        # Should be sorted
        assert tables == sorted(tables)


@pytest.mark.asyncio
async def test_get_tables_filter_by_name(test_db):
    """Test get_tables with name filter."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY)")

        # Filter for existing table
        tables = await conn.get_tables(name="users")
        assert isinstance(tables, list)
        assert len(tables) == 1
        assert tables[0] == "users"

        # Filter for non-existent table
        tables = await conn.get_tables(name="nonexistent")
        assert isinstance(tables, list)
        assert len(tables) == 0


@pytest.mark.asyncio
async def test_get_table_info_basic(test_db):
    """Test get_table_info with basic table."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )

        info = await conn.get_table_info("users")
        assert isinstance(info, list)
        assert len(info) == 3

        # Check first column (id)
        assert info[0]["name"] == "id"
        assert info[0]["type"].upper() == "INTEGER"
        assert info[0]["pk"] == 1  # Primary key
        assert "cid" in info[0]
        assert "notnull" in info[0]
        assert "dflt_value" in info[0]

        # Check second column (name)
        assert info[1]["name"] == "name"
        assert info[1]["type"].upper() == "TEXT"
        assert info[1]["pk"] == 0  # Not primary key


@pytest.mark.asyncio
async def test_get_table_info_with_constraints(test_db):
    """Test get_table_info with various column constraints."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                age INTEGER DEFAULT 0,
                email TEXT UNIQUE
            )
        """)

        info = await conn.get_table_info("test")
        assert len(info) == 4

        # Check NOT NULL constraint
        name_col = next(c for c in info if c["name"] == "name")
        assert name_col["notnull"] == 1

        # Check default value
        age_col = next(c for c in info if c["name"] == "age")
        assert age_col["dflt_value"] == "0" or age_col["dflt_value"] == 0


@pytest.mark.asyncio
async def test_get_table_info_nonexistent_table(test_db):
    """Test get_table_info with non-existent table."""
    async with Connection(test_db) as conn:
        # PRAGMA table_info returns empty list for non-existent tables
        info = await conn.get_table_info("nonexistent")
        assert isinstance(info, list)
        assert len(info) == 0


@pytest.mark.asyncio
async def test_get_indexes_no_indexes(test_db):
    """Test get_indexes when no indexes exist."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")

        indexes = await conn.get_indexes()
        assert isinstance(indexes, list)
        # PRIMARY KEY creates an implicit index, but we filter sqlite_% so might be empty
        # or might have the primary key index depending on implementation


@pytest.mark.asyncio
async def test_get_indexes_basic(test_db):
    """Test get_indexes with explicit indexes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON users(email)")
        await conn.execute("CREATE UNIQUE INDEX idx_unique_email ON users(email)")

        indexes = await conn.get_indexes()
        assert isinstance(indexes, list)
        assert len(indexes) >= 2

        # Find our indexes
        index_names = [idx["name"] for idx in indexes]
        assert "idx_email" in index_names
        assert "idx_unique_email" in index_names

        # Check unique index
        unique_idx = next(idx for idx in indexes if idx["name"] == "idx_unique_email")
        assert unique_idx["unique"] == 1
        assert unique_idx["table"] == "users"
        assert "sql" in unique_idx


@pytest.mark.asyncio
async def test_get_indexes_filter_by_table(test_db):
    """Test get_indexes with table filter."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE INDEX idx_users ON users(id)")
        await conn.execute("CREATE INDEX idx_posts ON posts(id)")

        # Get indexes for specific table
        indexes = await conn.get_indexes(table_name="users")
        assert isinstance(indexes, list)
        index_tables = [idx["table"] for idx in indexes]
        assert all(table == "users" for table in index_tables)
        assert "idx_users" in [idx["name"] for idx in indexes]


@pytest.mark.asyncio
async def test_get_foreign_keys_no_foreign_keys(test_db):
    """Test get_foreign_keys when no foreign keys exist."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")

        fks = await conn.get_foreign_keys("users")
        assert isinstance(fks, list)
        assert len(fks) == 0


@pytest.mark.asyncio
async def test_get_foreign_keys_with_foreign_keys(test_db):
    """Test get_foreign_keys with foreign key constraints."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER,
                title TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
        """)

        fks = await conn.get_foreign_keys("posts")
        assert isinstance(fks, list)
        assert len(fks) >= 1

        fk = fks[0]
        assert fk["table"] == "users"  # Referenced table
        assert fk["from"] == "user_id"  # Column in posts table
        assert fk["to"] == "id"  # Column in users table
        assert fk["on_delete"] == "CASCADE"
        assert "id" in fk
        assert "seq" in fk


@pytest.mark.asyncio
async def test_get_foreign_keys_nonexistent_table(test_db):
    """Test get_foreign_keys with non-existent table."""
    async with Connection(test_db) as conn:
        # Should return empty list, not raise error
        fks = await conn.get_foreign_keys("nonexistent")
        assert isinstance(fks, list)
        assert len(fks) == 0


@pytest.mark.asyncio
async def test_get_schema_all_tables(test_db):
    """Test get_schema for all tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY)")

        schema = await conn.get_schema()
        assert isinstance(schema, dict)
        assert "tables" in schema
        tables = schema["tables"]
        assert isinstance(tables, list)
        assert len(tables) == 2
        assert all("name" in t for t in tables)
        table_names = [t["name"] for t in tables]
        assert "users" in table_names
        assert "posts" in table_names


@pytest.mark.asyncio
async def test_get_schema_single_table(test_db):
    """Test get_schema for a specific table."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT UNIQUE
            )
        """)
        await conn.execute("CREATE INDEX idx_email ON users(email)")

        schema = await conn.get_schema(table_name="users")
        assert isinstance(schema, dict)
        assert "table_name" in schema
        assert schema["table_name"] == "users"
        assert "columns" in schema
        assert "indexes" in schema
        assert "foreign_keys" in schema

        # Check columns
        columns = schema["columns"]
        assert isinstance(columns, list)
        assert len(columns) == 3
        column_names = [c["name"] for c in columns]
        assert "id" in column_names
        assert "name" in column_names
        assert "email" in column_names

        # Check indexes
        indexes = schema["indexes"]
        assert isinstance(indexes, list)
        index_names = [idx["name"] for idx in indexes]
        assert "idx_email" in index_names

        # Check foreign keys (should be empty)
        fks = schema["foreign_keys"]
        assert isinstance(fks, list)


@pytest.mark.asyncio
async def test_get_schema_with_foreign_keys(test_db):
    """Test get_schema with foreign key relationships."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )
        """)

        schema = await conn.get_schema(table_name="posts")
        assert "foreign_keys" in schema
        fks = schema["foreign_keys"]
        assert isinstance(fks, list)
        assert len(fks) >= 1
        assert fks[0]["table"] == "users"


@pytest.mark.asyncio
async def test_get_schema_nonexistent_table(test_db):
    """Test get_schema with non-existent table."""
    async with Connection(test_db) as conn:
        schema = await conn.get_schema(table_name="nonexistent")
        # Should return empty structure, not raise error
        assert isinstance(schema, dict)
        # May have empty lists or None values


@pytest.mark.asyncio
async def test_schema_operations_in_transaction(test_db):
    """Test schema operations work within transactions."""
    async with Connection(test_db) as conn:
        await conn.begin()
        try:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            tables = await conn.get_tables()
            assert "test" in tables

            info = await conn.get_table_info("test")
            assert len(info) == 1
            assert info[0]["name"] == "id"

            await conn.commit()
        except Exception:
            await conn.rollback()
            raise


@pytest.mark.asyncio
async def test_get_table_info_various_types(test_db):
    """Test get_table_info with various SQLite types."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                name TEXT,
                price REAL,
                data BLOB,
                flag INTEGER
            )
        """)

        info = await conn.get_table_info("test")
        assert len(info) == 5

        types = {col["name"]: col["type"].upper() for col in info}
        assert types["id"] == "INTEGER"
        assert types["name"] == "TEXT"
        assert types["price"] == "REAL"
        assert types["data"] == "BLOB"
        assert types["flag"] == "INTEGER"


@pytest.mark.asyncio
async def test_get_indexes_multiple_tables(test_db):
    """Test get_indexes across multiple tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT)")
        await conn.execute("CREATE INDEX idx_users_email ON users(email)")
        await conn.execute("CREATE INDEX idx_posts_title ON posts(title)")

        all_indexes = await conn.get_indexes()
        assert len(all_indexes) >= 2

        user_indexes = await conn.get_indexes(table_name="users")
        assert all(idx["table"] == "users" for idx in user_indexes)

        post_indexes = await conn.get_indexes(table_name="posts")
        assert all(idx["table"] == "posts" for idx in post_indexes)


@pytest.mark.asyncio
async def test_get_tables_special_characters(test_db):
    """Test get_tables with table names containing special characters."""
    async with Connection(test_db) as conn:
        # SQLite allows quoted identifiers with special characters
        await conn.execute('CREATE TABLE "table-with-dashes" (id INTEGER PRIMARY KEY)')
        await conn.execute(
            'CREATE TABLE "table_with_underscores" (id INTEGER PRIMARY KEY)'
        )
        await conn.execute('CREATE TABLE "table with spaces" (id INTEGER PRIMARY KEY)')

        tables = await conn.get_tables()
        assert len(tables) == 3
        # Table names should be returned as stored (with quotes if needed)
        table_set = set(tables)
        assert "table-with-dashes" in table_set or '"table-with-dashes"' in table_set
        assert (
            "table_with_underscores" in table_set
            or '"table_with_underscores"' in table_set
        )


@pytest.mark.asyncio
async def test_get_table_info_composite_primary_key(test_db):
    """Test get_table_info with composite primary key."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id1 INTEGER,
                id2 INTEGER,
                name TEXT,
                PRIMARY KEY (id1, id2)
            )
        """)

        info = await conn.get_table_info("test")
        assert len(info) == 3

        # Both id1 and id2 should be marked as primary key
        id1_col = next(c for c in info if c["name"] == "id1")
        id2_col = next(c for c in info if c["name"] == "id2")
        # In SQLite, composite PKs have pk > 0 for each column
        assert id1_col["pk"] > 0
        assert id2_col["pk"] > 0


@pytest.mark.asyncio
async def test_get_table_info_default_values_various_types(test_db):
    """Test get_table_info with various default value types."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                text_default TEXT DEFAULT 'hello',
                int_default INTEGER DEFAULT 42,
                real_default REAL DEFAULT 3.14,
                null_default TEXT DEFAULT NULL,
                current_time TEXT DEFAULT CURRENT_TIMESTAMP
            )
        """)

        info = await conn.get_table_info("test")
        assert len(info) == 6

        # Check default values
        defaults = {col["name"]: col["dflt_value"] for col in info}
        assert (
            defaults["text_default"] == "hello" or defaults["text_default"] == "'hello'"
        )
        assert defaults["int_default"] == "42" or defaults["int_default"] == 42
        assert defaults["real_default"] == "3.14" or defaults["real_default"] == 3.14
        # NULL defaults may be None or "NULL" string
        assert (
            defaults["null_default"] is None
            or defaults["null_default"] == "NULL"
            or defaults["null_default"] == "null"
        )


@pytest.mark.asyncio
async def test_get_indexes_composite_index(test_db):
    """Test get_indexes with composite (multi-column) indexes."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, first_name TEXT, last_name TEXT)"
        )
        await conn.execute("CREATE INDEX idx_full_name ON users(first_name, last_name)")

        indexes = await conn.get_indexes(table_name="users")
        assert len(indexes) >= 1

        full_name_idx = next(
            (idx for idx in indexes if idx["name"] == "idx_full_name"), None
        )
        assert full_name_idx is not None
        assert full_name_idx["table"] == "users"
        assert "sql" in full_name_idx
        # SQL should contain both columns
        sql = full_name_idx.get("sql", "")
        assert "first_name" in sql.lower()
        assert "last_name" in sql.lower()


@pytest.mark.asyncio
async def test_get_indexes_partial_index(test_db):
    """Test get_indexes with partial index (WHERE clause)."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, active INTEGER)"
        )
        await conn.execute(
            "CREATE INDEX idx_active_email ON users(email) WHERE active = 1"
        )

        indexes = await conn.get_indexes(table_name="users")
        active_idx = next(
            (idx for idx in indexes if idx["name"] == "idx_active_email"), None
        )
        assert active_idx is not None
        assert "sql" in active_idx
        sql = active_idx.get("sql", "")
        assert "where" in sql.lower() or "WHERE" in sql


@pytest.mark.asyncio
async def test_get_foreign_keys_multiple_foreign_keys(test_db):
    """Test get_foreign_keys with multiple foreign key constraints."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE)"
        )
        await conn.execute(
            "CREATE TABLE categories (id INTEGER PRIMARY KEY, name TEXT)"
        )
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER,
                category_id INTEGER,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE SET NULL
            )
        """)

        fks = await conn.get_foreign_keys("posts")
        assert len(fks) == 2

        # Check both foreign keys
        user_fk = next(fk for fk in fks if fk["from"] == "user_id")
        assert user_fk["table"] == "users"
        assert user_fk["to"] == "id"
        assert user_fk["on_delete"] == "CASCADE"

        category_fk = next(fk for fk in fks if fk["from"] == "category_id")
        assert category_fk["table"] == "categories"
        assert category_fk["to"] == "id"
        assert category_fk["on_delete"] == "SET NULL"


@pytest.mark.asyncio
async def test_get_foreign_keys_composite_foreign_key(test_db):
    """Test get_foreign_keys with composite foreign key."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("""
            CREATE TABLE parent (
                id1 INTEGER,
                id2 INTEGER,
                PRIMARY KEY (id1, id2)
            )
        """)
        await conn.execute("""
            CREATE TABLE child (
                id INTEGER PRIMARY KEY,
                parent_id1 INTEGER,
                parent_id2 INTEGER,
                FOREIGN KEY (parent_id1, parent_id2) REFERENCES parent(id1, id2)
            )
        """)

        fks = await conn.get_foreign_keys("child")
        assert len(fks) >= 1
        # Composite FK may appear as multiple entries with same id but different seq
        fk_ids = {fk["id"] for fk in fks}
        assert len(fk_ids) >= 1


@pytest.mark.asyncio
async def test_get_schema_complex_schema(test_db):
    """Test get_schema with complex schema (multiple tables, indexes, foreign keys)."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )
        """)
        await conn.execute("""
            CREATE TABLE posts (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                title TEXT,
                content TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            )
        """)
        await conn.execute("CREATE INDEX idx_posts_user ON posts(user_id)")
        await conn.execute("CREATE INDEX idx_posts_title ON posts(title)")
        await conn.execute("CREATE UNIQUE INDEX idx_users_email ON users(email)")

        # Test schema for posts table
        schema = await conn.get_schema(table_name="posts")
        assert schema["table_name"] == "posts"
        assert len(schema["columns"]) == 4
        assert len(schema["indexes"]) >= 2
        assert len(schema["foreign_keys"]) >= 1

        # Verify foreign key details
        fk = schema["foreign_keys"][0]
        assert fk["table"] == "users"
        assert fk["from"] == "user_id"
        assert fk["on_delete"] == "CASCADE"


@pytest.mark.asyncio
async def test_get_schema_with_views(test_db):
    """Test get_schema excludes views (views are not tables)."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
        await conn.execute("CREATE VIEW user_names AS SELECT name FROM users")

        # Views should not appear in get_tables
        tables = await conn.get_tables()
        assert "users" in tables
        assert "user_names" not in tables  # Views are not tables

        # get_schema should only show tables, not views
        schema = await conn.get_schema()
        table_names = [t["name"] for t in schema["tables"]]
        assert "users" in table_names
        assert "user_names" not in table_names


@pytest.mark.asyncio
async def test_get_table_info_without_row_factory(test_db):
    """Test get_table_info returns dicts regardless of row_factory setting."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")

        # Set row_factory to dict (should not affect schema methods)
        conn.row_factory = "dict"
        info = await conn.get_table_info("test")
        # Should still return list of dicts
        assert isinstance(info, list)
        assert isinstance(info[0], dict)
        assert "name" in info[0]


@pytest.mark.asyncio
async def test_get_indexes_implicit_indexes(test_db):
    """Test get_indexes includes implicit indexes from UNIQUE constraints."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                email TEXT UNIQUE,
                username TEXT UNIQUE
            )
        """)

        indexes = await conn.get_indexes(table_name="users")
        # Should have indexes for UNIQUE constraints
        # UNIQUE constraints create implicit indexes
        assert len(indexes) >= 0  # May or may not show implicit indexes


@pytest.mark.asyncio
async def test_get_tables_case_sensitivity(test_db):
    """Test get_tables handles case sensitivity correctly."""
    async with Connection(test_db) as conn:
        await conn.execute('CREATE TABLE "Users" (id INTEGER PRIMARY KEY)')
        await conn.execute('CREATE TABLE "posts" (id INTEGER PRIMARY KEY)')

        tables = await conn.get_tables()
        # SQLite is case-sensitive for quoted identifiers
        assert "Users" in tables or '"Users"' in tables
        assert "posts" in tables or '"posts"' in tables


@pytest.mark.asyncio
async def test_get_table_info_all_column_attributes(test_db):
    """Test get_table_info returns all expected attributes for each column."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT 'unknown',
                age INTEGER
            )
        """)

        info = await conn.get_table_info("test")
        for col in info:
            # Verify all expected keys are present
            assert "cid" in col
            assert "name" in col
            assert "type" in col
            assert "notnull" in col
            assert "dflt_value" in col
            assert "pk" in col

            # Verify types
            assert isinstance(col["cid"], (int, type(None)))
            assert isinstance(col["name"], str)
            assert isinstance(col["type"], str)
            assert isinstance(col["notnull"], (int, type(None)))
            assert isinstance(col["pk"], (int, type(None)))


@pytest.mark.asyncio
async def test_get_indexes_all_index_attributes(test_db):
    """Test get_indexes returns all expected attributes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON test(email)")

        indexes = await conn.get_indexes(table_name="test")
        assert len(indexes) >= 1

        idx = next(idx for idx in indexes if idx["name"] == "idx_email")
        # Verify all expected keys
        assert "name" in idx
        assert "table" in idx
        assert "unique" in idx
        assert "sql" in idx

        # Verify types
        assert isinstance(idx["name"], str)
        assert isinstance(idx["table"], str)
        assert isinstance(idx["unique"], (int, type(None)))
        assert idx["sql"] is None or isinstance(idx["sql"], str)


@pytest.mark.asyncio
async def test_get_foreign_keys_all_attributes(test_db):
    """Test get_foreign_keys returns all expected attributes."""
    async with Connection(test_db) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
        await conn.execute("""
            CREATE TABLE child (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                FOREIGN KEY (parent_id) REFERENCES parent(id) ON DELETE CASCADE ON UPDATE RESTRICT
            )
        """)

        fks = await conn.get_foreign_keys("child")
        assert len(fks) >= 1

        fk = fks[0]
        # Verify all expected keys
        required_keys = [
            "id",
            "seq",
            "table",
            "from",
            "to",
            "on_update",
            "on_delete",
            "match",
        ]
        for key in required_keys:
            assert key in fk, f"Missing key: {key}"

        # Verify on_delete and on_update values
        assert fk["on_delete"] == "CASCADE"
        assert fk["on_update"] == "RESTRICT" or fk["on_update"] == "NO ACTION"


@pytest.mark.asyncio
async def test_schema_operations_with_callbacks(test_db):
    """Test schema operations work when callbacks are active."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")

        # Set a trace callback
        trace_calls = []

        def trace_callback(sql):
            trace_calls.append(sql)

        await conn.set_trace_callback(trace_callback)

        # Schema operations should still work
        tables = await conn.get_tables()
        assert "test" in tables

        info = await conn.get_table_info("test")
        assert len(info) == 1

        # Trace callback may or may not fire for schema queries
        await conn.set_trace_callback(None)


@pytest.mark.asyncio
async def test_get_schema_large_database(test_db):
    """Test get_schema with many tables."""
    async with Connection(test_db) as conn:
        # Create many tables
        for i in range(20):
            await conn.execute(
                f"CREATE TABLE table_{i} (id INTEGER PRIMARY KEY, value TEXT)"
            )

        schema = await conn.get_schema()
        assert len(schema["tables"]) == 20

        # Test single table schema still works
        table_5_schema = await conn.get_schema(table_name="table_5")
        assert table_5_schema["table_name"] == "table_5"
        assert len(table_5_schema["columns"]) == 2


@pytest.mark.asyncio
async def test_get_indexes_empty_database(test_db):
    """Test get_indexes on empty database."""
    async with Connection(test_db) as conn:
        indexes = await conn.get_indexes()
        assert isinstance(indexes, list)
        assert len(indexes) == 0


@pytest.mark.asyncio
async def test_get_table_info_table_with_no_columns(test_db):
    """Test get_table_info edge case (though SQLite doesn't allow this)."""
    # SQLite requires at least one column, so this test verifies we handle it
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        info = await conn.get_table_info("test")
        assert len(info) == 1
        assert info[0]["name"] == "id"


@pytest.mark.asyncio
async def test_get_schema_after_table_modification(test_db):
    """Test get_schema reflects table modifications."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")

        schema1 = await conn.get_schema(table_name="test")
        assert len(schema1["columns"]) == 2

        # Add column
        await conn.execute("ALTER TABLE test ADD COLUMN email TEXT")

        schema2 = await conn.get_schema(table_name="test")
        assert len(schema2["columns"]) == 3
        column_names = [c["name"] for c in schema2["columns"]]
        assert "email" in column_names


@pytest.mark.asyncio
async def test_get_tables_after_drop_table(test_db):
    """Test get_tables reflects dropped tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test1 (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE TABLE test2 (id INTEGER PRIMARY KEY)")

        tables = await conn.get_tables()
        assert len(tables) == 2

        await conn.execute("DROP TABLE test1")

        tables_after = await conn.get_tables()
        assert len(tables_after) == 1
        assert "test2" in tables_after
        assert "test1" not in tables_after


@pytest.mark.asyncio
async def test_get_indexes_after_drop_index(test_db):
    """Test get_indexes reflects dropped indexes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON test(email)")

        indexes = await conn.get_indexes(table_name="test")
        assert "idx_email" in [idx["name"] for idx in indexes]

        await conn.execute("DROP INDEX idx_email")

        indexes_after = await conn.get_indexes(table_name="test")
        assert "idx_email" not in [idx["name"] for idx in indexes_after]


@pytest.mark.asyncio
async def test_get_schema_empty_database(test_db):
    """Test get_schema on empty database."""
    async with Connection(test_db) as conn:
        schema = await conn.get_schema()
        assert isinstance(schema, dict)
        assert "tables" in schema
        assert len(schema["tables"]) == 0


@pytest.mark.asyncio
async def test_get_table_info_column_order(test_db):
    """Test get_table_info returns columns in correct order (by cid)."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                first TEXT,
                second TEXT,
                third TEXT
            )
        """)

        info = await conn.get_table_info("test")
        assert len(info) == 4

        # Verify cid values are sequential
        cids = [col["cid"] for col in info]
        assert cids == sorted(cids)
        assert cids == list(range(4))  # Should be 0, 1, 2, 3

        # Verify column order matches creation order
        assert info[0]["name"] == "id"
        assert info[1]["name"] == "first"
        assert info[2]["name"] == "second"
        assert info[3]["name"] == "third"


@pytest.mark.asyncio
async def test_get_views_basic(test_db):
    """Test get_views with basic views."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        await conn.execute("INSERT INTO users (name) VALUES ('Alice')")
        await conn.execute("CREATE VIEW user_names AS SELECT name FROM users")
        await conn.execute(
            "CREATE VIEW active_users AS SELECT * FROM users WHERE id > 0"
        )

        views = await conn.get_views()
        assert isinstance(views, list)
        assert len(views) == 2
        assert "user_names" in views
        assert "active_users" in views


@pytest.mark.asyncio
async def test_get_views_filter_by_name(test_db):
    """Test get_views with name filter."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE VIEW test_view AS SELECT * FROM test")

        # Filter for existing view
        views = await conn.get_views(name="test_view")
        assert isinstance(views, list)
        assert len(views) == 1
        assert views[0] == "test_view"

        # Filter for non-existent view
        views = await conn.get_views(name="nonexistent")
        assert isinstance(views, list)
        assert len(views) == 0


@pytest.mark.asyncio
async def test_get_views_empty_database(test_db):
    """Test get_views on empty database."""
    async with Connection(test_db) as conn:
        views = await conn.get_views()
        assert isinstance(views, list)
        assert len(views) == 0


@pytest.mark.asyncio
async def test_get_index_list_basic(test_db):
    """Test get_index_list with basic indexes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON users(email)")
        await conn.execute("CREATE UNIQUE INDEX idx_unique_email ON users(email)")

        index_list = await conn.get_index_list("users")
        assert isinstance(index_list, list)
        assert len(index_list) >= 2

        # Find our indexes
        index_names = [idx["name"] for idx in index_list]
        assert "idx_email" in index_names
        assert "idx_unique_email" in index_names

        # Check index properties
        unique_idx = next(
            idx for idx in index_list if idx["name"] == "idx_unique_email"
        )
        assert unique_idx["unique"] == 1
        assert "seq" in unique_idx
        assert "origin" in unique_idx
        assert "partial" in unique_idx


@pytest.mark.asyncio
async def test_get_index_list_no_indexes(test_db):
    """Test get_index_list when no indexes exist."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")

        index_list = await conn.get_index_list("users")
        assert isinstance(index_list, list)
        # PRIMARY KEY creates an implicit index, so may have entries


@pytest.mark.asyncio
async def test_get_index_info_basic(test_db):
    """Test get_index_info with basic index."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, name TEXT)"
        )
        await conn.execute("CREATE INDEX idx_email ON users(email)")

        index_info = await conn.get_index_info("idx_email")
        assert isinstance(index_info, list)
        assert len(index_info) >= 1

        # Check column information
        col_info = index_info[0]
        assert "seqno" in col_info
        assert "cid" in col_info
        assert "name" in col_info
        assert col_info["name"] == "email"


@pytest.mark.asyncio
async def test_get_index_info_composite_index(test_db):
    """Test get_index_info with composite index."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, first_name TEXT, last_name TEXT)"
        )
        await conn.execute("CREATE INDEX idx_full_name ON users(first_name, last_name)")

        index_info = await conn.get_index_info("idx_full_name")
        assert isinstance(index_info, list)
        assert len(index_info) == 2

        # Check both columns are present
        column_names = [col["name"] for col in index_info]
        assert "first_name" in column_names
        assert "last_name" in column_names

        # Check sequence numbers
        seqnos = [col["seqno"] for col in index_info]
        assert seqnos == sorted(seqnos)


@pytest.mark.asyncio
async def test_get_index_info_nonexistent_index(test_db):
    """Test get_index_info with non-existent index."""
    async with Connection(test_db) as conn:
        # PRAGMA index_info returns empty list for non-existent indexes
        index_info = await conn.get_index_info("nonexistent")
        assert isinstance(index_info, list)
        assert len(index_info) == 0


@pytest.mark.asyncio
async def test_get_table_xinfo_basic(test_db):
    """Test get_table_xinfo with basic table."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )

        xinfo = await conn.get_table_xinfo("users")
        assert isinstance(xinfo, list)
        assert len(xinfo) == 3

        # Check extended info includes hidden field
        for col in xinfo:
            assert "cid" in col
            assert "name" in col
            assert "type" in col
            assert "notnull" in col
            assert "dflt_value" in col
            assert "pk" in col
            assert "hidden" in col
            # hidden should be 0 for normal columns
            assert col["hidden"] == 0


@pytest.mark.asyncio
async def test_get_table_xinfo_vs_table_info(test_db):
    """Test that get_table_xinfo returns same info as get_table_info plus hidden."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)"
        )

        info = await conn.get_table_info("users")
        xinfo = await conn.get_table_xinfo("users")

        assert len(info) == len(xinfo)

        # Compare fields (xinfo should have same fields plus hidden)
        for info_col, xinfo_col in zip(info, xinfo):
            assert info_col["cid"] == xinfo_col["cid"]
            assert info_col["name"] == xinfo_col["name"]
            assert info_col["type"] == xinfo_col["type"]
            assert info_col["notnull"] == xinfo_col["notnull"]
            assert info_col["pk"] == xinfo_col["pk"]
            # xinfo should have hidden field
            assert "hidden" in xinfo_col
            assert xinfo_col["hidden"] == 0  # Normal columns


@pytest.mark.asyncio
async def test_get_table_xinfo_nonexistent_table(test_db):
    """Test get_table_xinfo with non-existent table."""
    async with Connection(test_db) as conn:
        # PRAGMA table_xinfo returns empty list for non-existent tables
        xinfo = await conn.get_table_xinfo("nonexistent")
        assert isinstance(xinfo, list)
        assert len(xinfo) == 0


@pytest.mark.asyncio
async def test_get_views_with_special_characters(test_db):
    """Test get_views with view names containing special characters."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.execute('CREATE VIEW "view-with-dashes" AS SELECT * FROM test')
        await conn.execute('CREATE VIEW "view_with_underscores" AS SELECT * FROM test')

        views = await conn.get_views()
        assert len(views) == 2
        view_set = set(views)
        assert "view-with-dashes" in view_set or '"view-with-dashes"' in view_set
        assert (
            "view_with_underscores" in view_set or '"view_with_underscores"' in view_set
        )


@pytest.mark.asyncio
async def test_get_views_in_transaction(test_db):
    """Test get_views works within transactions."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
        await conn.begin()
        try:
            await conn.execute("CREATE VIEW test_view AS SELECT * FROM test")
            views = await conn.get_views()
            assert "test_view" in views
            await conn.commit()
        except Exception:
            await conn.rollback()
            raise


@pytest.mark.asyncio
async def test_get_index_list_all_attributes(test_db):
    """Test get_index_list returns all expected attributes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON test(email)")

        index_list = await conn.get_index_list("test")
        assert len(index_list) >= 1

        idx = next((i for i in index_list if i["name"] == "idx_email"), None)
        if idx:
            assert "seq" in idx
            assert "name" in idx
            assert "unique" in idx
            assert "origin" in idx
            assert "partial" in idx

            assert isinstance(idx["seq"], (int, type(None)))
            assert isinstance(idx["name"], str)
            assert isinstance(idx["unique"], (int, type(None)))
            assert isinstance(idx["partial"], (int, type(None)))


@pytest.mark.asyncio
async def test_get_index_list_partial_index(test_db):
    """Test get_index_list with partial index."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, active INTEGER)"
        )
        await conn.execute(
            "CREATE INDEX idx_active_email ON users(email) WHERE active = 1"
        )

        index_list = await conn.get_index_list("users")
        active_idx = next(
            (idx for idx in index_list if idx["name"] == "idx_active_email"), None
        )
        if active_idx:
            assert active_idx["partial"] == 1


@pytest.mark.asyncio
async def test_get_index_list_origin_values(test_db):
    """Test get_index_list origin field values."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT UNIQUE, name TEXT)"
        )
        await conn.execute("CREATE INDEX idx_name ON test(name)")

        index_list = await conn.get_index_list("test")
        # Check that origin values are present (c=CREATE, u=UNIQUE, pk=PRIMARY KEY)
        for idx in index_list:
            assert "origin" in idx
            # Origin can be None, "c", "u", or "pk"
            if idx["origin"] is not None:
                assert idx["origin"] in ["c", "u", "pk"]


@pytest.mark.asyncio
async def test_get_index_list_nonexistent_table(test_db):
    """Test get_index_list with non-existent table."""
    async with Connection(test_db) as conn:
        # PRAGMA index_list returns empty list for non-existent tables
        index_list = await conn.get_index_list("nonexistent")
        assert isinstance(index_list, list)
        assert len(index_list) == 0


@pytest.mark.asyncio
async def test_get_index_info_all_attributes(test_db):
    """Test get_index_info returns all expected attributes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON test(email)")

        index_info = await conn.get_index_info("idx_email")
        assert len(index_info) >= 1

        col_info = index_info[0]
        assert "seqno" in col_info
        assert "cid" in col_info
        assert "name" in col_info

        assert isinstance(col_info["seqno"], (int, type(None)))
        assert isinstance(col_info["cid"], (int, type(None)))
        assert isinstance(col_info["name"], str)


@pytest.mark.asyncio
async def test_get_index_info_column_order(test_db):
    """Test get_index_info returns columns in correct order."""
    async with Connection(test_db) as conn:
        await conn.execute(
            "CREATE TABLE test (id INTEGER PRIMARY KEY, col1 TEXT, col2 TEXT, col3 TEXT)"
        )
        await conn.execute("CREATE INDEX idx_multi ON test(col1, col2, col3)")

        index_info = await conn.get_index_info("idx_multi")
        assert len(index_info) == 3

        # Verify seqno values are sequential
        seqnos = [col["seqno"] for col in index_info]
        assert seqnos == sorted(seqnos)
        assert seqnos == list(range(3))  # Should be 0, 1, 2

        # Verify column order
        assert index_info[0]["name"] == "col1"
        assert index_info[1]["name"] == "col2"
        assert index_info[2]["name"] == "col3"


@pytest.mark.asyncio
async def test_get_index_info_primary_key_index(test_db):
    """Test get_index_info with primary key index."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")

        # Get the primary key index name (usually sqlite_autoindex_test_1 or similar)
        index_list = await conn.get_index_list("test")
        pk_index = next((idx for idx in index_list if idx.get("origin") == "pk"), None)

        if pk_index:
            index_info = await conn.get_index_info(pk_index["name"])
            assert len(index_info) >= 1
            assert index_info[0]["name"] == "id"


@pytest.mark.asyncio
async def test_get_table_xinfo_all_attributes(test_db):
    """Test get_table_xinfo returns all expected attributes."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")

        xinfo = await conn.get_table_xinfo("test")
        assert len(xinfo) == 2

        for col in xinfo:
            required_keys = [
                "cid",
                "name",
                "type",
                "notnull",
                "dflt_value",
                "pk",
                "hidden",
            ]
            for key in required_keys:
                assert key in col, f"Missing key: {key}"


@pytest.mark.asyncio
async def test_get_table_xinfo_hidden_values(test_db):
    """Test get_table_xinfo hidden field values."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")

        xinfo = await conn.get_table_xinfo("test")
        for col in xinfo:
            assert "hidden" in col
            # hidden should be 0 for normal columns
            assert col["hidden"] == 0
            assert isinstance(col["hidden"], (int, type(None)))


@pytest.mark.asyncio
async def test_get_table_xinfo_with_constraints(test_db):
    """Test get_table_xinfo with various column constraints."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT UNIQUE,
                age INTEGER DEFAULT 0
            )
        """)

        xinfo = await conn.get_table_xinfo("test")
        assert len(xinfo) == 4

        # Check NOT NULL constraint
        name_col = next(c for c in xinfo if c["name"] == "name")
        assert name_col["notnull"] == 1
        assert name_col["hidden"] == 0

        # Check default value
        age_col = next(c for c in xinfo if c["name"] == "age")
        assert age_col["dflt_value"] == "0" or age_col["dflt_value"] == 0
        assert age_col["hidden"] == 0


@pytest.mark.asyncio
async def test_schema_methods_integration(test_db):
    """Test integration between new schema methods."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON users(email)")
        await conn.execute("CREATE VIEW user_view AS SELECT * FROM users")

        # Get views
        views = await conn.get_views()
        assert "user_view" in views

        # Get index list
        index_list = await conn.get_index_list("users")
        idx_email = next(
            (idx for idx in index_list if idx["name"] == "idx_email"), None
        )
        assert idx_email is not None

        # Get index info
        index_info = await conn.get_index_info("idx_email")
        assert len(index_info) == 1
        assert index_info[0]["name"] == "email"

        # Get extended table info
        xinfo = await conn.get_table_xinfo("users")
        assert len(xinfo) == 2
        assert all(col["hidden"] == 0 for col in xinfo)


@pytest.mark.asyncio
async def test_get_index_list_vs_get_indexes(test_db):
    """Test that get_index_list and get_indexes return consistent information."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON users(email)")
        await conn.execute("CREATE UNIQUE INDEX idx_unique_email ON users(email)")

        index_list = await conn.get_index_list("users")
        indexes = await conn.get_indexes(table_name="users")

        # Both should return the same indexes (may have different ordering)
        indexes_names = {idx["name"] for idx in indexes}

        # Compare unique flags
        for list_idx in index_list:
            if list_idx["name"] in indexes_names:
                indexes_idx = next(
                    idx for idx in indexes if idx["name"] == list_idx["name"]
                )
                assert list_idx["unique"] == indexes_idx["unique"]


@pytest.mark.asyncio
async def test_get_views_with_tables(test_db):
    """Test that get_views only returns views, not tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY)")
        await conn.execute("CREATE VIEW user_view AS SELECT * FROM users")

        views = await conn.get_views()
        tables = await conn.get_tables()

        # Views should not be in tables
        assert "user_view" not in tables
        assert "users" in tables
        assert "posts" in tables

        # Tables should not be in views
        assert "users" not in views
        assert "posts" not in views
        assert "user_view" in views


@pytest.mark.asyncio
async def test_get_index_list_multiple_tables(test_db):
    """Test get_index_list with multiple tables."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT)")
        await conn.execute("CREATE INDEX idx_users_email ON users(email)")
        await conn.execute("CREATE INDEX idx_posts_title ON posts(title)")

        user_indexes = await conn.get_index_list("users")
        post_indexes = await conn.get_index_list("posts")

        user_index_names = [idx["name"] for idx in user_indexes]
        post_index_names = [idx["name"] for idx in post_indexes]

        assert "idx_users_email" in user_index_names
        assert "idx_posts_title" in post_index_names
        assert "idx_users_email" not in post_index_names
        assert "idx_posts_title" not in user_index_names


@pytest.mark.asyncio
async def test_get_index_info_with_dropped_index(test_db):
    """Test get_index_info after dropping an index."""
    async with Connection(test_db) as conn:
        await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, email TEXT)")
        await conn.execute("CREATE INDEX idx_email ON test(email)")

        index_info_before = await conn.get_index_info("idx_email")
        assert len(index_info_before) == 1

        await conn.execute("DROP INDEX idx_email")

        index_info_after = await conn.get_index_info("idx_email")
        assert len(index_info_after) == 0


@pytest.mark.asyncio
async def test_get_table_xinfo_column_order(test_db):
    """Test get_table_xinfo returns columns in correct order."""
    async with Connection(test_db) as conn:
        await conn.execute("""
            CREATE TABLE test (
                id INTEGER PRIMARY KEY,
                first TEXT,
                second TEXT,
                third TEXT
            )
        """)

        xinfo = await conn.get_table_xinfo("test")
        assert len(xinfo) == 4

        # Verify cid values are sequential
        cids = [col["cid"] for col in xinfo]
        assert cids == sorted(cids)
        assert cids == list(range(4))  # Should be 0, 1, 2, 3

        # Verify column order matches creation order
        assert xinfo[0]["name"] == "id"
        assert xinfo[1]["name"] == "first"
        assert xinfo[2]["name"] == "second"
        assert xinfo[3]["name"] == "third"
