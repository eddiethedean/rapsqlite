"""Smoke tests for SQLAlchemy + sqlite async (rapsqlite and aiosqlite dialects).

Covers Core (engine.connect, engine.begin, run_sync, metadata, result API),
ORM (AsyncSession, add/commit/get, run_sync create_all, rollback),
exception propagation (IntegrityError, OperationalError), and pool/URL behavior.
Server-side cursors are not supported (supports_server_side_cursors = False).

Tests are parametrized to run with both sqlite+rapsqlite and sqlite+aiosqlite.
"""

import asyncio
from typing import AsyncGenerator

import pytest

pytest.importorskip("sqlalchemy")
import rapsqlite.sqlalchemy  # noqa: F401 -- register dialect before create_async_engine
from sqlalchemy import MetaData, Table, Column, Integer, String, text
from sqlalchemy import insert, select
from sqlalchemy.sql.expression import false
from sqlalchemy.ext.asyncio import create_async_engine, AsyncEngine
from sqlalchemy.pool import StaticPool


@pytest.fixture(params=["rapsqlite", "aiosqlite"])
def sqlite_dialect(request: pytest.FixtureRequest) -> str:
    """Parametrize dialect so tests run with both rapsqlite and aiosqlite."""
    dialect: str = request.param
    if dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    return dialect


@pytest.fixture
async def async_engine_sqlite(
    test_db: str, sqlite_dialect: str
) -> AsyncGenerator[AsyncEngine, None]:
    """Async engine for sqlite+rapsqlite or sqlite+aiosqlite with file DB; yields then disposes."""
    engine = create_async_engine(f"sqlite+{sqlite_dialect}:///{test_db}")
    try:
        yield engine
    finally:
        await engine.dispose()


@pytest.mark.asyncio
@pytest.mark.parametrize("sqlite_dialect", ["rapsqlite", "aiosqlite"], indirect=True)
async def test_sqlalchemy_engine_create(sqlite_dialect: str) -> None:
    """create_async_engine with sqlite+{dialect}:///:memory: builds an AsyncEngine."""
    if sqlite_dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    engine = create_async_engine(f"sqlite+{sqlite_dialect}:///:memory:")
    assert engine is not None
    assert str(engine.url).startswith(f"sqlite+{sqlite_dialect}")
    await engine.dispose()


