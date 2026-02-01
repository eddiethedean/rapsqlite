# Test Failures Report

**Date:** January 31, 2026  
**Status:** 0 failing, 655+ passed (full suite), 39 skipped; 30 passed (SQLAlchemy suite), 30 skipped (aiosqlite)  
**Scope:** SQLAlchemy dialect (`sqlite+rapsqlite`) integration tests

---

## Executive Summary

**All three previously failing SQLAlchemy tests are now fixed (2026-01-31).** Two root causes were addressed:

1. **Transaction rollback** – `test_connection_explicit_transaction`: rollback does not undo inserts when SQLAlchemy's `on_connect` registers UDFs.
2. **Doubled ORM rows** – `test_async_session_add_commit_get` and `test_async_session_add_all_many_rows`: ORM inserts return 2× the expected rows, suggesting double execution of INSERT...RETURNING.

Eight previously failing tests are now fixed (tuple params, cursor description, parameterized selects, cursor reuse, etc.).

---

## Previously Failing Tests (Now Fixed)

### 1. `test_connection_explicit_transaction[rapsqlite]` — FIXED

**File:** `tests/test_sqlalchemy_rapsqlite.py` (lines 139–162)

**Behavior:**
- Creates a table, inserts row 1 and commits
- Begins a transaction, inserts row 2, calls `rollback()`
- Expects SELECT to return only row 1; instead returns rows 1 and 2

**Error:** `assert (2 == 1)` – 2 rows returned when 1 expected

**Root cause:** When `on_connect` registers `regexp` and `floor`, `has_callbacks_flag` becomes true. SQLAlchemy sends `BEGIN` via `do_begin()` → `conn.exec_driver_sql("BEGIN")`, which uses the execute path. The execute path was updated to detect raw `BEGIN` and set `transaction_state`, but rollback still behaves incorrectly in this flow. Possible causes:
- Transaction state not synced with raw `BEGIN`
- Connection routing (callback vs transaction) not matching expectations
- Pooled connection reuse or state leakage across connections

**Flow:**
1. `on_connect` → `create_function("regexp", ...)` and `create_function("floor", ...)` → `has_callbacks_flag = true`
2. `conn.begin()` → dialect `do_begin` → `exec_driver_sql("BEGIN")`
3. `BEGIN` runs on callback_connection and should set `transaction_state = Active`
4. INSERT runs on `transaction_connection`
5. `conn.rollback()` uses `transaction_state`; if it is not Active, rollback becomes a no-op

---

### 2. `test_async_session_add_commit_get[rapsqlite]` — FIXED

**File:** `tests/test_sqlalchemy_rapsqlite.py` (lines 321–356)

**Behavior:**
- ORM: `session.add_all([User(name="alice"), User(name="bob")])`, commit
- Selects all `User` rows and expects 2; gets 4

**Error:** `assert 4 == 2` – 4 rows instead of 2

---

### 3. `test_async_session_add_all_many_rows[rapsqlite]` — FIXED

**File:** `tests/test_sqlalchemy_rapsqlite.py` (lines 582–612)

**Behavior:**
- ORM: `session.add_all([Item(name=f"item_{i}") for i in range(20)])`, commit
- Selects all `Item` rows and expects 20; gets 40

**Error:** `assert 40 == 20` – 40 rows instead of 20

**Root cause (tests 2 & 3):** Likely double execution of INSERT...RETURNING. SQLAlchemy ORM uses `insertmanyvalues` with RETURNING for identity fetch. Hypotheses:
- `cursor.fetchall()` or equivalent re-runs the query when results are not cached
- Results from `ExecuteContextManager.__aenter__` are not cached before a subsequent fetch
- Cursor or result caching logic in `cursor.rs` (around line 556) causes re-execution

---

## Fixes Implemented

| Fix | File(s) | Status |
|-----|---------|--------|
| **Tuple parameter handling** | `src/connection.rs` | Done |
| **Cursor description timing** | `src/cursor.rs` | Done |
| **Transaction state (BEGIN)** | `src/context_managers.rs`, `src/utils.rs` | Done |
| **COMMIT/ROLLBACK post-execution** | `src/context_managers.rs` | Done |

### Fix 1: Tuple Parameter Handling

