# Plan: Option B — set_pragma Behavior Fix

## Goal

Fix the failing `test_set_pragma` compat test and ensure `set_pragma` behavior (value formatting, PRAGMA application, and return values) matches expectations. Low-effort, high‑impact quick win.

## Current Failure

- **Test:** `tests/test_aiosqlite_compat.py::test_set_pragma`
- **Error:** `assert rows[0][0] == 2` fails — we return `1`, test expects `2`.
- **Context:** Test runs `await db.set_pragma("synchronous", "NORMAL")`, then `await db.fetch_all("PRAGMA synchronous")` and asserts `rows[0][0] == 2` (NORMAL = 2).

## Root Cause Analysis

### 1. PRAGMA synchronous numeric values (SQLite)

Per [SQLite PRAGMA documentation](https://www.sqlite.org/pragma.html#pragma_synchronous):

| Value | Mode  |
|-------|-------|
| 0     | OFF   |
| 1     | NORMAL|
| 2     | FULL  |
| 3     | EXTRA |

SQLite returns the **numeric** value when querying `PRAGMA synchronous`. So `NORMAL` → `1`, not `2`. The compat test assumes NORMAL = 2, which contradicts SQLite.

### 2. Inconsistency in compat tests

- **`test_set_pragma`:** expects `rows[0][0] == 2` for NORMAL (incorrect per SQLite).
- **`test_pragma_constructor_parameter`:** uses `synchronous = "NORMAL"` and expects `rows[0][0] == 1` (correct per SQLite).

So the suite is internally inconsistent. The fix should align both with SQLite semantics (NORMAL = 1).

### 3. set_pragma implementation (relevant parts)

- **Location:** `src/lib.rs`, `Connection::set_pragma` (~1736–1795).
- **Value conversion:** `None` → `NULL`; `int` → string; `str` → `'escaped'`. We execute e.g. `PRAGMA synchronous = 'NORMAL'`, which SQLite accepts.
- **Execution:** We run `PRAGMA name = value` via `sqlx::query(...).execute(&pool_clone)`. PRAGMAs are **per-connection**; we execute on the pool (one acquired connection). With `pool_size > 1`, only that connection gets the PRAGMA; others do not. With default `max_connections(1)`, there is only one connection, so behavior is effectively correct for the common case.

### 4. PRAGMA application on pool creation

`get_or_create_pool` applies stored pragmas to each new connection by running `PRAGMA name = value` for each stored entry. `set_pragma` both stores the pragma and immediately runs it on the pool. If the pool already exists, we run it on one connection only; the rest are unchanged. Consider documenting or fixing “set_pragma with pool_size > 1” if we want strict per-connection consistency.

## Implementation Plan

### Phase 1: Fix test expectation (recommended)

1. **Update `test_set_pragma`:** Change `assert rows[0][0] == 2` to `assert rows[0][0] == 1` when using `set_pragma("synchronous", "NORMAL")`, since SQLite returns 1 for NORMAL.
2. **Optional:** Add a short comment referencing SQLite’s numeric values (0=OFF, 1=NORMAL, 2=FULL, 3=EXTRA) to avoid future confusion.
3. **Run:** `pytest tests/test_aiosqlite_compat.py::test_set_pragma tests/test_aiosqlite_compat.py::test_pragma_constructor_parameter -v` and confirm both pass.

### Phase 2: Verify set_pragma behavior (optional hardening)

1. **Value formatting:** Confirm we never double-quote or otherwise corrupt values (e.g. `'NORMAL'` vs `NORMAL`). SQLite accepts both; we use quoted strings for non-numeric values. No change required if behavior is already correct.
2. **Integer PRAGMAs:** When the user passes an int (e.g. `set_pragma("synchronous", 2)`), we use the numeric string. Verify `PRAGMA synchronous` then returns 2.
3. **Quick regression:** Run `tests/test_pool_config.py::test_pool_config_with_set_pragma` and any other set_pragma-related tests.

### Phase 3: Documentation (optional)

- In ROADMAP or a short “Behavior” doc, note that PRAGMA return values follow SQLite’s numeric definitions (e.g. synchronous) and that `set_pragma` runs on the pool (single connection with default pool size).

## Out of Scope

- Changing how PRAGMAs are applied when `pool_size > 1` (multi-connection). Document as limitation or address in a separate change.
- Adding new PRAGMA-specific conveniences beyond the current API.

## Success Criteria

- `test_set_pragma` and `test_pragma_constructor_parameter` both pass.
- No regressions in `test_pool_config`, `test_rapsqlite`, or other aiosqlite compat tests that touch PRAGMAs.

## Order of Work

1. Update `test_set_pragma` assertion from `== 2` to `== 1` (and add comment).
2. Run the two PRAGMA compat tests plus pool_config set_pragma test.
3. (Optional) Add minimal docs on PRAGMA return values and pool behavior.

## References

- [SQLite PRAGMA synchronous](https://www.sqlite.org/pragma.html#pragma_synchronous)
- `src/lib.rs`: `Connection::set_pragma`, `get_or_create_pool` pragma application
- `tests/test_aiosqlite_compat.py`: `test_set_pragma`, `test_pragma_constructor_parameter`
- `docs/ROADMAP.md`: set_pragma behavior investigation