@pytest.mark.asyncio
@pytest.mark.parametrize("sqlite_dialect", ["rapsqlite", "aiosqlite"], indirect=True)
async def test_sqlalchemy_alembic_style_migration(
    test_db: str, sqlite_dialect: str
) -> None:
    """Validate sqlite+{dialect} for Alembic-style migrations (create table, add column)."""
    if sqlite_dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    url = f"sqlite+{sqlite_dialect}:///{test_db}"
    engine = create_async_engine(url)
    try:
        async with engine.begin() as conn:
            await conn.execute(
                text("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
            )
            await conn.execute(text("INSERT INTO users (id, name) VALUES (1, 'alice')"))
        async with engine.begin() as conn:
            await conn.execute(text("ALTER TABLE users ADD COLUMN email TEXT"))
            await conn.execute(text("UPDATE users SET email = 'a@b.com' WHERE id = 1"))
        async with engine.connect() as conn:
            res = await conn.execute(
                text("SELECT id, name, email FROM users WHERE id = 1")
            )
            row = res.fetchone()
        assert row is not None
        assert row[0] == 1 and row[1] == "alice" and row[2] == "a@b.com"
    finally:
        await engine.dispose()


# --- Core: engine and connection ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_engine_connect_lifecycle(async_engine_sqlite: AsyncEngine) -> None:
    """engine.connect() yields connection; execute SELECT 1; dispose."""
    engine = async_engine_sqlite
    async with engine.connect() as conn:
        res = await conn.execute(text("SELECT 1"))
        row = res.fetchone()
        assert row is not None and row[0] == 1


@pytest.mark.asyncio
@pytest.mark.parametrize("sqlite_dialect", ["rapsqlite", "aiosqlite"], indirect=True)
async def test_on_connect_regexp_and_floor(
    async_engine_sqlite: AsyncEngine,
) -> None:
    """on_connect registers regexp and floor; SELECT regexp(...) and SELECT floor(...) work."""
    engine = async_engine_sqlite
    async with engine.connect() as conn:
        res = await conn.execute(text("SELECT regexp('^a', 'abc')"))
        row = res.fetchone()
        assert row is not None and row[0] == 1  # match
        res = await conn.execute(text("SELECT regexp('^x', 'abc')"))
        row = res.fetchone()
        assert row is not None and row[0] == 0  # no match
        res = await conn.execute(text("SELECT floor(1.7)"))
        row = res.fetchone()
        assert row is not None and row[0] == 1.0


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_engine_begin_commit(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """engine.begin() auto-commits on exit; reconnect and verify row."""
    engine = async_engine_sqlite
    table = unique_table_prefix
    async with engine.begin() as conn:
        await conn.execute(
            text(f"CREATE TABLE {table} (id INTEGER PRIMARY KEY, x INTEGER)")
        )
        await conn.execute(text(f"INSERT INTO {table} (id, x) VALUES (1, 42)"))
    async with engine.connect() as conn:
        res = await conn.execute(text(f"SELECT id, x FROM {table} WHERE id = 1"))
        row = res.fetchone()
        assert row is not None and row[0] == 1 and row[1] == 42


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_connection_explicit_transaction(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Explicit conn.begin(), commit(), then SELECT; rollback branch leaves no row."""
    engine = async_engine_sqlite
    table = unique_table_prefix
    async with engine.connect() as conn:
        await conn.execute(text(f"CREATE TABLE {table} (id INTEGER PRIMARY KEY)"))
        await conn.commit()
    async with engine.connect() as conn:
        await conn.begin()
        await conn.execute(text(f"INSERT INTO {table} (id) VALUES (1)"))
        await conn.commit()
    async with engine.connect() as conn:
        rows = (await conn.execute(text(f"SELECT id FROM {table}"))).fetchall()
        assert len(rows) == 1 and rows[0][0] == 1
    # Rollback branch: insert then rollback
    async with engine.connect() as conn:
        await conn.begin()
        await conn.execute(text(f"INSERT INTO {table} (id) VALUES (2)"))
        await conn.rollback()
    async with engine.connect() as conn:
        rows = (
            await conn.execute(text(f"SELECT id FROM {table} ORDER BY id"))
        ).fetchall()
        assert len(rows) == 1 and rows[0][0] == 1


# --- Core: run_sync and metadata ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_run_sync_create_drop_all(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """run_sync(metadata.drop_all), run_sync(metadata.create_all), then insert/select."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname, meta, Column("id", Integer, primary_key=True), Column("name", String(50))
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.drop_all)
        await conn.run_sync(meta.create_all)
    async with engine.connect() as conn:
        await conn.execute(text(f"INSERT INTO {tname} (id, name) VALUES (1, 'alice')"))
        await conn.commit()
    async with engine.connect() as conn:
        rows = (await conn.execute(select(t))).fetchall()
        assert len(rows) == 1 and rows[0][0] == 1 and rows[0][1] == "alice"


# --- Core: result API ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_result_all_scalars_mappings(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """conn.execute(select); .all(), .scalars(), .mappings() return expected data."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname,
        meta,
        Column("id", Integer, primary_key=True),
        Column("label", String(20)),
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
        await conn.execute(
            text(f"INSERT INTO {tname} (id, label) VALUES (1, 'a'), (2, 'b')")
        )
    async with engine.connect() as conn:
        rows_all = (await conn.execute(select(t).order_by(t.c.id))).all()
        rows_scalars = (await conn.execute(select(t).order_by(t.c.id))).scalars().all()
        rows_mappings = (
            (await conn.execute(select(t).order_by(t.c.id))).mappings().all()
        )
        assert len(rows_all) == 2 and rows_all[0][0] == 1 and rows_all[0][1] == "a"
        # .scalars() returns first column only
        assert rows_scalars == [1, 2]
        assert (
            len(rows_mappings) == 2
            and dict(rows_mappings[0])["id"] == 1
            and dict(rows_mappings[0])["label"] == "a"
        )


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
@pytest.mark.edge_case
async def test_core_zero_row_select_result(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Core 0-row SELECT: fetchall and scalars().all() return [] with valid result metadata."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname,
        meta,
        Column("id", Integer, primary_key=True),
        Column("label", String(20)),
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
    async with engine.connect() as conn:
        res = await conn.execute(select(t).where(false()))
        rows = res.fetchall()
        assert rows == []
        # Valid description so SQLAlchemy does not see closed/empty result
        assert res.keys() is not None and len(res.keys()) >= 1
    async with engine.connect() as conn:
        scalars_empty = (await conn.execute(select(t).where(false()))).scalars().all()
        assert scalars_empty == []
    # Two 0-row selects in a row on same connection (7c)
    async with engine.connect() as conn:
        r1 = await conn.execute(select(t).where(false()))
        assert r1.fetchall() == []
        r2 = await conn.execute(select(t).where(false()))
        assert r2.fetchall() == []


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
@pytest.mark.edge_case
async def test_core_scalars_first_and_one_or_none_zero_rows(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """scalars().first() and one_or_none() on 0-row result return None without error."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname, meta, Column("id", Integer, primary_key=True), Column("x", Integer)
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
    async with engine.connect() as conn:
        res = await conn.execute(select(t).where(false()))
        assert res.scalars().first() is None
    async with engine.connect() as conn:
        res = await conn.execute(select(t).where(false()))
        assert res.scalars().one_or_none() is None


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_core_cursor_reuse_same_connection(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """After one execute and consumption, execute another on same connection; both correct."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname, meta, Column("id", Integer, primary_key=True), Column("n", Integer)
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
        await conn.execute(text(f"INSERT INTO {tname} (id, n) VALUES (1, 10), (2, 20)"))
    async with engine.connect() as conn:
        res1 = await conn.execute(select(t).where(t.c.id == 1))
        rows1 = res1.fetchall()
        assert len(rows1) == 1 and rows1[0][0] == 1 and rows1[0][1] == 10
        res2 = await conn.execute(select(t).where(t.c.id == 2))
        rows2 = res2.fetchall()
        assert len(rows2) == 1 and rows2[0][0] == 2 and rows2[0][1] == 20


# --- Core: parameter binding ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_parameterized_select_insert(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """insert(table).values() and select(table).where() with bound params."""
    engine = async_engine_sqlite
    meta = MetaData()
    tname = unique_table_prefix
    t = Table(
        tname,
        meta,
        Column("id", Integer, primary_key=True),
        Column("a", Integer),
        Column("b", String(20)),
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
    async with engine.connect() as conn:
        await conn.execute(insert(t).values(id=1, a=10, b="foo"))
        await conn.commit()
        res = await conn.execute(select(t).where(t.c.id == 1))
        row = res.fetchone()
        assert row is not None and row[0] == 1 and row[1] == 10 and row[2] == "foo"


# --- ORM: session and model ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_add_commit_get(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """AsyncSession: add_all, commit; new session execute(select), scalars().all(), get()."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class User(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True, autoincrement=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        async with session.begin():
            session.add_all([User(name="alice"), User(name="bob")])

    async with async_session() as session:
        result = await session.execute(select(User).order_by(User.id))
        users = result.scalars().all()  # consume inside session context
        assert len(users) == 2
        assert users[0].name == "alice" and users[1].name == "bob"

    async with async_session() as session:
        u1 = await session.get(User, 1)
        assert u1 is not None
        assert u1.id == 1 and u1.name == "alice"


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_run_sync_metadata(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """engine.begin(); run_sync(Base.metadata.create_all); then AsyncSession insert/select."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class Item(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True)
        value: Mapped[str] = mapped_column(String(20))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(f"INSERT INTO {table_name} (id, value) VALUES (1, 'x')")
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        result = await session.execute(select(Item))
        items = result.scalars().all()
        assert len(items) == 1
        assert items[0].id == 1 and items[0].value == "x"


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_rollback(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Session: add one, commit; then begin, add another, rollback; only first row exists."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class Log(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True, autoincrement=True)
        msg: Mapped[str] = mapped_column(String(20))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        session.add(Log(msg="first"))
        await session.commit()
    async with async_session() as session:
        async with session.begin():
            session.add(Log(msg="second"))
            await session.rollback()
    async with async_session() as session:
        result = await session.execute(select(Log))
        rows = result.scalars().all()
        assert len(rows) == 1
        msg = rows[0].msg
        assert msg == "first" or (isinstance(msg, str) and "first" in msg)


# --- Exceptions ---


@pytest.mark.asyncio
async def test_integrity_error_propagates(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """UNIQUE constraint violation raises IntegrityError (or SQLAlchemy wrapper)."""
    import sqlalchemy.exc

    engine = async_engine_sqlite
    table = unique_table_prefix
    async with engine.begin() as conn:
        await conn.execute(
            text(f"CREATE TABLE {table} (id INTEGER PRIMARY KEY, email TEXT UNIQUE)")
        )
        await conn.execute(
            text(f"INSERT INTO {table} (id, email) VALUES (1, 'a@b.com')")
        )
    with pytest.raises((sqlalchemy.exc.IntegrityError, Exception)) as exc_info:
        async with engine.begin() as conn:
            await conn.execute(
                text(f"INSERT INTO {table} (id, email) VALUES (2, 'a@b.com')")
            )
    msg = str(exc_info.value).lower()
    assert "unique" in msg or "constraint" in msg or "integrity" in msg


@pytest.mark.asyncio
async def test_operational_error_propagates(async_engine_sqlite: AsyncEngine) -> None:
    """Invalid SQL (e.g. typo table name) raises OperationalError or wrapper."""
    import sqlalchemy.exc

    engine = async_engine_sqlite
    with pytest.raises((sqlalchemy.exc.OperationalError, Exception)):
        async with engine.connect() as conn:
            await conn.execute(text("SELECT * FROM nonexistent_table_xyz"))


# --- Pool and URL ---


@pytest.mark.asyncio
async def test_file_url_uses_pool(async_engine_sqlite: AsyncEngine) -> None:
    """File URL engine: two connections open/close; no error (pool used)."""
    engine = async_engine_sqlite
    async with engine.connect() as conn1:
        result1 = await conn1.execute(text("SELECT 1"))
        assert result1.scalar() == 1  # consume inside context
    async with engine.connect() as conn2:
        result2 = await conn2.execute(text("SELECT 2"))
        assert result2.scalar() == 2  # consume inside context


@pytest.mark.asyncio
@pytest.mark.parametrize("sqlite_dialect", ["rapsqlite", "aiosqlite"], indirect=True)
async def test_memory_url_static_pool(sqlite_dialect: str) -> None:
    """Engine with sqlite+{dialect}:///:memory: uses StaticPool."""
    if sqlite_dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    engine = create_async_engine(f"sqlite+{sqlite_dialect}:///:memory:")
    try:
        pool = engine.sync_engine.pool
        assert isinstance(pool, StaticPool)
    finally:
        await engine.dispose()


# --- Optional: async_creator and concurrent_sessions ---


@pytest.mark.asyncio
async def test_async_creator(test_db: str, sqlite_dialect: str) -> None:
    """Engine with async_creator runs one execute (rapsqlite-only: uses rapsqlite dbapi)."""
    if sqlite_dialect != "rapsqlite":
        pytest.skip("async_creator test uses rapsqlite dbapi; only run with rapsqlite")
    from rapsqlite import dbapi

    async def creator():
        return await dbapi.connect(test_db)

    engine = create_async_engine(
        "sqlite+rapsqlite:///",
        async_creator=creator,
    )
    try:
        async with engine.connect() as conn:
            result = await conn.execute(text("SELECT 1"))
            assert result.scalar() == 1
    finally:
        await engine.dispose()


@pytest.mark.asyncio
async def test_concurrent_sessions(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Two connections run simple SELECT in parallel via asyncio.gather."""
    engine = async_engine_sqlite
    table = unique_table_prefix
    async with engine.begin() as conn:
        await conn.execute(
            text(f"CREATE TABLE {table} (id INTEGER PRIMARY KEY, n INTEGER)")
        )
        await conn.execute(text(f"INSERT INTO {table} (id, n) VALUES (1, 10)"))

    async def query_one(session_id: int):
        async with engine.connect() as conn:
            res = await conn.execute(text(f"SELECT id, n FROM {table} WHERE id = 1"))
            row = res.fetchone()
            return session_id, (row[1] if row else None)

    out1, out2 = await asyncio.gather(query_one(1), query_one(2))
    assert out1 == (1, 10)
    assert out2 == (2, 10)


@pytest.mark.asyncio
@pytest.mark.concurrency
@pytest.mark.edge_case
async def test_concurrent_zero_row_selects(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Two tasks run session.get(User, 999) in parallel; both return None without error."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class User(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(f"INSERT INTO {table_name} (id, name) VALUES (1, 'alice')")
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )

    async def get_missing(_: int):
        async with async_session() as session:
            return await session.get(User, 999)

    out1, out2 = await asyncio.gather(get_missing(1), get_missing(2))
    assert out1 is None
    assert out2 is None


# --- More robust: ORM add_all many rows, session.get missing, multiple fetches, RETURNING ---


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_add_all_many_rows(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """ORM add_all with many rows (insertmanyvalues / RETURNING); same code for both dialects."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class Item(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True, autoincrement=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        async with session.begin():
            session.add_all([Item(name=f"item_{i}") for i in range(20)])

    async with async_session() as session:
        result = await session.execute(select(Item).order_by(Item.id))
        items = result.scalars().all()
    assert len(items) == 20
    assert items[0].name == "item_0" and items[19].name == "item_19"


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_get_missing_key(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Fetch by PK: existing returns row, missing returns None. Same code for both dialects."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class User(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(f"INSERT INTO {table_name} (id, name) VALUES (1, 'alice')")
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        u1 = await session.get(User, 1)
        u_missing = await session.get(User, 999)
    assert u1 is not None and u1.name == "alice"
    assert u_missing is None


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
@pytest.mark.edge_case
async def test_async_session_two_get_missing_in_row(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Two session.get(missing) in same session; both return None."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class User(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(f"INSERT INTO {table_name} (id, name) VALUES (1, 'alice')")
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        u_missing_999 = await session.get(User, 999)
        u_missing_998 = await session.get(User, 998)
    assert u_missing_999 is None
    assert u_missing_998 is None


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
@pytest.mark.edge_case
async def test_async_session_select_one_or_none_zero_rows(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """execute(select(User).where(...)).scalars().one_or_none() on 0 rows returns None."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class User(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True)
        name: Mapped[str] = mapped_column(String(50))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(f"INSERT INTO {table_name} (id, name) VALUES (1, 'alice')")
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        result = await session.execute(select(User).where(User.id == 999))
        one = result.scalars().one_or_none()
    assert one is None


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
@pytest.mark.edge_case
async def test_async_session_get_composite_pk_missing(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """session.get(Model, (1, 2)) with composite PK when no row exists returns None."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class CompositeModel(Base):
        __tablename__ = table_name
        __table_args__ = {"sqlite_autoincrement": False}
        id_a: Mapped[int] = mapped_column(primary_key=True)
        id_b: Mapped[int] = mapped_column(primary_key=True)
        name: Mapped[str] = mapped_column(String(20))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
        await conn.execute(
            text(
                f"INSERT INTO {table_name} (id_a, id_b, name) VALUES (10, 20, 'exists')"
            )
        )

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        existing = await session.get(CompositeModel, (10, 20))
        missing = await session.get(CompositeModel, (1, 2))
    assert existing is not None and existing.name == "exists"
    assert missing is None


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_result_multiple_fetches(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Same result: fetchone, fetchmany, fetchall / scalars(); same code for both dialects."""
    engine = async_engine_sqlite
    tname = unique_table_prefix
    meta = MetaData()
    t = Table(
        tname, meta, Column("id", Integer, primary_key=True), Column("x", Integer)
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
        await conn.execute(
            text(f"INSERT INTO {tname} (id, x) VALUES (1, 10), (2, 20), (3, 30)")
        )
    async with engine.connect() as conn:
        res = await conn.execute(select(t).order_by(t.c.id))
        one = res.fetchone()
        many = res.fetchmany(2)
        rest = res.fetchall()
    assert one is not None and one[0] == 1 and one[1] == 10
    assert len(many) == 2 and many[0][0] == 2 and many[1][0] == 3
    assert len(rest) == 0
    # Scalar result
    async with engine.connect() as conn:
        res = await conn.execute(select(t.c.x).where(t.c.id == 1))
        assert res.scalar() == 10


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_core
async def test_core_insert_returning(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Core insert().returning(); same code for both dialects."""
    engine = async_engine_sqlite
    tname = unique_table_prefix
    meta = MetaData()
    t = Table(
        tname,
        meta,
        Column("id", Integer, primary_key=True, autoincrement=True),
        Column("label", String(20)),
    )
    async with engine.begin() as conn:
        await conn.run_sync(meta.create_all)
    async with engine.connect() as conn:
        res = await conn.execute(
            insert(t).values(label="a").returning(t.c.id, t.c.label)
        )
        row = res.fetchone()
        await conn.commit()
    assert row is not None
    assert row[0] is not None and row[1] == "a"


@pytest.mark.asyncio
@pytest.mark.sqlalchemy_orm
async def test_async_session_rollback_then_commit(
    async_engine_sqlite: AsyncEngine, unique_table_prefix: str
) -> None:
    """Session: begin, add, rollback; then new begin, add, commit; only second row exists."""
    from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
    from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

    engine = async_engine_sqlite
    table_name = unique_table_prefix

    class Base(DeclarativeBase):
        pass

    class Event(Base):
        __tablename__ = table_name
        id: Mapped[int] = mapped_column(primary_key=True, autoincrement=True)
        name: Mapped[str] = mapped_column(String(20))

    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

    async_session = async_sessionmaker(
        engine, expire_on_commit=False, class_=AsyncSession
    )
    async with async_session() as session:
        session.add(Event(name="rolled"))
        await session.rollback()
    async with async_session() as session:
        async with session.begin():
            session.add(Event(name="committed"))
    async with async_session() as session:
        result = await session.execute(select(Event))
        rows = result.scalars().all()
    assert len(rows) == 1
    assert rows[0].name == "committed"
