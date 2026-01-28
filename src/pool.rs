//! Pool creation and connection-management helpers.

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::into_future;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::types::{ProgressHandler, UserFunctions};
use crate::OperationalError;

/// Helper to get or create pool and apply PRAGMAs.
pub(crate) async fn get_or_create_pool(
    path: &str,
    pool: &Arc<Mutex<Option<SqlitePool>>>,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<SqlitePool, PyErr> {
    let mut pool_guard = pool.lock().await;
    if pool_guard.is_none() {
        let max_conn = {
            let g = pool_size.lock().unwrap();
            (g.unwrap_or(1).max(1)) as u32
        };
        let timeout_secs = {
            let g = connection_timeout_secs.lock().unwrap();
            *g
        };
        let mut opts = SqlitePoolOptions::new().max_connections(max_conn);
        // Set default timeout of 30 seconds if not specified
        let timeout = timeout_secs.unwrap_or(30);
        opts = opts.acquire_timeout(Duration::from_secs(timeout));
        let new_pool = opts.connect(&format!("sqlite:{path}")).await.map_err(|e| {
            OperationalError::new_err(format!("Failed to connect to database at {path}: {e}"))
        })?;

        // Apply PRAGMAs
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

        *pool_guard = Some(new_pool);
    }
    Ok(pool_guard.as_ref().unwrap().clone())
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
) -> Result<(), PyErr> {
    let mut callback_guard = callback_connection.lock().await;
    if callback_guard.is_none() {
        // Get or create pool first
        let pool_clone =
            get_or_create_pool(path, pool, pragmas, pool_size, connection_timeout_secs).await?;

        // Acquire a connection from the pool
        let pool_conn = pool_clone.acquire().await.map_err(|e| {
            OperationalError::new_err(format!("Failed to acquire connection for callbacks: {e}"))
        })?;

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

/// Check if any callbacks are currently set.
pub(crate) fn has_callbacks(
    load_extension_enabled: &Arc<StdMutex<bool>>,
    user_functions: &UserFunctions,
    trace_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    authorizer_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: &ProgressHandler,
) -> bool {
    let load_ext = *load_extension_enabled.lock().unwrap();
    let has_functions = !user_functions.lock().unwrap().is_empty();
    let has_trace = trace_callback.lock().unwrap().is_some();
    let has_authorizer = authorizer_callback.lock().unwrap().is_some();
    let has_progress = progress_handler.lock().unwrap().is_some();

    load_ext || has_functions || has_trace || has_authorizer || has_progress
}
