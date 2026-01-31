# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-30 21:16:22
**rapsqlite Version**: 0.3.0-dev
**Python Version**: 3.10.19

## Summary

- **Total Test Files**: 2
- **✅ Passed**: 0
- **❌ Failed**: 2
- **⏭️  Skipped**: 0

## Passed Tests

*No tests passed*

## Failed Tests

- `perf.py`
- `smoke.py`

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [migration guide](guides/migration-guide.rst) for details on known differences.

**Common failure reasons:**
- API differences (intentional or unintentional)
- Different error message formats
- Behavioral differences in edge cases
- Missing features in rapsqlite

**Next steps:**
1. Review failed tests to identify compatibility gaps
2. Fix compatibility issues where possible
3. Document intentional differences in the migration guide

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
