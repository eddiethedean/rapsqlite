# rapsqlite integration examples

Optional examples for using rapsqlite.

**Setup:** Install rapsqlite (`maturin develop` or `pip install -e .`). Run from project root.

### Run all examples with real output

```bash
python scripts/verify_examples.py
```

Runs basic usage, connect(), transactions, cursors, concurrent operations, error handling, backup, schema operations, and init_hook — and prints the actual output from each.

### Basic async (no extra deps)

```bash
python examples/async_basic.py
```

**Output:** `rows: [[1, 'hello']]`

### FastAPI

```bash
pip install "fastapi[standard]" "uvicorn[standard]"
python -m uvicorn examples.fastapi_db:app --reload
# GET http://127.0.0.1:8000/
```

### SQLAlchemy async

The aiosqlite SQLAlchemy dialect expects aiosqlite-specific behavior (e.g. `daemon` on connections) we don't yet emulate. Use rapsqlite directly or aiosqlite for SQLAlchemy async for now.
