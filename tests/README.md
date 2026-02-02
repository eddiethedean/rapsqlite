# rapsqlite Test Suite

This directory contains the comprehensive test suite for `rapsqlite`.

## Test Organization

Tests are organized into the following files:

### Core Tests
- **`test_rapsqlite.py`** - Basic functionality tests
- **`test_aiosqlite_compat.py`** - aiosqlite compatibility tests (migrated from aiosqlite test suite)
- **`test_sqlite3_dbapi.py`** - DB-API and sqlite3-style behavior (ported from CPython test_sqlite3)
- **`test_sqlite3_types.py`** - Type handling and register_adapter/register_converter (ported from CPython test_sqlite3)
- **`test_sqlite3_hooks.py`** - Authorizer and progress handler (ported from CPython test_sqlite3)

### Feature Tests
- **`test_pool_config.py`** - Connection pool configuration tests
- **`test_row_factory.py`** - Row factory tests
- **`test_prepared_statements.py`** - Prepared statement caching tests
- **`test_init_hook.py`** - Database initialization hook tests
- **`test_schema_operations.py`** - Schema introspection tests
- **`test_callback_robustness.py`** - SQLite callback tests
- **`test_async_with_execute.py`** - Async context manager tests
- **`test_dropin_replacement.py`** - Drop-in replacement validation

### Advanced Tests
- **`test_edge_cases.py`** - Comprehensive edge case tests
- **`test_error_conditions.py`** - Error handling and exception tests
- **`test_concurrency.py`** - Concurrent operation tests
- **`test_stress.py`** - Stress and load tests
- **`test_properties.py`** - Hypothesis property-based tests
- **`test_integration.py`** - Integration and real-world scenario tests
- **`test_performance.py`** - Performance regression tests

## External test sources

Tests in this suite are derived from or validated against the following external sources:

