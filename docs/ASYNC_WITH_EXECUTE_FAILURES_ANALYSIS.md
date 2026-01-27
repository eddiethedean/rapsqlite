# Analysis: `async with db.execute(...)` Pattern Test Failures

## Executive Summary

Two test failures remain after implementing the `async with db.execute(...)` pattern for aiosqlite compatibility:

1. **Parameterized Query Failure**: Named parameters return 0 rows instead of 1
2. **Transaction Timeout Failure**: Queries within transactions timeout with "pool timed out"

Both issues stem from the `Cursor` class not properly using stored processed parameters and transaction connections when executing queries.

## Problem 1: Parameterized Query Returns Empty Results

### Test Case
```python
async with conn.execute("SELECT * FROM test WHERE value = :value", {"value": "hello"}) as cursor:
    rows = await cursor.fetchall()
    assert len(rows) == 1  # FAILS: gets 0 rows
```

### Expected Behavior
- Query: `SELECT * FROM test WHERE value = :value`
- Parameters: `{"value": "hello"}`
- Expected: 1 row with value "hello"
- Actual: 0 rows (empty result)

### Current Implementation Flow

1. **Connection.execute()** (line ~1807-1865):
   - Processes parameters: `process_named_parameters()` converts `:value` → `?` and extracts `SqliteParam::Text("hello")`
   - Stores **processed** query and parameters in cursor:
     ```rust
     processed_query: Some(processed_query.clone()),  // "SELECT * FROM test WHERE value = ?"
     processed_params: Some(param_values.clone()),     // [SqliteParam::Text("hello")]
     ```
   - Also stores **original** query and parameters:
     ```rust
     query: original_query.clone(),  // "SELECT * FROM test WHERE value = :value"
     parameters: Arc<StdMutex<Option<Py<PyAny>>>>,  // Original dict {"value": "hello"}
     ```

2. **Cursor.fetchall()** (line ~6049-6088):
   - Checks if stored processed parameters exist
   - If they exist, uses them directly
   - If not, re-processes from original parameters

### Root Cause Hypothesis

The stored processed parameters **should** be used, but something is preventing them from working correctly. Possible issues:

1. **Parameter binding mismatch**: The stored `processed_params` might not be binding correctly to the `processed_query`
2. **Query execution path**: The query might be using the wrong connection or execution method
3. **Parameter order issue**: The parameters might be in the wrong order when binding

### Evidence

- Positional parameters (`?`) work correctly
- Named parameters (`:value`) fail
- `Connection.fetch_all()` with named parameters works correctly
- The difference: `fetch_all()` processes parameters directly, while `Cursor.fetchall()` uses stored parameters

### Debugging Findings

From previous debugging attempts:
- `process_named_parameters()` correctly converts `:value` → `?` and extracts `"hello"`
- The stored parameters are present in the cursor
- The fallback re-processing path also fails (returns empty results)
- This suggests the issue is in how parameters are **bound** to the query, not in parameter processing

## Problem 2: Transaction Queries Timeout

### Test Case
```python
async with conn.transaction():
    async with conn.execute("INSERT INTO test (value) VALUES (1)") as _:
        pass
    
    # This fails with timeout
    async with conn.execute("SELECT * FROM test") as cursor:
        rows = await cursor.fetchall()  # FAILS: "pool timed out"
```

### Expected Behavior
- Transaction is active
- SELECT query should use the transaction connection
- Expected: Returns 1 row
- Actual: Timeout error "pool timed out while waiting for an open connection"

### Current Implementation Flow

1. **Transaction starts**:
   - `Connection.transaction()` acquires a connection from the pool
   - Stores it in `transaction_connection: Arc<Mutex<Option<PoolConnection>>>`
   - Sets `transaction_state: Arc<Mutex<TransactionState>>` to `Active`

2. **Cursor creation in Connection.execute()** (line ~1857):
   - Clones `transaction_state` and `transaction_connection` Arcs:
     ```rust
     transaction_state: Arc::clone(&transaction_state),
     transaction_connection: Arc::clone(&transaction_connection),
     ```

3. **Cursor.fetchall()** (line ~6090-6111):
   - Checks transaction state:
     ```rust
     let in_transaction = {
         let g = transaction_state.lock().await;
         *g == TransactionState::Active
     };
     ```
   - If `in_transaction` is true, should use `transaction_connection`
   - Otherwise, tries to get connection from pool → **TIMEOUT**

### Root Cause Hypothesis

The transaction state check is likely returning `false` even when a transaction is active. Possible causes:

1. **Arc sharing issue**: The `transaction_state` Arc in the cursor might not be the same instance as in the connection
2. **Timing issue**: The transaction might not be fully initialized when the cursor is created
3. **State synchronization**: The transaction state might be reset or not properly shared

### Evidence

- The timeout suggests the pool has no available connections (the transaction connection is holding the only one)
- The error occurs specifically in `Cursor.fetchall()`, not in `Connection.fetch_all()`
- `Connection.fetch_all()` correctly uses transaction connections (lines 2106-2169)

## Technical Analysis

### Architecture Overview

The `async with db.execute(...)` pattern works as follows:

```
Connection.execute(query, params)
    ↓
Process parameters → (processed_query, processed_params)
    ↓
Create ExecuteContextManager with Cursor
    ↓
ExecuteContextManager.__aenter__()
    ↓
  - For SELECT: Return cursor (lazy execution)
  - For non-SELECT: Execute immediately, return cursor
    ↓
User: await cursor.fetchall()
    ↓
Cursor.fetchall()
    ↓
  - Use stored processed_query and processed_params
  - Check transaction state
  - Use appropriate connection (transaction > callback > pool)
  - Execute query and return results
```

### Key Components

1. **ExecuteContextManager**: Wraps the cursor and handles `__aenter__`/`__aexit__`
2. **Cursor**: Stores query, parameters, and connection state
3. **Connection priority logic**: Transaction > Callbacks > Pool

### Current State of Cursor Struct

The `Cursor` struct (line ~5745) includes:
- `processed_query: Option<String>` - Stored processed query
- `processed_params: Option<Vec<SqliteParam>>` - Stored processed parameters
- `transaction_state: Arc<Mutex<TransactionState>>` - Transaction state
- `transaction_connection: Arc<Mutex<Option<PoolConnection>>>` - Transaction connection
- All callback-related fields

### Connection Priority Implementation

The pattern used in `Connection.fetch_all()` (lines 2106-2169):
```rust
if in_transaction {
    // Use transaction connection
} else if has_callbacks {
    // Use callback connection
} else {
    // Use pool
}
```

This same pattern is implemented in `Cursor.fetchall()` (lines 6090-6130), but it's not working correctly.

## Root Cause Analysis

### Issue 1: Parameter Binding

**Hypothesis**: The stored `processed_params` are correct, but when they're used with `bind_and_fetch_all_on_connection()`, the parameter binding fails silently or binds incorrectly.

**Evidence**:
- Stored parameters exist and are correct
- The query string is correct (`?` placeholders)
- But the execution returns empty results

**Possible causes**:
1. Parameter order mismatch when binding
2. Parameter type conversion issue
3. Query execution using wrong connection context

### Issue 2: Transaction State Detection

**Hypothesis**: The `transaction_state` Arc in the cursor is not properly sharing state with the connection, or the state check happens before the transaction is fully initialized.

**Evidence**:
- Transaction connection exists (otherwise INSERT would fail)
- But `in_transaction` check returns false
- This causes fallback to pool, which times out

**Possible causes**:
1. Arc cloning creates separate state (unlikely, but possible)
2. Transaction state not set to `Active` when cursor is created
3. Race condition: cursor created before transaction fully initialized

## Proposed Solutions

### Solution 1: Fix Parameter Binding

**Approach**: Verify and fix how stored parameters are bound to queries.

1. **Add verification logging**:
   - Log the stored `processed_query` and `processed_params` before execution
   - Log the actual bound parameters during execution
   - Compare with what `Connection.fetch_all()` does

2. **Fix parameter binding**:
   - Ensure `bind_and_fetch_all_on_connection()` receives parameters in correct order
   - Verify parameter types match what sqlx expects
   - Check if parameter count matches placeholder count

3. **Fallback to re-processing**:
   - If stored parameters fail, fall back to re-processing from original parameters
   - This ensures backward compatibility

### Solution 2: Fix Transaction State Detection

**Approach**: Ensure transaction state is properly shared and detected.

1. **Verify Arc sharing**:
   - Add logging to verify `transaction_state` Arc pointers are the same
   - Verify state is `Active` when cursor tries to use it

2. **Fix state detection**:
   - Ensure transaction state is set to `Active` before cursor creation
   - Add explicit check that `transaction_connection` is not `None` when state is `Active`

3. **Add better error messages**:
   - If transaction state is `Active` but connection is `None`, provide clear error
   - This will help identify the exact failure point

### Solution 3: Unified Fix

**Approach**: Refactor to use the same execution path as `Connection.fetch_all()`.

1. **Extract common execution logic**:
   - Create a helper function that handles connection priority and parameter binding
   - Use this in both `Connection.fetch_all()` and `Cursor.fetchall()`

2. **Ensure consistency**:
   - Both paths should use identical logic for:
     - Parameter processing
     - Connection selection
     - Query execution

## Testing Strategy

### Debug Scripts

1. **`debug_parameterized.py`**:
   ```python
   # Test parameter processing in isolation
   # Compare stored vs re-processed parameters
   # Verify parameter binding
   ```

2. **`debug_transaction.py`**:
   ```python
   # Test transaction state detection
   # Verify Arc sharing
   # Check connection availability
   ```

### Verification Steps

1. Add temporary debug logging to trace:
   - Parameter storage and retrieval
   - Transaction state values
   - Connection selection logic

2. Run isolated tests:
   - Test parameter processing separately
   - Test transaction connection separately

3. Compare with working code:
   - Compare `Cursor.fetchall()` with `Connection.fetch_all()`
   - Identify differences in execution path

## Implementation Priority

1. **High Priority**: Fix transaction timeout (blocks transaction usage)
2. **High Priority**: Fix parameterized queries (blocks common use case)

Both issues are critical for aiosqlite compatibility and should be fixed together.

## Success Criteria

- `test_async_with_execute_parameterized` passes (returns 1 row)
- `test_async_with_execute_in_transaction` passes (uses transaction connection)
- All other tests still pass (no regressions)
- No debug code left in production

## Next Steps

1. Add comprehensive debug logging to identify exact failure points
2. Create minimal reproduction scripts for both issues
3. Compare execution paths between working and failing code
4. Implement fixes based on findings
5. Remove debug code and verify all tests pass
