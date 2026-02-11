"""Alembic tests for sqlite+rapsqlite and sqlite+aiosqlite.

Runs alembic init (async template), hand-written revisions, upgrade/downgrade,
and verifies database state. Tests are parametrized with aiosqlite first so we
validate behavior against a known-good dialect before rapsqlite.
"""

import os
import subprocess
import sys

import pytest

from conftest import cleanup_db

pytestmark = [pytest.mark.integration]

# Run aiosqlite first in parametrized tests to validate test logic
DIALECT_ORDER = ["aiosqlite", "rapsqlite"]


def _alembic_ini_content(script_location: str, db_url: str) -> str:
    """Minimal alembic.ini for async migrations with sqlite+rapsqlite."""
    return f"""[alembic]
script_location = {script_location}
sqlalchemy.url = {db_url}

[loggers]
keys = root,sqlalchemy,alembic

[handlers]
keys = console

[formatters]
keys = generic

[logger_root]
level = WARN
handlers = console
qualname =

[logger_sqlalchemy]
level = WARN
handlers =
qualname = sqlalchemy.engine

[logger_alembic]
level = INFO
handlers =
qualname = alembic

[handler_console]
class = StreamHandler
args = (sys.stderr,)
level = NOTSET
formatter = generic

[formatter_generic]
format = %(levelname)-5.5s [%(name)s] %(message)s
datefmt = %H:%M:%S
"""


def _revision_create_table() -> str:
    """Revision that creates alembic_test table and inserts one row."""
    return '''"""create alembic_test table

Revision ID: 001_rapsqlite
Revises:
Create Date: 2025-01-01 00:00:00

"""
from alembic import op
import sqlalchemy as sa

revision = "001_rapsqlite"
down_revision = None
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "alembic_test",
        sa.Column("id", sa.Integer(), nullable=False),
        sa.Column("name", sa.String(64), nullable=True),
        sa.PrimaryKeyConstraint("id"),
    )
    op.execute("INSERT INTO alembic_test (id, name) VALUES (1, 'migrated')")


def downgrade() -> None:
    op.drop_table("alembic_test")
'''


def _revision_add_column() -> str:
    """Revision 002: add email column to alembic_test."""
    return '''"""add email to alembic_test

Revision ID: 002_rapsqlite
Revises: 001_rapsqlite
Create Date: 2025-01-01 00:00:01

"""
from alembic import op
import sqlalchemy as sa

revision = "002_rapsqlite"
down_revision = "001_rapsqlite"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.add_column("alembic_test", sa.Column("email", sa.String(128), nullable=True))
    op.execute("UPDATE alembic_test SET email = 'a@b.com' WHERE id = 1")


def downgrade() -> None:
    op.drop_column("alembic_test", "email")
'''


def _revision_second_table() -> str:
    """Revision 003: create alembic_extra table."""
    return '''"""create alembic_extra table

Revision ID: 003_rapsqlite
Revises: 002_rapsqlite
Create Date: 2025-01-01 00:00:02

"""
from alembic import op
import sqlalchemy as sa

revision = "003_rapsqlite"
down_revision = "002_rapsqlite"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "alembic_extra",
        sa.Column("id", sa.Integer(), nullable=False),
        sa.Column("value", sa.String(64), nullable=True),
        sa.PrimaryKeyConstraint("id"),
    )
    op.execute("INSERT INTO alembic_extra (id, value) VALUES (1, 'extra')")


def downgrade() -> None:
    op.drop_table("alembic_extra")
'''


