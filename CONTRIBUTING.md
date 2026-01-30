# Contributing to rapsqlite

Thank you for your interest in contributing to rapsqlite!

## Development Setup

1. Clone the repository:
```bash
git clone https://github.com/eddiethedean/rapsqlite.git
cd rapsqlite
```

2. Ensure Python development headers are installed:
   - **macOS (Homebrew)**: Python headers are included with Python installations
   - **Linux (Debian/Ubuntu)**: `sudo apt-get install python3-dev`
   - **Linux (Fedora/RHEL)**: `sudo dnf install python3-devel`
   - **Windows**: Python headers are included with Python installations
   - **pyenv users**: Headers are included when installing Python versions via pyenv

3. Install development dependencies (use the same Python you will use for build and tests):
```bash
python -m pip install maturin pytest pytest-asyncio pytest-cov pytest-xdist pytest-timeout hypothesis
```

4. Build the package in development mode:
```bash
# Use the same Python for build and tests (required)
python -m maturin develop

# Or set PYO3_PYTHON explicitly if needed
export PYO3_PYTHON="$(python -c 'import sys; print(sys.executable)')"
python -m maturin develop
```

**Python version alignment**: The extension is built for the Python used by `maturin develop`. Always run tests with **that same interpreter** (e.g. `python -m pytest tests/`). Using a different Python (e.g. pyenv 3.8 vs system 3.12) loads a different or missing wheel, causing Phase 3 tests to skip and compatibility fallbacks to activate. Use `scripts/dev_test.sh` to build and test with one Python.

**Note**: Use `maturin develop` instead of `cargo build` for development, as maturin automatically handles Python library linking. If you need to use `cargo build` directly, ensure `PYO3_PYTHON` is set to your Python executable.

## Running Tests

See [tests/README.md](tests/README.md) for detailed testing documentation.

### Quick Start
```bash
# Run all tests (use same Python as build)
python -m pytest tests/

# Run tests in parallel
python -m pytest tests/ -n 10

# Run with coverage
python -m pytest tests/ --cov=rapsqlite --cov-report=html
```

Or use the dev script: `./scripts/dev_test.sh` (builds then runs tests with one Python).

## Code Style

- **Python**: Follow PEP 8, use `ruff` for formatting and linting
- **Rust**: Follow Rust style guidelines, use `cargo fmt` and `cargo clippy`
- **Type hints**: Use type hints for all Python code
- **Docstrings**: Use Google-style docstrings

### Formatting
```bash
# Python
ruff format .

# Rust
cargo fmt
```

### Linting
```bash
# Python
ruff check .

# Rust
cargo clippy --lib -- -D clippy::all -A deprecated
```

## Writing Tests

### Test Organization
- Use appropriate pytest markers (`@pytest.mark.unit`, `@pytest.mark.integration`, etc.)
- Group related tests logically
- Use descriptive test names
- Write clear test descriptions

### Test Fixtures
Always use the provided fixtures:
- `test_db` - Temporary database file
- `test_db_memory` - In-memory database

### Example Test
```python
import pytest
from rapsqlite import connect

@pytest.mark.asyncio
async def test_feature_name(test_db):
    """Test description."""
    async with connect(test_db) as db:
        await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        # Test assertions
        assert True
```

See [tests/README.md](tests/README.md) for more details.

## Submitting Changes

1. **Create a branch** from `main` or the appropriate feature branch
2. **Write tests** for your changes
3. **Ensure all tests pass**: `python -m pytest tests/` (same Python as build)
4. **Run linting**: `ruff check .` and `cargo clippy`
5. **Update documentation** if needed
6. **Commit changes** with clear commit messages
7. **Push and create a pull request**

## Commit Messages

Use clear, descriptive commit messages:
```
Add feature: connection pool configuration

- Add pool_size getter/setter
- Add connection_timeout getter/setter
- Add comprehensive test suite
- Update documentation
```

## Pull Request Process

1. Ensure all tests pass
2. Ensure code is formatted and linted
3. Update documentation if needed
4. Add changelog entry if applicable
5. Request review

## Test Coverage

- Aim for 80%+ test coverage
- Add tests for new features
- Add edge case tests for critical paths
- Test error conditions

## Documentation

- Update README.md for user-facing changes
- Update docstrings for API changes
- Update CHANGELOG.md for notable changes
- Update docs/ for significant features

## Questions?

Feel free to open an issue for questions or discussions!
