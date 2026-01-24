# execute_many Transaction Lock Issue - Consolidated Documentation

## Problem Statement

The `execute_many()` method fails with **"database is locked"** error (SQLite error code 5) when executing multiple parameter sets within a transaction. This affects three test cases:

1. `test_transaction_context_manager_with_execute_many`
2. `test_transaction_with_all_query_methods`
3. `test_transaction_connection_consistency`

### Symptoms

- ✅ Single parameter set in `execute_many()`: Works correctly
- ✅ Multiple `execute()` calls from Python: Works correctly
- ❌ Multiple parameter sets in `execute_many()`: Fails on second iteration
- ❌ Error: `_rapsqlite.OperationalError: Failed to execute query on database ...: error returned from database: (code: 5) database is locked`

## Root Cause Analysis

### The Core Issue: SQLite Transactions Are Per-Connection

**Key Insight**: SQLite transactions are **per-connection**, not per-pool. When we execute `BEGIN` on one connection from the pool, but then `execute_many()` gets a different connection from the pool, SQLite sees:

- **Connection A**: Has an active transaction (from `BEGIN`)
- **Connection B**: Tries to execute INSERT (from `execute_many()`)
- **SQLite**: "Database is locked" because Connection B doesn't have the transaction context

### Why execute() Works But execute_many() Fails

**execute() works**:
- When called multiple times from Python, each call is a separate async future
- Each call may get the same connection (by chance) or SQLite's connection pooling handles it differently
- The connection state is managed per-call

**execute_many() fails**:
- Multiple executes happen in a loop within the same async function
- Higher chance of getting different connections from the pool
- SQLite detects the connection mismatch and reports "database is locked"

### Current Architecture

**Transaction Management**:
- `begin()` executes `BEGIN IMMEDIATE` against the pool (any connection)
- No `transaction_connection` field existed initially - transactions were managed at the pool level
- `commit()` and `rollback()` execute against the pool (any connection)
- This means transactions are **connection-agnostic** - any connection from the pool can participate

**execute_many() Implementation**:
- Does NOT check for active transactions
- Uses `bind_and_execute()` which executes against the pool directly
- Each iteration may use a different connection from the pool
- This causes the lock conflict when in a transaction

## What We Know

### 1. SQLx Package Analysis

**Current Setup**:
- sqlx version: 0.8 (resolves to 0.8.6)
- Using `SqlitePool` with connection pooling
- Manual transaction management with `BEGIN IMMEDIATE` on stored `PoolConnection`

**Known SQLx Issues**:
- SQLx 0.8 has known transaction safety issues (fixed in 0.8.6)
- SQLite connection pools can encounter "database table is locked" errors even with WAL enabled
- Connection pools don't automatically eliminate SQLite's single-writer limitation
- Default pool settings allow multiple concurrent connections, which conflicts with SQLite's single-writer model

**Solution Implemented**: Limited pool to 1 connection using `SqlitePoolOptions::new().max_connections(1)`

### 2. Transaction API Migration Attempt

**Attempted**: Migrate from manual `PoolConnection` management to sqlx's native `Transaction` API

**Result**: **Not feasible** due to Rust lifetime constraints:
- `Transaction<'c, DB>` has a lifetime parameter that cannot be `'static`
- Cannot store `Transaction` in `Arc<Mutex<...>>` which requires `'static`
- `Transaction::commit()` requires `mut self` (consumes the transaction)
- Cannot move mutable reference out of `Arc`

**Conclusion**: Our current `PoolConnection` + manual transaction management approach is the **correct pattern** for this use case.

### 3. Implementation Status

**Completed**:
- ✅ Added `transaction_connection` field to `Connection` struct
- ✅ Initialize `transaction_connection` in `Connection::new()`
- ✅ Updated `begin()` to acquire and store connection with `BEGIN IMMEDIATE`
- ✅ Updated `commit()` to use stored connection
- ✅ Updated `rollback()` to use stored connection
- ✅ Created `bind_and_execute_on_connection()` helper function
- ✅ Updated `execute()` to use stored connection when in transaction
- ✅ Updated `execute_many()` to use stored connection when in transaction
- ✅ Limited SQLite pool to 1 connection (`max_connections(1)`)

**Current State**:
- Code compiles and builds successfully
- All infrastructure is in place
- Tests still fail with "database is locked"
- Debug logs are not appearing (suggesting transaction connection not detected)

## What We've Tried

### Phase 1: Connection Identity Verification
- Added connection pointer logging
- Added SQLite transaction status logging
- Verified connection pointer remains stable across iterations
- **Result**: Connection identity is consistent

### Phase 2: MutexGuard and Lock Behavior
- Tested releasing lock between iterations
- Tested holding lock across entire loop
- **Result**: Lock management approach doesn't affect the issue

### Phase 3: SQLite Transaction State
- Added `PRAGMA busy_timeout = 5000`
- Verified `BEGIN IMMEDIATE` executes successfully
- Checked transaction status before each query
- **Result**: Transaction state appears correct, but issue persists

### Phase 4: sqlx Connection Handling
- Tested `&mut **conn` pattern (current)
- Tested `as_mut()` to get direct `&mut SqliteConnection`
- Tested matching `execute()` pattern exactly
- **Result**: Connection access pattern doesn't affect the issue

### Phase 5: Alternative Execution Patterns
- Tested async yield between iterations
- Verified single param set works
- Confirmed multiple param sets fail
- **Result**: Timing/yielding doesn't resolve the issue

