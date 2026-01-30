# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: 2026-01-29 15:55:35
**rapsqlite Version**: 0.3.0-dev
**Python Version**: 3.12.7

## Summary

- **Total Test Files**: 4
- **✅ Passed**: 0
- **❌ Failed**: 2
- **⏭️  Skipped**: 2

## Passed Tests

*No tests passed*

## Failed Tests

- `perf.py`
- `smoke.py`

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [MIGRATION.md](MIGRATION.md) for details on known differences.

**Common failure reasons:**
- API differences (intentional or unintentional)
- Different error message formats
- Behavioral differences in edge cases
- Missing features in rapsqlite

**Next steps:**
1. Review failed tests to identify compatibility gaps
2. Fix compatibility issues where possible
3. Document intentional differences in MIGRATION.md

## Skipped Tests

- `__main__.py`
- `helpers.py`

## Notes

- Tests were run by patching aiosqlite imports to use rapsqlite
- Some failures may be due to intentional differences (see [MIGRATION.md](MIGRATION.md))
- Some failures may indicate areas for improvement in rapsqlite compatibility
- This is a compatibility validation exercise, not a requirement for 100% pass rate
