"""Shared pytest fixtures and utilities for rapsqlite tests."""

import hashlib
import os
import sys
import tempfile
import pytest
from typing import Any, AsyncGenerator, Generator

# Windows-specific asyncio event loop policy fix
# Windows uses ProactorEventLoop by default, which has known issues with pytest-asyncio
# Setting SelectorEventLoopPolicy prevents event loop closure errors and hangs
if sys.platform == "win32":
    import asyncio

    # Use SelectorEventLoop on Windows instead of ProactorEventLoop
    # This prevents "Event loop is closed" errors and test hangs
    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())


def cleanup_db(test_db: str) -> None:
    """Helper to clean up database file.

    Args:
        test_db: Path to database file to clean up
    """
    if os.path.exists(test_db):
        try:
            os.unlink(test_db)
        except (PermissionError, OSError):
            # On Windows, database files may still be locked by SQLite
            # This is a cleanup issue, not a test failure
            if sys.platform == "win32":
                pass
            else:
                raise


@pytest.fixture
def test_db() -> Generator[str, None, None]:
    """Create a temporary database file for testing.

    Yields a unique path per test. The database file is cleaned up after the test.
    """
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = f.name
    try:
        yield db_path
    finally:
        cleanup_db(db_path)


@pytest.fixture
def unique_table_prefix(request) -> str:
    """Unique table name per test to avoid cross-test collisions when running in parallel.

    Use for all CREATE TABLE / INSERT / SELECT so tables never clash across workers.
    Example: tbl = unique_table_prefix; conn.execute(f'CREATE TABLE {tbl} (a INT)').
    """
    h = hashlib.sha256(request.node.name.encode()).hexdigest()[:12]
    return f"t_{h}"


@pytest.fixture
def test_db_memory() -> str:
    """Create an in-memory database for testing.

    Returns:
        ":memory:" database path
    """
    return ":memory:"


@pytest.fixture
def dbapi_test_db(tmp_path):
    """Isolated temp DB path for dbapi tests (unique per test, uses pytest tmp_path)."""
    db_path = tmp_path / "dbapi_isolated.db"
    yield str(db_path)
    cleanup_db(str(db_path))


@pytest.fixture
async def dbapi_conn() -> AsyncGenerator[Any, None]:
    """Isolated async DBAPI connection (:memory:). Guaranteed close after test."""
    pytest.importorskip("rapsqlite")
    from rapsqlite import dbapi

    conn = await dbapi.connect(":memory:")
    try:
        yield conn
    finally:
        await conn.close()


# Pytest markers for test categorization
def pytest_configure(config):
    """Register custom pytest markers and ensure Windows event loop policy is set."""
    # Ensure WindowsSelectorEventLoopPolicy is set before any tests run
    # This is especially important for pytest-xdist parallel execution on Windows
    # Each worker process will import conftest.py and get the correct policy
    if sys.platform == "win32":
        import asyncio

        # Set policy again in pytest_configure to ensure it's set early
        # (conftest.py module-level code runs first, but this provides extra safety)
        asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())

    config.addinivalue_line("markers", "unit: Unit tests")
    config.addinivalue_line("markers", "integration: Integration tests")
    config.addinivalue_line("markers", "edge_case: Edge case tests")
    config.addinivalue_line("markers", "concurrency: Concurrency tests")
    config.addinivalue_line("markers", "stress: Stress/load tests")
    config.addinivalue_line("markers", "performance: Performance tests")
    config.addinivalue_line(
        "markers", "perf_smoke: Quick performance smoke tests for PR CI"
    )
    config.addinivalue_line("markers", "property: Property-based tests")
    config.addinivalue_line("markers", "slow: Slow-running tests")
