//! Pool creation and connection-management helpers.
//!
//! Uses a path-based global pool registry so multiple Connection objects
//! connecting to the same database path share one SqlitePool, improving
//! concurrent operation performance (e.g. many `connect(path)` calls).

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::into_future;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::types::{ProgressHandler, UserAggregates, UserCollations, UserFunctions};
use crate::OperationalError;

/// Minimum pool size when creating a shared pool so many concurrent
/// Connection objects to the same path can acquire connections.
const SHARED_POOL_MIN_CONNECTIONS: u32 = 25;

/// Global registry: path -> SqlitePool. Connections to the same path share one pool.
fn global_registry() -> &'static StdMutex<HashMap<String, SqlitePool>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, SqlitePool>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Create a helpful error message for pool acquisition failures.
pub(crate) fn pool_acquisition_error(
    path: &str,
    error: &sqlx::Error,
    pool_size: Option<usize>,
    timeout: Option<u64>,
) -> PyErr {
    let error_str = error.to_string();
    let is_timeout = error_str.contains("timeout") || error_str.contains("timed out");

    let mut msg = format!("Failed to acquire connection from pool at {path}: {error_str}");

    if is_timeout {
        msg.push_str("\n\nPossible solutions:");
        msg.push_str("\n  - Increase pool_size (current: ");
        msg.push_str(
            &pool_size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "1 (default)".to_string()),
        );
        msg.push(')');
        msg.push_str("\n  - Increase connection_timeout (current: ");
        msg.push_str(
            &timeout
                .map(|t| format!("{}s", t))
                .unwrap_or_else(|| "30s (default)".to_string()),
        );
        msg.push(')');
        msg.push_str("\n  - Ensure connections are properly released (use async context managers)");
        msg.push_str("\n  - Check for long-running transactions that hold connections");
    }

    OperationalError::new_err(msg)
}

/// Apply this Connection's PRAGMAs to a pooled connection so session pragmas
/// (e.g. set via set_pragma) are in effect when using a shared pool.
pub(crate) async fn apply_pragmas_to_connection(
    conn: &mut PoolConnection<sqlx::Sqlite>,
    pragmas: &[(String, String)],
    path: &str,
) -> Result<(), PyErr> {
    for (name, value) in pragmas {
        let pragma_query = format!("PRAGMA {name} = {value}");
        sqlx::query(&pragma_query)
            .execute(&mut **conn)
            .await
            .map_err(|e| crate::map_sqlx_error(e, path, &pragma_query))?;
    }
    Ok(())
}

/// Acquire a connection from the pool and apply the Connection's pragmas to it.
/// Use this whenever a Connection acquires a connection so set_pragma and
/// connect(pragmas=...) are respected with a shared pool.
pub(crate) async fn acquire_with_pragmas(
    pool: &SqlitePool,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    path: &str,
    pool_size_val: Option<usize>,
    timeout_val: Option<u64>,
) -> Result<PoolConnection<sqlx::Sqlite>, PyErr> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| pool_acquisition_error(path, &e, pool_size_val, timeout_val))?;
    let pragmas_list = pragmas.lock().unwrap().clone();
    apply_pragmas_to_connection(&mut conn, &pragmas_list, path).await?;
    Ok(conn)
}