SQLAlchemy passes positional params as `PyTuple`, but only `PyList` was handled. Added `PyTuple` handling via `process_positional_parameters_tuple()` in all parameter-processing sites.

**Tests fixed:** `test_core_cursor_reuse_same_connection`, `test_parameterized_select_insert`

### Fix 2: Cursor Description Timing

The `description` getter fell back to `pending_description`, exposing metadata before `fetchall()`. Removed that fallback so `description` is only set after the first fetch or for 0-row results.

**Tests fixed:** `test_cursor_description`

### Fix 3: Transaction State with Raw SQL

Added helpers in `src/utils.rs`:
- `is_begin_query(query)`
- `is_commit_or_rollback_query(query)`

In `src/context_managers.rs`:
- For raw `BEGIN` with callbacks: execute on `callback_connection`, move it to `transaction_connection`, set `transaction_state = Active`
- After raw `COMMIT` or `ROLLBACK`: set `transaction_state = None`, move connection back to `callback_connection` when callbacks are used

**Note:** `test_connection_explicit_transaction` still fails; SQLAlchemy’s `do_begin` uses `exec_driver_sql("BEGIN")`, so this path may need further verification.

---

## Investigation Notes

### SQLAlchemy Transaction Flow

- Dialect `do_begin(conn)` calls `conn.exec_driver_sql("BEGIN")` (see `sqlalchemy/dialects/sqlite/base.py`)
- `exec_driver_sql` goes through the normal execute path → `cursor.execute("BEGIN")`
- For async, this uses `AsyncAdapt_dbapi_cursor` wrapping the rapsqlite cursor

### Connection Pooling

- File DBs use `AsyncAdaptedQueuePool`
- Each `async with engine.connect()` may receive a different connection from the pool
- Within one block, `begin()`, `execute()`, and `rollback()` operate on the same connection

### Callback vs Transaction Connection

- `callback_connection`: used when UDFs/trace/authorizer are registered; holds the connection with UDFs
- `transaction_connection`: used for explicit or implicit transactions
- When callbacks are active and raw `BEGIN` is executed, the connection is moved from `callback_connection` to `transaction_connection`

---

## Recommendations

1. **Transaction rollback:** Add debug logging or a focused test to confirm that raw `BEGIN` sets `transaction_state` and that `rollback()` uses the correct connection. Verify execution order and connection routing when `on_connect` has run.
2. **Doubled rows:** Inspect `cursor.rs` `fetchall()` and result caching for INSERT...RETURNING. Ensure results from `ExecuteContextManager.__aenter__` are cached before any fetch so the query is not run twice.
3. **Manual verification:** Use a small script that mirrors `test_connection_explicit_transaction` to reproduce the rollback behavior and verify each step of the transaction flow.

---

## Test Run Summary

**After fixes (2026-01-31):**
```
=================== 30 passed (rapsqlite SQLAlchemy suite), 30 skipped (aiosqlite) ===================
```

**Fixes applied:**
1. **Doubled rows:** Added `returns_result_rows` to ExecuteContextManager; for INSERT/UPDATE/DELETE ... RETURNING, use `bind_and_fetch_all_on_connection` and cache results via `_set_select_results` so `cursor.fetchall()` does not re-execute.
2. **Transaction rollback:** Added DML-with-callbacks branch: when `has_callbacks` and DML and not in transaction, run BEGIN on callback_connection, move to transaction_connection, set `transaction_state = Active` and `explicit_transaction = true`, so rollback works when SQLAlchemy's `conn.begin()` does not emit BEGIN through our execute path.

---

## Debugging Guide for Future Fixers

### Reproduce Locally

```bash
# Run failing tests only (faster)
pytest tests/test_sqlalchemy_rapsqlite.py::test_connection_explicit_transaction \
       tests/test_sqlalchemy_rapsqlite.py::test_async_session_add_commit_get \
       tests/test_sqlalchemy_rapsqlite.py::test_async_session_add_all_many_rows \
       -v -k rapsqlite --tb=short

# Rebuild after Rust changes
unset CONDA_PREFIX && maturin develop --release
```

### Key File Locations

