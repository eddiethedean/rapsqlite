# rapsqlite Test Suite

This directory contains the comprehensive test suite for `rapsqlite`.

## Test Organization

Tests are organized into the following files:

### Core Tests
- **`test_rapsqlite.py`** - Basic functionality tests
- **`test_aiosqlite_compat.py`** - aiosqlite compatibility tests (migrated from aiosqlite test suite)

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

### Run All Tests
```bash
python -m pytest tests/
```

Use the same Python you used to install the package (`python -m pip install -e .` or `python -m maturin develop`). Alternatively, run `./scripts/dev_test.sh` to build and test with one interpreter.

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

Tests are categorized using pytest markers:

- `@pytest.mark.unit` - Unit tests
- `@pytest.mark.integration` - Integration tests
- `@pytest.mark.edge_case` - Edge case tests
- `@pytest.mark.concurrency` - Concurrency tests
- `@pytest.mark.stress` - Stress/load tests
- `@pytest.mark.performance` - Performance tests
- `@pytest.mark.property` - Property-based tests
- `@pytest.mark.slow` - Slow-running tests

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
- `test_db` - Temporary database file fixture
- `test_db_memory` - In-memory database fixture
- `cleanup_db()` - Database cleanup helper

## Test Coverage

Current test coverage targets:
- **Goal**: 80%+ coverage
- **Current**: Run `python -m pytest tests/ --cov=rapsqlite --cov-report=term-missing` to see current coverage

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

## Best Practices

1. **Use fixtures** - Always use `test_db` or `test_db_memory` fixtures
2. **Clean up** - Fixtures handle cleanup automatically
3. **Mark tests** - Use appropriate markers for test categorization
4. **Test edge cases** - Add edge case tests for critical paths
5. **Test errors** - Test error conditions and exception handling
6. **Document tests** - Write clear test descriptions
7. **Keep tests fast** - Mark slow tests with `@pytest.mark.slow`