/// Helper to get or create pool and apply PRAGMAs.
/// Uses a global path-based registry so connections to the same path share one pool.
pub(crate) async fn get_or_create_pool(
    path: &str,
    pool: &Arc<Mutex<Option<SqlitePool>>>,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<SqlitePool, PyErr> {
    // Fast path: this connection already has a pool (from registry or prior creation).
    {
        let pool_guard = pool.lock().await;
        if let Some(ref p) = *pool_guard {
            return Ok(p.clone());
        }
    }

    let registry = global_registry();

    // Check global registry for an existing pool for this path.
    let from_registry = {
        let reg = registry.lock().unwrap();
        reg.get(path).cloned()
    };
    if let Some(shared_clone) = from_registry {
        let mut pool_guard = pool.lock().await;
        *pool_guard = Some(shared_clone.clone());
        return Ok(shared_clone);
    }

    // No pool for this path: create one, then register or use existing (race).
    let max_conn = {
        let g = pool_size.lock().unwrap();
        (g.unwrap_or(1).max(1)) as u32
    };
    let timeout_secs = {
        let g = connection_timeout_secs.lock().unwrap();
        *g
    };
    let idle_secs = {
        let g = idle_timeout_secs.lock().unwrap();
        *g
    };
    // Shared pool must support many concurrent connections to the same path.
    let max_conn = max_conn.max(SHARED_POOL_MIN_CONNECTIONS);
    let mut opts = SqlitePoolOptions::new().max_connections(max_conn);
    let timeout = timeout_secs.unwrap_or(30);
    opts = opts.acquire_timeout(Duration::from_secs(timeout));
    if let Some(idle) = idle_secs {
        opts = opts.idle_timeout(Some(Duration::from_secs(idle)));
    }
    let new_pool = opts.connect(&format!("sqlite:{path}")).await.map_err(|e| {
        OperationalError::new_err(format!("Failed to connect to database at {path}: {e}"))
    })?;

    let pragmas_list = {
        let pragmas_guard = pragmas.lock().unwrap();
        pragmas_guard.clone()
    };
    for (name, value) in pragmas_list {
        let pragma_query = format!("PRAGMA {name} = {value}");
        sqlx::query(&pragma_query)
            .execute(&new_pool)
            .await
            .map_err(|e| crate::map_sqlx_error(e, path, &pragma_query))?;
    }

    let to_use = {
        let mut reg = registry.lock().unwrap();
        if let Some(existing) = reg.get(path) {
            Some(existing.clone())
        } else {
            reg.insert(path.to_string(), new_pool.clone());
            None
        }
    };
    match to_use {
        Some(existing) => {
            let mut pool_guard = pool.lock().await;
            *pool_guard = Some(existing.clone());
            Ok(existing)
        }
        None => {
            let mut pool_guard = pool.lock().await;
            *pool_guard = Some(new_pool.clone());
            Ok(new_pool)
        }
    }
}

/// Helper to ensure callback connection exists.
/// This acquires a connection from the pool and stores it for callback installation.
/// The connection is stored in the callback_connection mutex and should be accessed via that mutex.
/// Note: Accessing the raw sqlite3* handle from PoolConnection requires further research
/// into sqlx 0.8's API. This is a known limitation that needs to be resolved.
pub(crate) async fn ensure_callback_connection(
    path: &str,
    pool: &Arc<Mutex<Option<SqlitePool>>>,
    callback_connection: &Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<(), PyErr> {
    let mut callback_guard = callback_connection.lock().await;
    if callback_guard.is_none() {
        // Get or create pool first
        let pool_clone = get_or_create_pool(
            path,
            pool,
            pragmas,
            pool_size,
            connection_timeout_secs,
            idle_timeout_secs,
        )
        .await?;

        // Acquire a connection from the pool
        let pool_size_val = {
            let g = pool_size.lock().unwrap();
            *g
        };
        let timeout_val = {
            let g = connection_timeout_secs.lock().unwrap();
            *g
        };
        let pool_conn =
            acquire_with_pragmas(&pool_clone, pragmas, path, pool_size_val, timeout_val).await?;

        *callback_guard = Some(pool_conn);
    }
    Ok(())
}

/// Execute init_hook if it hasn't been called yet.
/// This should be called from the first operation method that uses the pool.
pub(crate) async fn execute_init_hook_if_needed(
    init_hook: &Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_called: &Arc<StdMutex<bool>>,
    connection: Py<crate::Connection>,
) -> Result<(), PyErr> {
    // Check if init_hook has already been called
    let already_called = {
        let guard = init_hook_called.lock().unwrap();
        *guard
    };

    if already_called {
        return Ok(());
    }

    // Check if init_hook is set and call it if needed
    // Note: Python::with_gil is used here because this is a sync helper function
    // called from async contexts. The deprecation warning is acceptable here.
    #[allow(deprecated)]
    let hook_opt: Option<Py<PyAny>> = Python::with_gil(|py| {
        let guard = init_hook.lock().unwrap();
        guard.as_ref().map(|h| h.clone_ref(py))
    });

    if let Some(hook) = hook_opt {
        // Mark as called before execution (to avoid re-entry if hook calls other methods)
        {
            let mut guard = init_hook_called.lock().unwrap();
            *guard = true;
        }

        // Call the hook with the Connection object and await the coroutine
        // Note: Python::with_gil is used here because this is a sync helper function
        // called from async contexts. The deprecation warning is acceptable here.
        #[allow(deprecated)]
        let coro_future = Python::with_gil(|py| -> PyResult<_> {
            let hook_bound = hook.bind(py);
            let conn_bound = connection.bind(py);

            // Call the hook with Connection as argument
            let coro = hook_bound
                .call1((conn_bound,))
                .map_err(|e| OperationalError::new_err(format!("Failed to call init_hook: {e}")))?;

            // Convert Python coroutine to Rust future (into_future expects Bound)
            into_future(coro).map_err(|e| {
                OperationalError::new_err(format!(
                    "Failed to convert init_hook coroutine to future: {e}"
                ))
            })
        })?;

        // Await the future
        coro_future.await.map_err(|e| {
            OperationalError::new_err(format!("init_hook raised an exception: {e}"))
        })?;
    }

    Ok(())
}

/// Ensure the Connection has a session connection from the pool (acquire and store if None).
/// Used to reuse one connection per Connection for many queries when not in a transaction
/// and not using callbacks, matching aiosqlite behavior and improving concurrent-read performance.
pub(crate) async fn ensure_session_connection(
    path: &str,
    pool: &Arc<Mutex<Option<SqlitePool>>>,
    session_connection: &Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<(), PyErr> {
    let mut guard = session_connection.lock().await;
    if guard.is_none() {
        let pool_clone = get_or_create_pool(
            path,
            pool,
            pragmas,
            pool_size,
            connection_timeout_secs,
            idle_timeout_secs,
        )
        .await?;
        let pool_size_val = *pool_size.lock().unwrap();
        let timeout_val = *connection_timeout_secs.lock().unwrap();
        let conn =
            acquire_with_pragmas(&pool_clone, pragmas, path, pool_size_val, timeout_val).await?;
        *guard = Some(conn);
    }
    Ok(())
}

/// Release the session connection (return to pool). Call on close() and when starting a transaction.
pub(crate) async fn release_session_connection(
    session_connection: &Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
) {
    let mut guard = session_connection.lock().await;
    let _ = guard.take();
}

/// Check if any callbacks are currently set.
pub(crate) fn has_callbacks(
    load_extension_enabled: &Arc<StdMutex<bool>>,
    user_functions: &UserFunctions,
    user_aggregates: &UserAggregates,
    user_collations: &UserCollations,
    trace_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    authorizer_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: &ProgressHandler,
) -> bool {
    // Safety: StdMutex::lock() only fails if the mutex is poisoned (another thread panicked).
    // In Python's GIL context and with proper error handling, this is extremely unlikely.
    // These are read-only operations, so unwrap() is acceptable.
    let load_ext = *load_extension_enabled.lock().unwrap();
    let has_functions = !user_functions.lock().unwrap().is_empty();
    let has_aggregates = !user_aggregates.lock().unwrap().is_empty();
    let has_collations = !user_collations.lock().unwrap().is_empty();
    let has_trace = trace_callback.lock().unwrap().is_some();
    let has_authorizer = authorizer_callback.lock().unwrap().is_some();
    let has_progress = progress_handler.lock().unwrap().is_some();

    load_ext
        || has_functions
        || has_aggregates
        || has_collations
        || has_trace
        || has_authorizer
        || has_progress
}
