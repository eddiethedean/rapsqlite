# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-31 11:27:51
**rapsqlite Version**: 0.3.0-dev
**Python Version**: 3.10.18

## Summary

- **Total Test Files**: 2
- **✅ Passed**: 9 (perf.py: 6, smoke.py: 3)
- **❌ Failed**: 31 (perf.py: 4, smoke.py: 27)
- **⏭️  Skipped**: 0
- **Pass rate**: 22.5% (9/40). v0.3.0 release criteria allow "intentional differences documented" when pass rate is below 80%.

## Passed Tests

- `perf.py`: test_atomics, test_connection_file, test_connection_memory, test_insert_ids, test_insert_macro_ids, test_iterable_cursor_perf
- `smoke.py`: test_close_twice, test_stop_after_event_loop_closed, test_stop_without_close

## Failed Tests

- `perf.py` (4 failed)
- `smoke.py` (27 failed)

### Known intentional differences

rapsqlite intentionally differs from aiosqlite in these areas. Failures in the aiosqlite suite that fall into these categories are **documented** rather than fixed:

- **Row format**: rapsqlite default is list rows; aiosqlite/sqlite3 use tuples. Use `connect(..., aiosqlite_compat=True)` or `conn.row_factory = tuple` for tuple rows. See [migration guide](guides/migration-guide.rst) and [compatibility](guides/compatibility.rst).
- **Transaction queue**: aiosqlite queues transaction work on a background thread; rapsqlite runs operations directly on the connection. No "block until transaction queue empty" behavior.
- **Backup API**: rapsqlite supports backup to rapsqlite and to sqlite3.Connection (file-backed only). See migration guide for details.
- **Connection lifecycle**: rapsqlite requires explicit `close()` or `async with`; GC cleanup is best-effort.
- **Error message format**: rapsqlite may raise with different message text; exception types match (e.g. `OperationalError`, `ProgrammingError`).

For full lists of differences and migration steps, see [migration guide](guides/migration-guide.rst) and [compatibility](guides/compatibility.rst).

### Failure categories (per test)

| Test | Category | Reason |
|------|----------|--------|
| perf: test_inserts | environment | Test isolation / shared :memory: or table state (DatabaseError: table) |
| perf: test_inserts_authorized | environment | Same as test_inserts |
| perf: test_select | environment | Same as test_inserts |
| perf: test_select_macro | environment | Same as test_select |
| smoke: test_backup_aiosqlite | document | Backup API / target behavior |
| smoke: test_backup_sqlite | document | Backup to sqlite3 only for file-backed DBs |
| smoke: test_close_blocking_until_transaction_queue_empty | document | No transaction queue in rapsqlite |
| smoke: test_connect_base_exception | fix/document | Connect error handling / exception type or message |
| smoke: test_connect_error | fix/document | Connect error handling |
| smoke: test_connection_await | fix | Connection.__await__ semantics |
| smoke: test_connection_context | fix | async with connect() |
| smoke: test_connection_locations | fix/document | Path/URI handling |
| smoke: test_connection_properties | fix | total_changes / in_transaction sync props |
| smoke: test_context_cursor | fix | async with db.execute() |
| smoke: test_create_function | fix | create_function callback behavior |
| smoke: test_create_function_deterministic | fix | create_function deterministic |
| smoke: test_cursor_on_closed_connection | document | Error when using cursor after close |
| smoke: test_cursor_on_closed_connection_loop | document | Same |
| smoke: test_cursor_return_self | fix | execute/executemany return self |
| smoke: test_emits_warning_when_left_open | document | Warning on connection left open |
| smoke: test_enable_load_extension | fix/document | load_extension / enable_load_extension |
| smoke: test_fetch_all | fix | Row format (lists vs tuples); use aiosqlite_compat=True |
| smoke: test_iterable_cursor | fix | Row format or async iteration |
| smoke: test_iterdump | fix/document | iterdump() return or behavior |
| smoke: test_multi_loop_usage | document | Multiple event loops / connection reuse |
| smoke: test_multiple_connections | fix/document | Multiple connection handling |
| smoke: test_multiple_queries | fix | Query execution / row format |
| smoke: test_set_authorizer_deny_drops | fix | set_authorizer callback |
| smoke: test_set_authorizer_exception_propagation | fix | set_authorizer exception |
| smoke: test_set_progress_handler | fix | set_progress_handler callback |
| smoke: test_set_trace_callback | fix | set_trace_callback |
| smoke: test_stop_without_close | document | stop() is no-op; use close() |

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [migration guide](guides/migration-guide.rst) for details on known differences.

**Common failure reasons:**
- API differences (intentional or unintentional)
- Different error message formats
- Behavioral differences in edge cases
- Row format (lists vs tuples): use `aiosqlite_compat=True` or `row_factory=tuple`
- Test environment (patched imports, shared :memory: isolation)

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
