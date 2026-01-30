# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-30 12:45:12
**rapsqlite Version**: 0.3.0-dev
**Python Version**: 3.9.6

## Summary

- **Total Test Files**: 2
- **✅ Passed**: 1
- **❌ Failed**: 1
- **⏭️  Skipped**: 0

## Passed Tests

- `perf.py`

## Failed Tests

- `smoke.py`

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [MIGRATION.md](MIGRATION.md) for details on known differences.

**Compatibility fix applied:** `connect()` now accepts `pathlib.Path` / `os.PathLike` via `os.fspath(path)` so aiosqlite smoke tests that pass `Path` (e.g. fixture `self.db`) no longer raise "PosixPath cannot be cast as str". Run script was updated to use PYTHONPATH and exclude helpers/__main__ so smoke.py relative imports work; perf.py passes.

**Per-test failure notes (smoke.py):**

| Failure reason | Tests | Notes |
|----------------|-------|--------|
| Unable to open database file (code 14) | test_close_blocking_until_transaction_queue_empty, test_connection_await, test_connection_context, test_connection_locations, test_context_cursor, test_create_function, test_create_function_deterministic, test_cursor_on_closed_connection, test_cursor_on_closed_connection_loop, test_enable_load_extension, test_fetch_all, test_multiple_connections, test_multiple_queries, test_set_authorizer_*, test_set_trace_callback | Temp dir / path in cloned test run; may be environment (cwd, permissions) when running from script temp dir. |
| API difference: Connection(factory, loop) | test_connect_base_exception | aiosqlite uses two-arg Connection(factory, loop); rapsqlite uses Connection(path). Intentional; document. |
| connect(bad_path) behavior | test_connect_error | aiosqlite may raise on connect; rapsqlite may return connection that fails on first use. |
| total_changes | test_connection_properties | aiosqlite may expose as property; rapsqlite exposes async method (await conn.total_changes()). |
| Cursor.execute() return / await | test_cursor_return_self, test_iterable_cursor | aiosqlite may return awaitable that resolves to cursor; rapsqlite returns awaitable. |
| set_progress_handler(n, callback) | test_set_progress_handler | Argument order or types (n vs callback); aiosqlite may use (callback, n). |
| ResourceWarning when left open | test_emits_warning_when_left_open | rapsqlite uses __del__ to schedule close(); may not emit ResourceWarning. Intentional. |
| iterdump INSERT format | test_iterdump | rapsqlite outputs column names in INSERT; aiosqlite may not. Optional alignment. |
| tuple parameter type | test_multi_loop_usage | Unsupported parameter type: tuple; aiosqlite may allow. |
| stop() return | test_stop_without_close | aiosqlite stop() may be awaitable; rapsqlite stop() is no-op and returns None. |
| backup to sqlite3 :memory: | test_backup_sqlite | rapsqlite disallows backup to sqlite3.Connection for in-memory. Intentional. |
| backup_aiosqlite / no such table | test_backup_aiosqlite | Backup or :memory: behavior difference. |

**Common failure reasons:**
- API differences (intentional or unintentional)
- Different error message formats
- Behavioral differences in edge cases
- Missing features in rapsqlite
- Environment (temp dir) when running from adapter script

**Next steps:**
1. Review failed tests to identify compatibility gaps
2. Fix compatibility issues where possible (e.g. total_changes property, set_progress_handler signature)
3. Document intentional differences in MIGRATION.md

## Notes

- Tests were run by patching aiosqlite imports to use rapsqlite
- Some failures may be due to intentional differences (see [MIGRATION.md](MIGRATION.md))
- Some failures may indicate areas for improvement in rapsqlite compatibility
- This is a compatibility validation exercise, not a requirement for 100% pass rate