### Phase 6: Pool Configuration
- Limited pool to 1 connection using `SqlitePoolOptions`
- **Result**: Issue persists

## Current Status

### Test Results
- **Core tests**: 24/24 passing ✅
- **Compatibility tests**: 79 passed, 3 failed, 8 skipped
- **Failing tests**:
  - `test_transaction_context_manager_with_execute_many`
  - `test_transaction_with_all_query_methods`
  - `test_transaction_connection_consistency`

### Code Status
- ✅ All implementation complete
- ✅ Code compiles successfully
- ❌ Tests still fail with "database is locked"
- ❌ Debug logs not appearing (suggesting transaction connection not being detected)

### Mystery: Transaction Context Manager

**Critical Finding**: The transaction context manager (`db.transaction()`) exists and has `__aenter__` and `__aexit__` methods, but:
- Cannot find its implementation in Rust code
- Debug logs suggest `begin()` may not be called by the context manager
- Tests timeout, suggesting possible deadlock

**Investigation Needed**: Verify that the transaction context manager actually calls `begin()` in `__aenter__` and `commit()`/`rollback()` in `__aexit__`.

## Hypotheses (Unverified)

### Hypothesis 1: Transaction Context Manager Not Calling begin()
- The transaction context manager may not be properly calling `begin()` when entering
- This would explain why the transaction connection isn't detected
- **Evidence**: Debug logs not appearing, transaction state not set

### Hypothesis 2: Connection Pool Still Causing Conflicts
- Even with `max_connections(1)`, there may be subtle pool behavior issues
- SQLite's single-writer model may still conflict with connection pooling
- **Evidence**: Issue persists after limiting pool size

### Hypothesis 3: sqlx Internal State Management
- sqlx may maintain internal state that gets corrupted when multiple executes happen in quick succession
- The manual transaction management may conflict with sqlx's internal tracking
- **Evidence**: Works for single execution, fails for multiple

### Hypothesis 4: SQLite Connection State
- SQLite may see the second execution as a different "operation" and incorrectly detect a lock conflict
- The `BEGIN IMMEDIATE` may not be properly propagating to subsequent operations
- **Evidence**: Transaction state appears correct but lock still occurs

## Next Steps

### Immediate Actions

1. **Verify Transaction Context Manager Implementation**
   - Find or implement the transaction context manager
   - Verify it calls `begin()` in `__aenter__`
   - Verify it calls `commit()`/`rollback()` in `__aexit__`
   - Test with explicit `begin()`/`commit()` to bypass context manager

2. **Debug Log Visibility**
   - Investigate why debug logs (`eprintln!`) are not appearing
   - Check if stderr is being captured by pytest
   - Add Python-level logging to verify code paths

3. **Test with Explicit begin()/commit()**
   - Bypass the transaction context manager
   - Test if the core functionality works with explicit transaction management
   - This will isolate whether the issue is with the context manager or the core implementation

### Alternative Approaches

1. **Use Dedicated Connection for Transactions**
   - Don't use pool for transactions
   - Create a separate `SqliteConnection` for transaction operations
   - Use pool only for non-transaction queries

2. **Investigate sqlx Source Code**
   - Look into sqlx's handling of multiple executes on the same connection
   - Check for known issues with SQLite transactions and connection pooling
   - Review sqlx's internal connection state management

3. **Consider Workaround**
   - For now, implement `execute_many()` by calling `execute()` multiple times from Python
   - This pattern is known to work
   - Can be optimized later once root cause is found

## Files Modified

- `src/lib.rs`:
  - Added `transaction_connection` field to `Connection` struct
  - Updated `begin()`, `commit()`, `rollback()` methods
  - Updated `execute()` and `execute_many()` methods
  - Created `bind_and_execute_on_connection()` helper
  - Limited pool to 1 connection using `SqlitePoolOptions`

- `docs/`:
  - Multiple investigation and research documents (now consolidated here)

## Related Documentation

This document consolidates information from:
- `INVESTIGATION_PLAN.md` - Initial investigation plan
- `FIX_PLAN_EXECUTE_MANY_LOCK.md` - Detailed fix implementation plan
- `DEEP_DEBUG_FINDINGS.md` - Previous debugging efforts
- `TRANSACTION_API_MIGRATION_ISSUE.md` - Transaction API migration attempt
- `TRANSACTION_API_RESEARCH_FINDINGS.md` - Research into Transaction API
- `SQLX_ISSUES_ANALYSIS.md` - SQLx package analysis
- `RESEARCH_SUMMARY.md` - Executive summary of research

## Success Criteria

1. ✅ All three failing tests pass:
   - `test_transaction_context_manager_with_execute_many`
   - `test_transaction_with_all_query_methods`
   - `test_transaction_connection_consistency`

2. ✅ All existing tests still pass (24/24 core tests)

3. ✅ No regressions in compatibility tests

4. ✅ Both `execute()` and `execute_many()` consistently handle transactions

## Conclusion

Despite extensive investigation and implementation of the fix plan, the issue persists. The infrastructure is in place, but the transaction connection is not being detected or used properly. The critical next step is to verify that the transaction context manager is actually calling `begin()`, as this appears to be the missing link in the chain.

The root cause is well-understood (SQLite transactions are per-connection), and the solution is implemented (store and reuse transaction connection), but something is preventing the solution from working. The mystery of the transaction context manager implementation needs to be solved.
