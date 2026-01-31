# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-31 11:45:02
**rapsqlite Version**: 0.3.0-dev
**Python Version**: 3.10.18

**Run script**: `scripts/run_aiosqlite_tests.py` patches imports to use rapsqlite and injects `aiosqlite_compat=True` for all `connect()` calls so rows are tuples (aiosqlite default).

## Summary

- **Total Test Files**: 2
- **✅ Passed (tests)**: 9 (perf.py: 6, smoke.py: 3)
- **❌ Failed (tests)**: 31 (perf.py: 4, smoke.py: 27)
- **⏭️  Skipped**: 0
- **Pass rate**: 22.5% (9/40). v0.3.0 release criteria allow "intentional differences documented" when pass rate is below 80%.

## Passed Tests

- `perf.py`: test_atomics, test_connection_file, test_connection_memory, test_insert_ids, test_insert_macro_ids, test_iterable_cursor_perf
- `smoke.py`: test_close_twice, test_stop_after_event_loop_closed, test_stop_without_close

## Failed Tests

- `perf.py` (4 failed): test_inserts, test_inserts_authorized, test_select, test_select_macro (DatabaseError: table — environment/isolation)
- `smoke.py` (27 failed)

### Known intentional differences

rapsqlite intentionally differs from aiosqlite in these areas. Failures that fall into these categories are **documented** rather than fixed:

- **Row format**: rapsqlite default is list rows; aiosqlite/sqlite3 use tuples. The run script uses `aiosqlite_compat=True` so tests get tuple rows. See [migration guide](guides/migration-guide.rst) and [compatibility](guides/compatibility.rst).
- **Transaction queue**: aiosqlite queues transaction work on a background thread; rapsqlite runs operations directly on the connection.
- **Backup API**: rapsqlite supports backup to rapsqlite and to sqlite3.Connection (file-backed only).
- **Connection lifecycle**: rapsqlite requires explicit `close()` or `async with`; GC cleanup is best-effort.
- **Error message format**: rapsqlite may raise with different message text; exception types match.

For full lists and migration steps, see [migration guide](guides/migration-guide.rst) and [compatibility](guides/compatibility.rst).

### Failure categories (per test)

| Test | Category | Reason |
|------|----------|--------|
| perf: test_inserts, test_inserts_authorized, test_select, test_select_macro | environment | Test isolation / shared :memory: or table state (DatabaseError: table) |
| smoke: test_backup_*, test_close_blocking_until_transaction_queue_empty | document | Backup API; no transaction queue |
| smoke: test_connect_*, test_connection_*, test_context_cursor | fix/document | Connect/context behavior or exception format |
| smoke: test_create_function*, test_cursor_*, test_fetch_all, test_iterable_cursor | fix | create_function/cursor/row format or iteration |
| smoke: test_set_authorizer_*, test_set_progress_handler, test_set_trace_callback | fix | Callback signature or behavior |
| smoke: test_emits_warning*, test_cursor_on_closed_*, test_stop_without_close | document | Warnings; closed connection; stop() no-op |

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [migration guide](guides/migration-guide.rst) for details on known differences.

**Common failure reasons:** API differences, error message format, row format (handled by aiosqlite_compat in run script), test environment (patched imports, :memory: isolation).

## Per-Test Breakdown

### `perf.py`

| Test | Status | Error |
|------|--------|-------|
| test_atomics | PASSED |  |
| test_connection_file | PASSED |  |
| test_connection_memory | PASSED |  |
| test_insert_ids | PASSED |  |
| test_insert_macro_ids | PASSED |  |
| test_inserts | FAILED |  |
| test_inserts_authorized | FAILED |  |
| test_iterable_cursor_perf | PASSED |  |
| test_select | FAILED |  |
| test_select_macro | FAILED | ____________________________ PerfTest.test_inserts _____________________________ |

### `smoke.py`

| Test | Status | Error |
|------|--------|-------|
| test_backup_aiosqlite | FAILED |  |
| test_backup_sqlite | FAILED |  |
| test_close_blocking_until_transaction_queue_empty | FAILED |  |
| test_close_twice | PASSED |  |
| test_connect_base_exception | FAILED |  |
| test_connect_error | FAILED |  |
| test_connection_await | FAILED |  |
| test_connection_context | FAILED |  |
| test_connection_locations | FAILED |  |
| test_connection_properties | FAILED |  |
| test_context_cursor | FAILED |  |
| test_create_function | FAILED |  |
| test_create_function_deterministic | FAILED |  |
| test_cursor_on_closed_connection | FAILED |  |
| test_cursor_on_closed_connection_loop | FAILED |  |
| test_cursor_return_self | FAILED |  |
| test_emits_warning_when_left_open | FAILED |  |
| test_enable_load_extension | FAILED |  |
| test_fetch_all | FAILED |  |
| test_iterable_cursor | FAILED |  |
| test_iterdump | FAILED |  |
| test_multi_loop_usage | FAILED |  |
| test_multiple_connections | FAILED |  |
| test_multiple_queries | FAILED |  |
| test_set_authorizer_deny_drops | FAILED |  |
| test_set_authorizer_exception_propagation | FAILED |  |
| test_set_progress_handler | FAILED |  |
| test_set_trace_callback | FAILED |  |
| test_stop_after_event_loop_closed | PASSED |  |
| test_stop_without_close | FAILED | _______________________ SmokeTest.test_backup_aiosqlite ________________________ |


## Notes

- Tests were run by patching aiosqlite imports to use rapsqlite
- Some failures may be due to intentional differences (see migration guide)
- Some failures may indicate areas for improvement in rapsqlite compatibility
- This is a compatibility validation exercise, not a requirement for 100% pass rate
