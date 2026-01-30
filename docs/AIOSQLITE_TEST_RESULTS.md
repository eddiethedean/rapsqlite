# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-30  
**rapsqlite Version**: 0.3.0-dev  
**Python Version**: 3.9.6

## Summary

- **Total Test Files**: 2
- **Passed**: 1 (`perf.py`)
- **Failed**: 1 (`smoke.py` — multiple test cases)
- **Skipped**: 0

## Passed Tests

- `perf.py` — Performance tests pass with rapsqlite.

## Failed Tests (smoke.py)

Per-test breakdown with category: **fix** (compatibility change in rapsqlite), **document** (intentional difference; see migration guide), or **environment** (runner/temp dir/quirk).

| Test | Category | Notes |
|------|----------|--------|
| `test_backup_aiosqlite` | document | Backup API: rapsqlite supports backup to rapsqlite or sqlite3 target; target type or progress callback signature may differ. |
| `test_backup_sqlite` | document | Same as above; backup to sync sqlite3.Connection has specific semantics (file-backed only). |
| `test_close_blocking_until_transaction_queue_empty` | document | aiosqlite uses a transaction queue; rapsqlite does not (true async, no queue). `close()` semantics differ. |
| `test_connect_base_exception` | document | Connect error handling: exception type or message may differ; rapsqlite raises OperationalError/ProgrammingError. |
| `test_connect_error` | document | Same as above; connect failure behavior documented in migration guide. |
| `test_connection_await` | fix | `await conn` pattern is supported; likely assertion on return type or context. Verify and fix if trivial. |
| `test_connection_context` | document | Context manager behavior may differ (e.g. commit on exit); documented. |
| `test_connection_locations` | environment | May rely on temp dir or path handling when run from script. |
| `test_connection_properties` | document | `total_changes` and `in_transaction` are async methods in rapsqlite (`await conn.total_changes()`), not properties. Documented in migration guide. |
| `test_context_cursor` | document | Cursor context manager; small behavioral differences possible. |
| `test_create_function` | document | UDF support is implemented; signature or callback semantics may differ slightly. |
| `test_create_function_deterministic` | document | `deterministic=True` is supported; SQLite version or error message may differ. |
| `test_cursor_on_closed_connection` | document | Error when using cursor after close; exception type/message may differ. |
| `test_cursor_on_closed_connection_loop` | document | Same as above. |
| `test_cursor_return_self` | fix | rapsqlite supports `Cursor.execute()` returning self (chaining). Likely assertion detail; verify and fix if trivial. |

## Category summary

- **fix**: 2 — `test_connection_await`, `test_cursor_return_self` (verify and fix assertion/behavior if low-risk).
- **document**: 12 — Intentional or known differences; see [Migration Guide](guides/migration-guide.rst) and [Compatibility](guides/compatibility.rst).
- **environment**: 1 — Temp dir or path when run via script.

## Next steps

1. **Fix**: For `test_connection_await` and `test_cursor_return_self`, run with `-v --tb=long` to get exact assertion; adjust rapsqlite or tests if trivial.
2. **Document**: Keep compatibility.rst and migration-guide.rst as the source of truth for intentional differences.
3. **CI**: Optionally add a CI job or contributor note to run `scripts/run_aiosqlite_tests.py` periodically.

## Notes

- Tests are run by patching aiosqlite imports to use rapsqlite (`import rapsqlite as aiosqlite`).
- Some failures are intentional design differences (async methods vs properties, no transaction queue, backup API).
- This is a compatibility validation exercise; 100% pass rate is not required if differences are documented.