| Purpose | File | Key Areas |
|--------|------|-----------|
| Transaction routing, BEGIN/COMMIT/ROLLBACK handling | `src/context_managers.rs` | Lines 178–313 (result routing, post-execution COMMIT/ROLLBACK) |
| Rollback logic | `src/connection.rs` | Lines 1199–1260 (`fn rollback`) |
| Cursor fetchall, re-execution check | `src/cursor.rs` | Lines 545–625 (`fetchall`, `needs_fetch`, `returns_result_rows`) |
| Query classification | `src/utils.rs` | `is_select_query`, `returns_result_rows`, `is_dml_query`, `is_begin_query`, `is_commit_or_rollback_query` |
| Connection `execute` entry point | `src/connection.rs` | Lines 1288–1520 (ExecuteContextManager creation, `is_select` from `is_select_query`) |
| DBAPI adapter | `rapsqlite/dbapi.py` | `AsyncCursor.execute` (lines 181–215), calls `_raw.fetchall()` after execute |
| SQLAlchemy dialect | `rapsqlite/sqlalchemy.py` | `on_connect` (lines 129–158) registers regexp/floor |

### Critical Distinction: `is_select_query` vs `returns_result_rows`

- **`is_select_query`** (`src/utils.rs:9`): `true` only for `SELECT` and `WITH`. Used in `connection.rs` to set `ExecuteContextManager.is_select`.
- **`returns_result_rows`** (`src/utils.rs:17`): `true` for `SELECT`, `WITH`, and `INSERT/UPDATE/DELETE ... RETURNING`. Used in `cursor.rs` `fetchall()` to decide if a query needs results.

**Implication for doubled rows:** `INSERT ... RETURNING` has `is_select_query = false`, so it goes through the **non-SELECT** branch in `ExecuteContextManager.__aenter__` (line 127). That branch executes the statement but does **not** call `_set_select_results` or otherwise cache RETURNING rows in the cursor. When `cursor.fetchall()` is later called (e.g. by SQLAlchemy to get identity rows), the cursor sees `results.is_none()` and `returns_result_rows(&query) == true`, so it **re-executes** the INSERT...RETURNING. That causes double inserts.

**Potential fix:** For non-SELECT queries where `returns_result_rows` is true, run the same eager fetch-and-cache logic as for SELECT, or ensure the non-SELECT branch populates the cursor’s results for RETURNING statements.

### Transaction Rollback Flow (test_connection_explicit_transaction)

1. **on_connect** runs when a connection is first checked out → `create_function("regexp", ...)`, `create_function("floor", ...)` → `user_functions` populated → `has_callbacks_flag = true`.
2. **`conn.begin()`** → SQLAlchemy dialect `do_begin(conn)` → `conn.exec_driver_sql("BEGIN")` → `cursor.execute("BEGIN")`.
3. **ExecuteContextManager** receives `"BEGIN"`: `is_select = false`, `has_callbacks_flag = true`, `is_begin_query("BEGIN") = true` → hits branch at lines 184–212.
4. That branch: takes `callback_connection`, executes BEGIN, moves it to `transaction_connection`, sets `transaction_state = Active`.
5. **INSERT** is routed via `in_transaction_after_hook = true` → uses `transaction_connection`.
6. **`conn.rollback()`** → `connection.rollback()` in Rust. Checks `transaction_state == Active`; if not, returns early (no-op).
7. If rollback runs: takes connection from `transaction_connection`, executes ROLLBACK, returns it to `callback_connection` if `has_callbacks_flag`.

**Debug checks:** Log `transaction_state` when rollback is called; confirm `transaction_connection` is non-empty; verify the `"BEGIN"` branch is actually taken (e.g. log when `is_begin_query` matches). Check whether `exec_driver_sql` uses a different execution path than `execute()`.

### SQLAlchemy Call Paths

- **Transaction:** `conn.begin()` → dialect `do_begin` → `conn.exec_driver_sql("BEGIN")` → DBAPI `cursor.execute("BEGIN")`.
- **Queries:** `conn.execute(text("INSERT ..."))` → DBAPI `cursor.execute()`.
- **Rollback:** `conn.rollback()` → DBAPI `connection.rollback()` (Rust `Connection::rollback`).

