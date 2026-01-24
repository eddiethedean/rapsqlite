# Research: Accessing Raw SQLite Connection Handle

## Summary

To implement features requiring raw SQLite C API access (extension loading, custom functions, trace callback, authorizer), we need to access the underlying `sqlite3*` handle from sqlx's `SqlitePool`.

## Solution: Using sqlx's `lock_handle()` Method

### Approach 1: Acquire Connection and Lock Handle (Recommended)

**Steps:**
1. Acquire a `SqliteConnection` from the pool using `pool.acquire().await?`
2. Call `connection.lock_handle().await?` to get a `LockedSqliteHandle`
3. Use `locked_handle.as_raw_handle()` to get `NonNull<sqlite3>` (raw SQLite handle)
4. Use `libsqlite3-sys` to call SQLite C API functions

**Code Pattern:**
```rust
use sqlx::sqlite::{SqliteConnection, LockedSqliteHandle};
use libsqlite3_sys::{sqlite3, sqlite3_enable_load_extension};

// In an async function
let mut conn = pool.acquire().await?;
let mut handle = conn.lock_handle().await?;
let raw_handle = handle.as_raw_handle();

// Now we can use raw_handle with libsqlite3-sys
unsafe {
    sqlite3_enable_load_extension(raw_handle.as_ptr(), 1);
}
```

### Key Types

- **`SqlitePool`**: Connection pool (what we currently store)
- **`PoolConnection<Sqlite>`**: Returned by `pool.acquire()`, derefs to `SqliteConnection`
- **`SqliteConnection`**: Individual connection with `lock_handle()` method
- **`LockedSqliteHandle<'a>`**: Locked handle that prevents concurrent access
  - Provides `as_raw_handle()` → `NonNull<sqlite3>`
  - Already implements `set_progress_handler()`!
- **`NonNull<sqlite3>`**: Raw SQLite handle pointer (from `libsqlite3-sys`)

## Implementation Strategy

### Option A: Store Connection Per Feature Call (Simpler)

For each feature method (enable_load_extension, create_function, etc.):
1. Acquire connection from pool
2. Lock handle
3. Perform operation
4. Release connection (returns to pool automatically)

**Pros:**
- Simple, no connection management
- Connection automatically returned to pool
- Works with existing pool architecture

**Cons:**
- Each call acquires/releases connection (small overhead)
- Features are per-connection, so settings apply to that specific connection

### Option B: Cache Locked Handle (More Complex)

Store a `LockedSqliteHandle` in the Connection struct.

**Pros:**
- Single connection used for all raw handle operations
- Settings persist across calls

**Cons:**
- Complex lifetime management
- Need to handle connection dropping
- May block pool if handle held too long

**Recommendation:** Use Option A (acquire per call) for simplicity and safety.

## Required Dependencies

Add to `Cargo.toml`:
```toml
[dependencies]
libsqlite3-sys = "0.28"  # or compatible version with sqlx 0.8
```

**Note:** sqlx 0.8 uses `libsqlite3-sys` internally, so we need to ensure version compatibility.

## Features to Implement

### 1. Extension Loading (`enable_load_extension`)

**SQLite C API:**
- `sqlite3_enable_load_extension(db, onoff)` - Enable/disable extension loading
- `sqlite3_load_extension(db, zFile, zProc, pzErrMsg)` - Load extension

**Implementation:**
```rust
use libsqlite3_sys::{sqlite3_enable_load_extension, sqlite3_load_extension};

async fn enable_load_extension(pool: &SqlitePool, enabled: bool) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let mut handle = conn.lock_handle().await?;
    let raw_handle = handle.as_raw_handle();
    
    unsafe {
        let result = sqlite3_enable_load_extension(
            raw_handle.as_ptr(),
            if enabled { 1 } else { 0 }
        );
        if result != 0 {
            return Err(/* convert SQLite error code */);
        }
    }
    Ok(())
}
```

### 2. Progress Handler (`set_progress_handler`)

**Good News:** `LockedSqliteHandle` already implements `set_progress_handler()`!

**Implementation:**
```rust
async fn set_progress_handler(
    pool: &SqlitePool,
    num_ops: i32,
    callback: impl FnMut() -> bool + Send + 'static
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let mut handle = conn.lock_handle().await?;
    handle.set_progress_handler(num_ops, callback);
    Ok(())
}
```

**Challenge:** Need to bridge Python callable to Rust closure. This requires:
- Storing Python callable in Connection struct
- Creating Rust closure that calls Python function
- Handling GIL and async context

### 3. Custom Functions (`create_function`)

**SQLite C API:**
- `sqlite3_create_function_v2()` - Create scalar function
- `sqlite3_create_function16_v2()` - UTF-16 version

**Implementation:**
```rust
use libsqlite3_sys::sqlite3_create_function_v2;

// Need to implement callback that calls Python function
// This is complex - requires storing Python callable and calling it from C callback
```