def _run_alembic(
    cwd: str, *args: str, timeout: int = 30
) -> subprocess.CompletedProcess:
    """Run alembic in cwd with given args. Returns CompletedProcess."""
    return subprocess.run(
        [sys.executable, "-m", "alembic", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def _db_url(dialect: str, db_path_str: str) -> str:
    """Build sqlite+<dialect> URL for the given absolute path."""
    if os.name == "nt":
        path_part = db_path_str.replace(os.sep, "/")
    else:
        path_part = db_path_str
    return f"sqlite+{dialect}:///{path_part}"


@pytest.fixture
def alembic_root(tmp_path):
    """Create temp dir, run alembic init -t async, return path to root (tmp_path)."""
    root = tmp_path
    result = subprocess.run(
        [sys.executable, "-m", "alembic", "init", "-t", "async", "alembic"],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, f"alembic init failed: {result.stderr}"
    assert (root / "alembic" / "env.py").exists()
    assert (root / "alembic" / "versions").exists()
    return root


def test_alembic_upgrade_head_with_rapsqlite(alembic_root, tmp_path):
    """Run alembic upgrade head with sqlite+rapsqlite and verify table and row."""
    pytest.importorskip("alembic")
    from rapsqlite import connect

    root = alembic_root
    db_path = root / "migrations.db"
    # Absolute path: Unix needs four slashes (scheme + path start), Windows uses three
    db_path_str = str(db_path.resolve())
    if os.name == "nt":
        db_url = f"sqlite+rapsqlite:///{db_path_str.replace(os.sep, '/')}"
    else:
        db_url = f"sqlite+rapsqlite:///{db_path_str}"

    # Write alembic.ini with rapsqlite URL
    ini = root / "alembic.ini"
    ini.write_text(_alembic_ini_content("alembic", db_url), encoding="utf-8")

    # Ensure DB file exists so driver can open it (some backends don't create on first connect)
    db_path.touch()

    # Write one revision
    versions_dir = root / "alembic" / "versions"
    rev_file = versions_dir / "001_create_alembic_test.py"
    rev_file.write_text(_revision_create_table(), encoding="utf-8")

    # Run upgrade head
    result = subprocess.run(
        [sys.executable, "-m", "alembic", "upgrade", "head"],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, f"alembic upgrade head failed: {result.stderr}"

    # Verify with rapsqlite
    async def check():
        async with connect(str(db_path)) as conn:
            rows = await conn.fetch_all("SELECT id, name FROM alembic_test")
            assert rows == [[1, "migrated"]]

    import asyncio

    asyncio.run(check())

    cleanup_db(str(db_path))


@pytest.mark.parametrize("dialect", DIALECT_ORDER)
def test_alembic_upgrade_then_downgrade_base(alembic_root, tmp_path, dialect):
    """Upgrade head then downgrade base. aiosqlite first to validate test."""
    pytest.importorskip("alembic")
    if dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    from rapsqlite import connect

    root = alembic_root
    db_path = root / "migrations.db"
    db_path.touch()
    db_url = _db_url(dialect, str(db_path.resolve()))
    (root / "alembic.ini").write_text(
        _alembic_ini_content("alembic", db_url), encoding="utf-8"
    )
    (root / "alembic" / "versions" / "001_create_alembic_test.py").write_text(
        _revision_create_table(), encoding="utf-8"
    )

    r = _run_alembic(str(root), "upgrade", "head")
    assert r.returncode == 0, f"alembic upgrade head ({dialect}): {r.stderr}"

    r = _run_alembic(str(root), "downgrade", "base")
    assert r.returncode == 0, f"alembic downgrade base ({dialect}): {r.stderr}"

    async def check():
        async with connect(str(db_path)) as conn:
            with pytest.raises(Exception):
                await conn.fetch_all("SELECT * FROM alembic_test")

    import asyncio

    asyncio.run(check())
    cleanup_db(str(db_path))


def _setup_three_revisions(root: object, db_url: str, db_path: object) -> None:
    """Write alembic.ini and three revision files; touch db."""
    root = root  # Path
    (root / "alembic.ini").write_text(
        _alembic_ini_content("alembic", db_url), encoding="utf-8"
    )
    db_path.touch()
    versions = root / "alembic" / "versions"
    (versions / "001_create_alembic_test.py").write_text(
        _revision_create_table(), encoding="utf-8"
    )
    (versions / "002_add_email.py").write_text(_revision_add_column(), encoding="utf-8")
    (versions / "003_alembic_extra.py").write_text(
        _revision_second_table(), encoding="utf-8"
    )


@pytest.mark.parametrize("dialect", DIALECT_ORDER)
def test_alembic_multiple_revisions_upgrade_downgrade_stepwise(
    alembic_root, tmp_path, dialect
):
    """Three revisions: upgrade head, verify; downgrade stepwise to base; upgrade head again."""
    pytest.importorskip("alembic")
    if dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    from rapsqlite import connect

    root = alembic_root
    db_path = root / "migrations.db"
    db_url = _db_url(dialect, str(db_path.resolve()))
    _setup_three_revisions(root, db_url, db_path)

    r = _run_alembic(str(root), "upgrade", "head")
    assert r.returncode == 0, f"upgrade head ({dialect}): {r.stderr}"

    async def verify_head():
        async with connect(str(db_path)) as conn:
            r1 = await conn.fetch_all("SELECT id, name, email FROM alembic_test")
            r2 = await conn.fetch_all("SELECT id, value FROM alembic_extra")
            assert r1 == [[1, "migrated", "a@b.com"]]
            assert r2 == [[1, "extra"]]

    import asyncio

    asyncio.run(verify_head())

    r = _run_alembic(str(root), "downgrade", "-1")
    assert r.returncode == 0, f"downgrade -1 ({dialect}): {r.stderr}"

    async def verify_after_d1():
        async with connect(str(db_path)) as conn:
            await conn.fetch_all("SELECT id, name, email FROM alembic_test")
            with pytest.raises(Exception):
                await conn.fetch_all("SELECT * FROM alembic_extra")

    asyncio.run(verify_after_d1())

    r = _run_alembic(str(root), "downgrade", "-1")
    assert r.returncode == 0, f"downgrade -1 again ({dialect}): {r.stderr}"

    async def verify_after_d2():
        async with connect(str(db_path)) as conn:
            rows = await conn.fetch_all("SELECT id, name FROM alembic_test")
            assert rows == [[1, "migrated"]]

    asyncio.run(verify_after_d2())

    r = _run_alembic(str(root), "downgrade", "base")
    assert r.returncode == 0, f"downgrade base ({dialect}): {r.stderr}"

    async def verify_base():
        async with connect(str(db_path)) as conn:
            with pytest.raises(Exception):
                await conn.fetch_all("SELECT * FROM alembic_test")

    asyncio.run(verify_base())

    r = _run_alembic(str(root), "upgrade", "head")
    assert r.returncode == 0, f"upgrade head again ({dialect}): {r.stderr}"
    asyncio.run(verify_head())

    cleanup_db(str(db_path))


@pytest.mark.parametrize("dialect", DIALECT_ORDER)
def test_alembic_upgrade_to_revision_then_head_then_downgrade_steps(
    alembic_root, tmp_path, dialect
):
    """Upgrade to 002 (has email), verify; upgrade to head (has extra table); downgrade to 001, 002, base."""
    pytest.importorskip("alembic")
    if dialect == "aiosqlite":
        pytest.importorskip("aiosqlite")
    from rapsqlite import connect

    root = alembic_root
    db_path = root / "migrations.db"
    db_url = _db_url(dialect, str(db_path.resolve()))
    _setup_three_revisions(root, db_url, db_path)

    r = _run_alembic(str(root), "upgrade", "002_rapsqlite")
    assert r.returncode == 0, f"upgrade 002 ({dialect}): {r.stderr}"

    async def verify_002():
        async with connect(str(db_path)) as conn:
            rows = await conn.fetch_all("SELECT id, name, email FROM alembic_test")
            assert rows == [[1, "migrated", "a@b.com"]]
            with pytest.raises(Exception):
                await conn.fetch_all("SELECT * FROM alembic_extra")

    import asyncio

    asyncio.run(verify_002())

    r = _run_alembic(str(root), "upgrade", "head")
    assert r.returncode == 0, f"upgrade head ({dialect}): {r.stderr}"

    async def verify_head():
        async with connect(str(db_path)) as conn:
            r1 = await conn.fetch_all("SELECT id, name, email FROM alembic_test")
            r2 = await conn.fetch_all("SELECT id, value FROM alembic_extra")
            assert r1 == [[1, "migrated", "a@b.com"]]
            assert r2 == [[1, "extra"]]

    asyncio.run(verify_head())

    r = _run_alembic(str(root), "downgrade", "001_rapsqlite")
    assert r.returncode == 0, f"downgrade to 001 ({dialect}): {r.stderr}"

    async def verify_001():
        async with connect(str(db_path)) as conn:
            rows = await conn.fetch_all("SELECT id, name FROM alembic_test")
            assert rows == [[1, "migrated"]]

    asyncio.run(verify_001())

    r = _run_alembic(str(root), "downgrade", "base")
    assert r.returncode == 0, f"downgrade base ({dialect}): {r.stderr}"

    async def verify_base():
        async with connect(str(db_path)) as conn:
            with pytest.raises(Exception):
                await conn.fetch_all("SELECT * FROM alembic_test")

    asyncio.run(verify_base())
    cleanup_db(str(db_path))