The DBAPI connection is `rapsqlite.Connection` (high-level); it wraps the Rust `Connection`. `cursor()` returns an `AsyncCursor` (dbapi.py) wrapping the Rust `Cursor`.

### Data Flow for INSERT...RETURNING (ORM)

1. SQLAlchemy ORM `session.add_all([...])` + commit → `insertmanyvalues` with RETURNING for identity fetch.
2. SQLAlchemy emits `INSERT INTO table (...) VALUES (...), (...), ... RETURNING id`.
3. DBAPI: `cursor.execute(sql)` → Rust `Connection::execute` → `ExecuteContextManager.__aenter__`.
4. `is_select_query("INSERT...RETURNING")` = false → non-SELECT branch executes DML, does not fetch/cache RETURNING rows.
5. SQLAlchemy calls `cursor.fetchall()` to read RETURNING rows.
6. Rust `Cursor::fetchall` sees `results.is_none()` and `returns_result_rows` = true → re-executes the INSERT → second set of rows inserted.
7. SQLAlchemy receives duplicated rows.

### Where to Add Debug Logging

- **Transaction state:** In `src/connection.rs` `rollback()` at line 1218: log `*trans_guard` and whether `transaction_connection` has a connection.
- **BEGIN handling:** In `src/context_managers.rs` at line 184: log when the `is_begin_query` branch is taken.
- **Double execution:** In `src/cursor.rs` `fetchall()` at line 554: log when `needs_fetch` is true and `returns_result_rows` is true for a non-SELECT query.
- **Query classification:** Log `is_select` and `returns_result_rows` when creating `ExecuteContextManager` for INSERT...RETURNING.

### Known Gotchas

- **Pool vs connection:** Each `async with engine.connect()` may get a different pooled connection. Transaction state is per-connection.
- **Two Connection types:** `rapsqlite.Connection` (high-level) vs Rust `Connection`; SQLAlchemy uses the high-level one via the DBAPI.
- **Cursor results:** The Rust `Cursor` has `results: Arc<StdMutex<Option<Vec<SqliteRow>>>>`. For non-SELECT that returns rows, this is never filled by `ExecuteContextManager`; only the SELECT branch calls `_set_select_results`.

### Minimal Reproduction Scripts

**Rollback (test_connection_explicit_transaction):**
```python
import asyncio
import rapsqlite.sqlalchemy
from sqlalchemy.ext.asyncio import create_async_engine
from sqlalchemy import text
import tempfile, os

async def main():
    with tempfile.NamedTemporaryFile(suffix='.db', delete=False) as f:
        db = f.name
    try:
        engine = create_async_engine(f'sqlite+rapsqlite:///{db}')
        async with engine.connect() as conn:
            await conn.execute(text('CREATE TABLE t (id INTEGER PRIMARY KEY)'))
            await conn.commit()
        async with engine.connect() as conn:
            await conn.begin()
            await conn.execute(text('INSERT INTO t (id) VALUES (2)'))
            await conn.rollback()
        async with engine.connect() as conn:
            rows = (await conn.execute(text('SELECT id FROM t'))).fetchall()
            print(rows)  # Expect [(1,)] if rollback worked; get [(1,), (2,)] if not
        await engine.dispose()
    finally:
        os.unlink(db)
asyncio.run(main())
```

**Doubled rows (test_async_session_add_commit_get):**
```python
import asyncio
import rapsqlite.sqlalchemy
from sqlalchemy.ext.asyncio import create_async_engine, async_sessionmaker
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
from sqlalchemy import String, select

class Base(DeclarativeBase): pass
class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True, autoincrement=True)
    name: Mapped[str] = mapped_column(String(50))

async def main():
    engine = create_async_engine("sqlite+rapsqlite:///:memory:")
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    from sqlalchemy.ext.asyncio import AsyncSession
    async_session = async_sessionmaker(engine, expire_on_commit=False, class_=AsyncSession)
    async with async_session() as session:
        async with session.begin():
            session.add_all([User(name="alice"), User(name="bob")])
    async with async_session() as session:
        result = await session.execute(select(User).order_by(User.id))
        users = result.scalars().all()
        print(len(users))  # Expect 2; get 4 if doubled
    await engine.dispose()
asyncio.run(main())
```