**Challenge:** SQLite expects C callbacks, but we need to call Python functions. This requires:
- Storing Python callables in Connection struct
- Creating C-compatible callback wrappers
- Managing GIL during callback execution
- Handling async context (callbacks are synchronous in SQLite)

### 4. Trace Callback (`set_trace_callback`)

**SQLite C API:**
- `sqlite3_trace_v2()` - Modern trace callback (replaces deprecated `sqlite3_trace()`)

**Implementation:**
```rust
use libsqlite3_sys::sqlite3_trace_v2;

// Similar challenges as create_function - need Python callback bridge
```

### 5. Authorizer (`set_authorizer`)

**SQLite C API:**
- `sqlite3_set_authorizer()` - Set authorization callback

**Implementation:**
```rust
use libsqlite3_sys::sqlite3_set_authorizer;

// Similar challenges - Python callback bridge required
```

## Python Callback Bridge Pattern

For features requiring Python callbacks (progress handler, custom functions, trace, authorizer):

### Pattern 1: Store Python Callable, Call from Rust

```rust
struct Connection {
    // ... existing fields ...
    progress_handler: Arc<StdMutex<Option<Py<PyAny>>>>,
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    // etc.
}

// In set_progress_handler:
fn set_progress_handler(&self, num_ops: i32, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    // Store callback
    *self.progress_handler.lock().unwrap() = callback;
    
    // Create Rust closure that calls Python
    let handler = Arc::clone(&self.progress_handler);
    let closure = move || {
        Python::with_gil(|py| {
            if let Some(cb) = handler.lock().unwrap().as_ref() {
                let bound = cb.bind(py);
                if let Ok(result) = bound.call0() {
                    result.extract::<bool>().unwrap_or(true)
                } else {
                    true
                }
            } else {
                true
            }
        })
    };
    
    // Set on locked handle
    // ...
}
```

### Pattern 2: Use sqlx's Built-in Support

For `set_progress_handler`, sqlx's `LockedSqliteHandle` already supports it with Rust closures. We just need to bridge Python callables to Rust closures.

## Alternative: Direct rusqlite Integration

**Consideration:** Could we use rusqlite alongside sqlx?

**Answer:** No, not recommended. Both sqlx and rusqlite link `libsqlite3-sys`, and having both creates a semver hazard. Only one version can exist in the dependency graph.

## Recommended Implementation Order

1. **Progress Handler** - Easiest, sqlx already supports it
2. **Extension Loading** - Straightforward C API calls
3. **Trace Callback** - Medium complexity (Python callback bridge)
4. **Authorizer** - Medium complexity (Python callback bridge)
5. **Custom Functions** - Most complex (Python callback bridge + function registration)

## Code Structure

### Helper Function Pattern

```rust
/// Acquire connection and lock handle for raw SQLite operations
async fn with_locked_handle<F, R>(
    pool: &SqlitePool,
    f: F,
) -> Result<R, PyErr>
where
    F: FnOnce(&mut LockedSqliteHandle<'_>) -> Result<R, PyErr>,
{
    let mut conn = pool.acquire().await
        .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {}", e)))?;
    
    let mut handle = conn.lock_handle().await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
    
    f(&mut handle)
}
```

### Usage Example

```rust
fn enable_load_extension(&self, enabled: bool) -> PyResult<Py<PyAny>> {
    let pool = Arc::clone(&self.pool);
    let path = self.path.clone();
    
    Python::attach(|py| {
        let future = async move {
            let pool_clone = get_or_create_pool(&path, &pool, &pragmas).await?;
            
            with_locked_handle(&pool_clone, |handle| {
                let raw_handle = handle.as_raw_handle();
                unsafe {
                    let result = libsqlite3_sys::sqlite3_enable_load_extension(
                        raw_handle.as_ptr(),
                        if enabled { 1 } else { 0 }
                    );
                    if result != libsqlite3_sys::SQLITE_OK {
                        return Err(OperationalError::new_err(
                            format!("Failed to enable load extension: SQLite error {}", result)
                        ));
                    }
                }
                Ok(())
            })
        };
        future_into_py(py, future).map(|bound| bound.unbind())
    })
}
```

## References

- [sqlx SqliteConnection docs](https://docs.rs/sqlx/latest/sqlx/sqlite/struct.SqliteConnection.html)
- [sqlx LockedSqliteHandle docs](https://docs.rs/sqlx/latest/sqlx/sqlite/struct.LockedSqliteHandle.html)
- [libsqlite3-sys docs](https://docs.rs/libsqlite3-sys)
- [SQLite C API - Extension Loading](https://sqlite.org/c3ref/load_extension.html)
- [SQLite C API - Custom Functions](https://sqlite.org/c3ref/create_function.html)
- [SQLite C API - Trace Callback](https://sqlite.org/c3ref/trace_v2.html)
- [SQLite C API - Authorizer](https://sqlite.org/c3ref/set_authorizer.html)