- **CPython test_sqlite3** — Tests in `test_sqlite3_dbapi.py`, `test_sqlite3_types.py`, and `test_sqlite3_hooks.py` are converted from the CPython stdlib sqlite3 test package ([Lib/test/test_sqlite3](https://github.com/python/cpython/tree/main/Lib/test/test_sqlite3)). They have been adapted to async pytest and rapsqlite’s API; tests that require unsupported features (e.g. blobopen, setlimit) are skipped. See the file headers for CPython/pysqlite license attribution.

- **aiosqlite** — The aiosqlite test suite is run against rapsqlite via `scripts/run_aiosqlite_tests.py` (import patching). Results and intentional differences are documented in `docs/AIOSQLITE_TEST_RESULTS.md`. Selected aiosqlite smoke tests are also ported into `test_aiosqlite_compat.py` for in-tree coverage.

- **SQL logic tests** — A minimal runner in `scripts/run_sql_logic_tests.py` runs simple-format test files (SQL statements, then `----`, then expected rows) against rapsqlite `:memory:` and compares results. Test files live in `scripts/sql_logic_tests/*.sql`.

## Running Tests

### Install/build for local testing

Most tests exercise the compiled Rust extension, so install the package in editable mode first:

```bash
python -m pip install -e .
```

Use the **same** Python for both install and tests (`python -m pytest tests/`). Building with one interpreter and testing with another loads a different extension build; Phase 3 tests will skip and some compatibility fallbacks apply. See **Version alignment** below.

On macOS, if you hit linker errors about missing Python symbols when building locally, use:

```bash
RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" python -m pip install -e .
```

Rust unit tests (fast, pure helpers):

```bash
cargo test
```

Note: because this crate links against the Python C-API via PyO3, `cargo test` can be
environment-dependent on macOS when it’s built in “extension module” mode (symbols resolved at
import time by Python).

On macOS, run Rust unit tests like this:

```bash
PYO3_PYTHON="$(python3 -c 'import sys; print(sys.executable)')" cargo test --no-default-features
```

This builds the crate **without** the `extension-module` feature, so the Rust test binary links
against `libpython` and can run standalone.

### Run All Tests (Rust + Python)
A full test run includes **Rust unit tests** and **Python pytest**. Use either:

```bash
./scripts/dev_test.sh
```

This builds with maturin, runs Rust unit tests (via `scripts/run_rust_tests.sh` on macOS/Linux, or `cargo test` on Windows), then runs `pytest tests/`. You can pass pytest options: `./scripts/dev_test.sh -m "not slow"`.

To run only Python tests:

```bash
python -m pytest tests/
```

Use the same Python you used to install the package (`python -m pip install -e .` or `python -m maturin develop`). On macOS, to run Rust tests alone use `./scripts/run_rust_tests.sh` (or set `PYO3_PYTHON` and `cargo test --no-default-features --lib`); on Windows, run `cargo test --no-default-features --lib` with Python on PATH if needed.

### Recommended fast local run (matches PR CI defaults)
```bash
python -m pytest tests/ -m "not slow and not stress and not performance"
```

### Run Tests in Parallel
```bash
python -m pytest tests/ -n 10
```

For high parallelism (e.g. `-n 12`) with timeout and grouped workers to reduce flakiness:
```bash
pip install -r requirements-test.txt   # includes pytest-timeout
python -m pytest tests/ -n 12 --timeout 60 --dist loadgroup
```
A default per-test timeout (90s) is set in `pyproject.toml`; tests marked `@pytest.mark.slow` get a 120s timeout via conftest. `--dist loadgroup` runs tests in the same `xdist_group` on one worker (e.g. init_hook, pool_exhaustion, concurrency), avoiding pool/DB contention and timeout flakiness.

**CI** uses pytest-timeout, `--timeout 90`, and `--dist loadgroup` for stable parallel runs.

Known unraisable-exception warnings when running with many workers (e.g. `-n 10`) come from background connection cleanup during shutdown. These are filtered in `pyproject.toml` (`filterwarnings`) so CI logs stay readable; see CONTRIBUTING or this README for details.

Tests use unique table names per test (via the `unique_table_prefix` fixture) so parallel runs avoid table-name collisions. Use this fixture for any new tests that create tables.

### Run Specific Test Categories
```bash
# Unit tests only
python -m pytest tests/ -m unit

# Integration tests only
python -m pytest tests/ -m integration

# Edge case tests
python -m pytest tests/ -m edge_case

# Concurrency tests
python -m pytest tests/ -m concurrency

# Stress tests (may be slow)
python -m pytest tests/ -m stress

# Performance tests
python -m pytest tests/ -m performance

# Property-based tests
python -m pytest tests/ -m property

# Skip slow tests
python -m pytest tests/ -m "not slow"

# Skip slow/stress/performance (fast default)
python -m pytest tests/ -m "not slow and not stress and not performance"
```

### Run with Coverage
```bash
python -m pytest tests/ --cov=rapsqlite --cov-report=html
```

### Version alignment
The Rust extension is built for a specific Python. Run tests with **that same** interpreter:

- **Correct**: `python -m maturin develop` then `python -m pytest tests/` (same `python`).
- **Incorrect**: building with `python3.12 -m maturin develop` but running `python3.10 -m pytest tests/` (different versions). The 3.10 run will load a different or missing wheel; Phase 3 API tests skip, and `connect(iter_chunk_size=...)` uses a fallback.

Use `./scripts/dev_test.sh` to build and test with one Python consistently.

## Test Fixtures

### `test_db` fixture
Creates a temporary database file for testing. Automatically cleaned up after each test.

```python
@pytest.mark.asyncio
async def test_example(test_db):
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
```

### `test_db_memory` fixture
Provides an in-memory database (`:memory:`) for testing.

```python
@pytest.mark.asyncio
async def test_example(test_db_memory):
    async with connect(test_db_memory) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
```

## Test Markers

Tests are categorized using pytest markers. Use `pytest -m unit`, `pytest -m "not slow"`, etc. to filter.

- `@pytest.mark.unit` - Unit tests (core API, basic behavior)
- `@pytest.mark.integration` - Integration tests (multi-component, examples, frameworks)
- `@pytest.mark.edge_case` - Edge case tests
- `@pytest.mark.concurrency` - Concurrency tests
- `@pytest.mark.stress` - Stress/load tests
- `@pytest.mark.performance` - Performance tests
- `@pytest.mark.property` - Property-based tests (Hypothesis)
- `@pytest.mark.slow` - Slow-running tests

**Marker by file (module-level `pytestmark` or per-test):** Core/feature tests use `unit`; example and framework tests use `integration`; edge/error tests use `edge_case`; concurrency/stress/performance/property files use their respective markers. See each test file for `pytestmark` or per-test markers.

## Writing New Tests

### Basic Test Structure
```python
import pytest
from rapsqlite import connect

@pytest.mark.asyncio
async def test_feature_name(test_db):
    """Test description."""
    async with connect(test_db) as db:
        # Test implementation
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        # Assertions
        assert True
```

### Using Markers
```python
@pytest.mark.edge_case
@pytest.mark.asyncio
async def test_edge_case(test_db):
    """Test edge case."""
    # Test implementation
    pass
```

### Testing Error Conditions
```python
import pytest
from rapsqlite import OperationalError

@pytest.mark.asyncio
async def test_error_handling(test_db):
    """Test error handling."""
    async with connect(test_db) as db:
        with pytest.raises(OperationalError):
            await db.execute("INVALID SQL")
```

## Test Utilities

### Shared Fixtures (`conftest.py`)
Use these for consistent isolation and parallel-safe runs (`-n 10`):

- **`test_db`** — Unique temp database file per test (path includes test name hash). Use for tests that need a real path or multiple connections; each test is fully isolated.
- **`connected_db`** — Open rapsqlite connection to `test_db`; closed after test. Use when you only need a single open connection. Use `test_db` when you need the path or multiple connections (e.g. backup source and target).
- **`test_db_memory`** — In-memory database (`:memory:`).
- **`test_db_file`** — Single temp file for tests that need a real path (e.g. backup, locking); use when you need one extra file alongside `test_db`.
- **`target_db_file`** — Second temp file (e.g. backup target).
- **`unique_table_prefix`** — Unique table name prefix per test to avoid cross-test collisions when using `-n 10`. Use for all `CREATE TABLE` / `INSERT` / `SELECT` in parallel runs.
- **`dbapi_conn`** — Isolated async DBAPI connection (`:memory:`), guaranteed closed after test.
- **`cleanup_db(path)`** — Helper to unlink a database file (fixtures use this for teardown).

For parallel runs, prefer `test_db` and `unique_table_prefix` so tests do not share state. See `tests/conftest.py` for fixture implementations.

## Test Coverage

Coverage is **informational** (no fail-under in CI yet). Goal: **80%+** for the `rapsqlite` package.

**Run with coverage:**
```bash
python -m pytest tests/ --cov=rapsqlite --cov-report=term-missing
```

**HTML report (open `htmlcov/index.html`):**
```bash
python -m pytest tests/ --cov=rapsqlite --cov-report=html
```

`--cov-report=term-missing` lists uncovered lines in the terminal. Install test deps first: `pip install -r requirements-test.txt` (includes pytest-cov).

## Continuous Integration

Tests run automatically on:
- All supported Python versions (3.10-3.14)
- All supported platforms (Linux, macOS, Windows)
- Full test suite with coverage reporting

## Performance Tests

Performance tests are marked with `@pytest.mark.performance` and `@pytest.mark.slow`. They:
- Measure execution time
- Detect performance regressions
- Validate performance characteristics

Run performance tests separately:
```bash
python -m pytest tests/ -m performance
```

## Property-Based Tests

Property-based tests use Hypothesis to test invariants:
- Parameter round-trip (insert → select)
- Transaction atomicity
- Pool size invariants
- Type conversion consistency

Run property tests:
```bash
python -m pytest tests/ -m property
```

## Debugging Tests

### Run Single Test
```bash
python -m pytest tests/test_rapsqlite.py::test_create_table -v
```

### Run with Output
```bash
python -m pytest tests/ -v -s
```

### Run with Debugger
```bash
python -m pytest tests/ --pdb
```

## Writing tests

**Fixtures:** Use `test_db` when you need a unique temp database path or multiple connections (e.g. backup source and target). Use `connected_db` when you only need a single open connection (fixture yields an open connection; use `test_db` when you need the path or a fresh connection). Use `test_db_file` when you need one extra temp file (e.g. backup target). Use `test_db_memory` for `:memory:`. See [tests/conftest.py](conftest.py) for definitions.

**Parallel safety:** Use the `unique_table_prefix` fixture for table names in tests that may run in parallel (`-n auto`), so `CREATE TABLE` / `INSERT` / `SELECT` do not clash across workers. Example: `tbl = unique_table_prefix; await conn.execute(f"CREATE TABLE {tbl} (id INT)")`.

**Markers:** Add a module-level `pytestmark` (e.g. `pytestmark = [pytest.mark.unit]`) or per-test markers so `pytest -m unit` / `pytest -m "not slow"` work. Use one category per file where the whole file fits (unit, integration, edge_case, concurrency, stress, performance, property).

**Coverage:** Run `python -m pytest tests/ --cov=rapsqlite --cov-report=term-missing` to see covered and missing lines. Goal is 80%+ for the `rapsqlite` package (informational; see **Test Coverage** above).

## Best Practices

1. **Use fixtures** - Always use `test_db` or `test_db_memory` fixtures
2. **Clean up** - Fixtures handle cleanup automatically
3. **Mark tests** - Use appropriate markers for test categorization
4. **Test edge cases** - Add edge case tests for critical paths
5. **Test errors** - Test error conditions and exception handling
6. **Document tests** - Write clear test descriptions
7. **Keep tests fast** - Mark slow tests with `@pytest.mark.slow`
