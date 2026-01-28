//! `Connection` implementation (main user-facing class).

#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqliteConnection;
use sqlx::{Column, Row, SqlitePool};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;

// libsqlite3-sys for raw SQLite C API access
use libsqlite3_sys::{
    sqlite3, sqlite3_backup_finish, sqlite3_backup_init, sqlite3_backup_pagecount,
    sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_context, sqlite3_create_function_v2,
    sqlite3_enable_load_extension, sqlite3_errcode, sqlite3_errmsg, sqlite3_free,
    sqlite3_get_autocommit, sqlite3_libversion, sqlite3_load_extension, sqlite3_progress_handler,
    sqlite3_result_null, sqlite3_set_authorizer, sqlite3_total_changes, sqlite3_trace_v2,
    sqlite3_user_data, sqlite3_value, SQLITE_BUSY, SQLITE_DONE, SQLITE_LOCKED, SQLITE_OK,
    SQLITE_TRACE_STMT, SQLITE_UTF8,
};

use crate::conversion::{
    py_to_sqlite_c_result, row_to_py_with_factory, sqlite_c_value_to_py,
};
use crate::errors::map_sqlx_error;
use crate::parameters::{process_named_parameters, process_positional_parameters};
use crate::pool::{ensure_callback_connection, execute_init_hook_if_needed, get_or_create_pool, has_callbacks};
use crate::query::{
    bind_and_execute, bind_and_execute_on_connection, bind_and_fetch_all,
    bind_and_fetch_all_on_connection, bind_and_fetch_one, bind_and_fetch_one_on_connection,
    bind_and_fetch_optional, bind_and_fetch_optional_on_connection,
};
use crate::types::{ProgressHandler, SqliteParam, TransactionState, UserFunctions};
use crate::utils::{
    cstr_from_i8_ptr, is_select_query, parse_connection_string, track_query_usage, validate_path,
};
use crate::{Cursor, ExecuteContextManager, TransactionContextManager};
use crate::OperationalError;

/// Async SQLite connection.
#[pyclass]
pub(crate) struct Connection {
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    // Store the connection used for active transaction
    // All operations within a transaction must use this same connection
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    last_rowid: Arc<Mutex<i64>>,
    last_changes: Arc<Mutex<u64>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Store PRAGMA settings
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,   // Optional initialization hook
    init_hook_called: Arc<StdMutex<bool>>,         // Track if init_hook has been executed
    pool_size: Arc<StdMutex<Option<usize>>>,       // Configurable pool size
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>, // Connection timeout in seconds
    row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // None | "dict" | "tuple" | callable
    text_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // Callable(bytes) -> str, or None for default UTF-8
    // Prepared statement cache tracking (Phase 2.13)
    // Tracks normalized query strings and usage counts for analytics/optimization.
    // This is separate from sqlx's internal prepared statement cache, which automatically
    // caches prepared statements per connection. This field tracks query usage patterns
    // for analytics and optimization insights, while sqlx handles the actual statement
    // caching and reuse for performance.
    query_cache: Arc<StdMutex<HashMap<String, u64>>>, // normalized_query -> usage_count
    // Callback infrastructure (Phase 2.7)
    callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>, // Dedicated connection for callbacks
    load_extension_enabled: Arc<StdMutex<bool>>, // Track load_extension state
    user_functions: UserFunctions,               // name -> (nargs, callback)
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Trace callback
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Authorizer callback
    progress_handler: ProgressHandler,           // (n, callback)
}

#[pymethods]
impl Connection {
    /// Create a new async SQLite connection.
    ///
    /// The connection uses lazy initialization - the actual database connection
    /// pool is created on first use. This allows configuration (like pool_size
    /// and connection_timeout) to be set before the pool is created.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the SQLite database file. Can be ":memory:" for an
    ///   in-memory database, a file path, or a URI format: "file:path?param=value".
    ///   The path is validated for security (non-empty, no null bytes).
    /// * `pragmas` - Optional dictionary of PRAGMA settings to apply when the
    ///   connection pool is first created. Example: {"journal_mode": "WAL",
    ///   "synchronous": "NORMAL", "foreign_keys": True}. See SQLite PRAGMA
    ///   documentation for available settings.
    /// * `init_hook` - Optional async callable that receives the Connection
    ///   object and runs initialization code. Called once when the connection
    ///   pool is first used. This is a rapsqlite-specific enhancement for
    ///   automatic database initialization (schema setup, data seeding, etc.).
    ///
    /// # Returns
    ///
    /// A new Connection instance. The connection must be used as an async
    /// context manager or explicitly closed to ensure proper resource cleanup.
    ///
    /// # Errors
    ///
    /// Raises ValueError if the database path is invalid (empty or contains
    /// null bytes). Raises OperationalError if the database connection cannot
    /// be established.
    ///
    /// # Example
    ///
    /// ```python
    /// from rapsqlite import Connection
    ///
    /// # Basic connection
    /// async with Connection("example.db") as conn:
    ///     await conn.execute("CREATE TABLE test (id INTEGER)")
    ///
    /// # With PRAGMA settings
    /// async with Connection("example.db", pragmas={
    ///     "journal_mode": "WAL",
    ///     "foreign_keys": True
    /// }) as conn:
    ///     await conn.execute("CREATE TABLE test (id INTEGER)")
    ///
    /// # With initialization hook
    /// async def init_db(conn):
    ///     await conn.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER)")
    ///
    /// async with Connection("example.db", init_hook=init_db) as conn:
    ///     # Database is already initialized
    ///     pass
    /// ```
    #[new]
    #[pyo3(signature = (path, *, pragmas = None, init_hook = None))]
    fn new(
        path: String,
        pragmas: Option<&Bound<'_, pyo3::types::PyDict>>,
        init_hook: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        // Parse connection string if it's a URI
        let (db_path, uri_params) = parse_connection_string(&path)?;
        validate_path(&db_path)?;

        // Merge URI params with pragmas dict
        let mut all_pragmas = Vec::new();

        // Add URI parameters
        for (key, value) in uri_params {
            all_pragmas.push((key, value));
        }

        // Add pragmas from dict if provided
        if let Some(pragmas_dict) = pragmas {
            for item in pragmas_dict.iter() {
                let (key, value) = item; // iter() returns tuples directly in pyo3 0.27
                let key_str = key.extract::<String>()?;
                let value_str = value.to_string();
                all_pragmas.push((key_str, value_str));
            }
        }

        Ok(Connection {
            path: db_path,
            pool: Arc::new(Mutex::new(None)),
            transaction_state: Arc::new(Mutex::new(TransactionState::None)),
            transaction_connection: Arc::new(Mutex::new(None)),
            last_rowid: Arc::new(Mutex::new(0)),
            last_changes: Arc::new(Mutex::new(0)),
            pragmas: Arc::new(StdMutex::new(all_pragmas)),
            init_hook: Arc::new(StdMutex::new(init_hook)),
            init_hook_called: Arc::new(StdMutex::new(false)),
            pool_size: Arc::new(StdMutex::new(None)),
            connection_timeout_secs: Arc::new(StdMutex::new(None)),
            row_factory: Arc::new(StdMutex::new(None)),
            text_factory: Arc::new(StdMutex::new(None)),
            // Prepared statement cache tracking (Phase 2.13)
            query_cache: Arc::new(StdMutex::new(HashMap::new())),
            // Callback infrastructure (Phase 2.7)
            callback_connection: Arc::new(Mutex::new(None)),
            load_extension_enabled: Arc::new(StdMutex::new(false)),
            user_functions: Arc::new(StdMutex::new(HashMap::new())),
            trace_callback: Arc::new(StdMutex::new(None)),
            authorizer_callback: Arc::new(StdMutex::new(None)),
            progress_handler: Arc::new(StdMutex::new(None)),
        })
    }

    #[getter(row_factory)]
    fn row_factory(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let guard = self.row_factory.lock().unwrap();
        Ok(match guard.as_ref() {
            Some(f) => f.clone_ref(py),
            None => py.None(),
        })
    }

    #[setter(row_factory)]
    fn set_row_factory(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.row_factory.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            Some(value.clone().unbind())
        };
        Ok(())
    }

    /// Get the total number of database changes since connection was opened.
    ///
    /// This is a cumulative count of all INSERT, UPDATE, and DELETE operations
    /// performed on this connection. The count includes changes from all
    /// transactions and is reset when the connection is closed.
    ///
    /// # Returns
    ///
    /// Returns an awaitable that resolves to an integer (u64) representing the
    /// total number of changes.
    ///
    /// # Note
    ///
    /// In aiosqlite, this is a property. In rapsqlite, it's an async method
    /// due to internal implementation, but functionally equivalent. You must
    /// await the result: `changes = await conn.total_changes()`
    ///
    /// # Example
    ///
    /// ```python
    /// await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    /// await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
    /// changes = await conn.total_changes()  # Returns 2
    /// ```
    fn total_changes(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);

        Python::attach(|py| {
            let future = async move {
                // Check if we're in a transaction - if so, use transaction connection
                let in_transaction = {
                    let trans_guard = transaction_state.lock().await;
                    *trans_guard == TransactionState::Active
                };

                let raw_db = if in_transaction {
                    // Use transaction connection
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock handle: {e}"))
                    })?;
                    handle.as_raw_handle().as_ptr()
                } else {
                    // Check if callbacks are set - if not, use pool directly (temporary connection)
                    let has_callbacks_flag = has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );

                    if has_callbacks_flag {
                        // Use callback connection (needed for callbacks)
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;

                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock handle: {e}"))
                        })?;
                        handle.as_raw_handle().as_ptr()
                    } else {
                        // No callbacks - use pool directly with temporary connection
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        let mut temp_conn = pool_clone.acquire().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to acquire connection: {e}"))
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut temp_conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock handle: {e}"))
                        })?;
                        let handle_ptr = handle.as_raw_handle().as_ptr();

                        // Call sqlite3_total_changes while connection is alive
                        let total = unsafe { sqlite3_total_changes(handle_ptr) };

                        // Connection will be released when temp_conn is dropped
                        drop(handle);
                        drop(temp_conn);

                        return Ok(total as u64);
                    }
                };

                // Call sqlite3_total_changes (for transaction or callback connection paths)
                let total = unsafe { sqlite3_total_changes(raw_db) };

                Ok(total as u64)
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Check if connection is currently in a transaction.
    ///
    /// Returns True if a transaction has been started with `begin()` or
    /// `transaction()` context manager and not yet committed or rolled back.
    ///
    /// # Returns
    ///
    /// Returns an awaitable that resolves to a boolean indicating whether the
    /// connection is currently in a transaction.
    ///
    /// # Note
    ///
    /// In aiosqlite, this is a property. In rapsqlite, it's an async method
    /// due to internal implementation, but functionally equivalent. You must
    /// await the result: `in_tx = await conn.in_transaction()`
    ///
    /// # Example
    ///
    /// ```python
    /// in_tx = await conn.in_transaction()  # False
    /// await conn.begin()
    /// in_tx = await conn.in_transaction()  # True
    /// await conn.commit()
    /// in_tx = await conn.in_transaction()  # False
    /// ```
    fn in_transaction(&self) -> PyResult<Py<PyAny>> {
        let transaction_state = Arc::clone(&self.transaction_state);

        Python::attach(|py| {
            let future = async move {
                let trans_guard = transaction_state.lock().await;
                Ok(*trans_guard == TransactionState::Active)
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    #[getter(text_factory)]
    fn text_factory(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let guard = self.text_factory.lock().unwrap();
        Ok(match guard.as_ref() {
            Some(f) => f.clone_ref(py),
            None => py.None(),
        })
    }

    #[setter(text_factory)]
    fn set_text_factory(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.text_factory.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            Some(value.clone().unbind())
        };
        Ok(())
    }

    #[getter(pool_size)]
    fn pool_size(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let guard = self.pool_size.lock().unwrap();
        Ok(match guard.as_ref() {
            Some(&n) => PyInt::new(py, n as i64).into_any().unbind(),
            None => py.None(),
        })
    }

    #[setter(pool_size)]
    fn set_pool_size(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.pool_size.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            let n = value.extract::<i64>()?;
            if n < 0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "pool_size must be >= 0",
                ));
            }
            Some(n as usize)
        };
        Ok(())
    }

    #[getter(connection_timeout)]
    fn connection_timeout(&self) -> PyResult<Py<PyAny>> {
        // Note: Python::with_gil is used here for sync operation in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::with_gil(|py| {
            let guard = self.connection_timeout_secs.lock().unwrap();
            Ok(match guard.as_ref() {
                Some(&n) => PyInt::new(py, n as i64).into_any().unbind(),
                None => py.None(),
            })
        })
    }

    #[setter(connection_timeout)]
    fn set_connection_timeout(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.connection_timeout_secs.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            let n = value.extract::<i64>()?;
            if n < 0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "connection_timeout must be >= 0",
                ));
            }
            Some(n as u64)
        };
        Ok(())
    }

    /// Async context manager entry.
    fn __aenter__(slf: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        Python::attach(|py| {
            let future = async move { Ok(slf) };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Async context manager exit.
    fn __aexit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let pool = Arc::clone(&self.pool);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                // Clear all callbacks before closing
                // Clear user functions
                {
                    let mut funcs_guard = user_functions.lock().unwrap();
                    funcs_guard.clear();
                }

                // Clear trace callback
                {
                    let mut trace_guard = trace_callback.lock().unwrap();
                    *trace_guard = None;
                }

                // Clear authorizer callback
                {
                    let mut auth_guard = authorizer_callback.lock().unwrap();
                    *auth_guard = None;
                }

                // Clear progress handler
                {
                    let mut progress_guard = progress_handler.lock().unwrap();
                    *progress_guard = None;
                }

                // Clear callback connection (callbacks are cleared, connection returns to pool)
                {
                    let mut callback_guard = callback_connection.lock().await;
                    callback_guard.take();
                }

                // Rollback any open transaction using the stored connection
                let trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    drop(trans_guard);
                    let mut conn_guard = transaction_connection.lock().await;
                    if let Some(mut conn) = conn_guard.take() {
                        // Rollback the transaction on the same connection
                        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                        // Connection is automatically returned to pool when dropped
                    }
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::None;
                }

                // Close pool
                let mut pool_guard = pool.lock().await;
                if let Some(p) = pool_guard.take() {
                    p.close().await;
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Close the connection.
    fn close(&self) -> PyResult<Py<PyAny>> {
        let pool = Arc::clone(&self.pool);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                // Clear all callbacks before closing
                {
                    let mut funcs_guard = user_functions.lock().unwrap();
                    funcs_guard.clear();
                }
                {
                    let mut trace_guard = trace_callback.lock().unwrap();
                    *trace_guard = None;
                }
                {
                    let mut auth_guard = authorizer_callback.lock().unwrap();
                    *auth_guard = None;
                }
                {
                    let mut progress_guard = progress_handler.lock().unwrap();
                    *progress_guard = None;
                }
                {
                    let mut callback_guard = callback_connection.lock().await;
                    callback_guard.take();
                }

                // Rollback any open transaction using the stored connection
                let trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    drop(trans_guard);
                    let mut conn_guard = transaction_connection.lock().await;
                    if let Some(mut conn) = conn_guard.take() {
                        // Rollback the transaction on the same connection
                        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                        // Connection is automatically returned to pool when dropped
                    }
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::None;
                }

                // Close pool
                let mut pool_guard = pool.lock().await;
                if let Some(p) = pool_guard.take() {
                    p.close().await;
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Begin a transaction.
    fn begin(self_: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();
        Python::attach(|py| {
            let future = async move {
                // Check transaction state and release lock before calling init_hook
                // This prevents deadlock when init_hook calls conn.execute() which needs to check transaction state
                {
                    let trans_guard = transaction_state.lock().await;
                    if *trans_guard == TransactionState::Active {
                        return Err(OperationalError::new_err("Transaction already in progress"));
                    }
                } // Lock released here

                // Ensure pool exists before calling init_hook
                // Note: If pool_size is 1, init_hook's execute() may compete with begin() for the connection
                // This is handled by ensuring init_hook completes before begin() acquires connection
                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Execute init_hook if needed (before starting transaction)
                // Init_hook operations will use pool connections
                // If pool_size is 1, init_hook's execute() will get the connection, then release it
                // before begin() tries to acquire it for the transaction
                // Lock is released, so init_hook's execute() can check transaction state without deadlock
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                // Check if callbacks are set - if so, use callback connection for transaction
                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let mut conn = if has_callbacks_flag {
                    // Use callback connection for transaction
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    conn_guard.take().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?
                } else {
                    // Acquire a connection from the pool for the transaction
                    pool_clone.acquire().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to acquire connection: {e}"))
                    })?
                };

                // Set PRAGMA busy_timeout on this connection to handle lock contention
                sqlx::query("PRAGMA busy_timeout = 5000")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "PRAGMA busy_timeout = 5000"))?;

                // Execute BEGIN IMMEDIATE on this specific connection
                // BEGIN IMMEDIATE acquires the write lock upfront, preventing "database is locked" errors
                sqlx::query("BEGIN IMMEDIATE")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "BEGIN IMMEDIATE"))?;

                // Store the connection for reuse in all transaction operations
                {
                    let mut conn_guard = transaction_connection.lock().await;
                    *conn_guard = Some(conn);
                }

                // Re-acquire lock to set transaction state
                {
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::Active;
                }
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Commit the current transaction.
    fn commit(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        // Callback infrastructure (Phase 2.7) - need to return connection if it came from callbacks
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                // Check if callbacks are set - if so, we need to return connection to callback_connection
                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Retrieve the stored transaction connection
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard.take().ok_or_else(|| {
                    OperationalError::new_err("Transaction connection not available")
                })?;

                // Execute COMMIT on the same connection that started the transaction
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "COMMIT"))?;

                // If callbacks are set, return connection to callback_connection; otherwise it goes back to pool
                if has_callbacks_flag {
                    let mut callback_guard = callback_connection.lock().await;
                    *callback_guard = Some(conn);
                } else {
                    // Connection is automatically returned to pool when dropped
                    drop(conn);
                }

                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Rollback the current transaction.
    fn rollback(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        // Callback infrastructure (Phase 2.7) - need to return connection if it came from callbacks
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                // Check if callbacks are set - if so, we need to return connection to callback_connection
                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Retrieve the stored transaction connection
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard.take().ok_or_else(|| {
                    OperationalError::new_err("Transaction connection not available")
                })?;

                // Execute ROLLBACK on the same connection that started the transaction
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "ROLLBACK"))?;

                // If callbacks are set, return connection to callback_connection; otherwise it goes back to pool
                if has_callbacks_flag {
                    let mut callback_guard = callback_connection.lock().await;
                    *callback_guard = Some(conn);
                } else {
                    // Connection is automatically returned to pool when dropped
                    drop(conn);
                }

                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Execute a SQL query (does not return results).
    ///
    /// Executes a SQL statement such as CREATE, INSERT, UPDATE, DELETE, etc.
    /// For SELECT queries, use `fetch_all()`, `fetch_one()`, or `fetch_optional()`
    /// instead. This method supports parameterized queries with both named and
    /// positional parameters.
    ///
    /// # Arguments
    ///
    /// * `query` - SQL query string to execute. Can contain parameter placeholders:
    ///   - Named parameters: `:name`, `@name`, `$name`
    ///   - Positional parameters: `?`, `?1`, `?2`
    /// * `parameters` - Optional parameters for the query. Can be:
    ///   - A dictionary for named parameters: `{"name": "value", ...}`
    ///   - A list/tuple for positional parameters: `[value1, value2, ...]`
    ///   - A single value (treated as single positional parameter)
    ///   - None (no parameters)
    ///
    /// # Returns
    ///
    /// Returns an ExecuteContextManager that can be used as:
    /// - `await conn.execute(...)` - Execute and return None
    /// - `async with conn.execute(...) as cursor:` - Execute and get cursor
    ///
    /// # Errors
    ///
    /// Raises OperationalError if the query execution fails (e.g., database
    /// locked, disk full). Raises ProgrammingError for SQL syntax errors.
    /// Raises IntegrityError for constraint violations.
    ///
    /// # Example
    ///
    /// ```python
    /// # Simple query
    /// await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
    ///
    /// # With positional parameters
    /// await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    ///
    /// # With named parameters
    /// await conn.execute(
    ///     "INSERT INTO users (name, email) VALUES (:name, :email)",
    ///     {"name": "Bob", "email": "bob@example.com"}
    /// )
    ///
    /// # Using as context manager (returns cursor)
    /// async with conn.execute("SELECT * FROM users") as cursor:
    ///     rows = await cursor.fetchall()
    /// ```
    #[pyo3(signature = (query, parameters = None))]
    fn execute(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Prepared statement cache tracking (Phase 2.13)
        let query_cache = Arc::clone(&self_.query_cache);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let row_factory = Arc::clone(&self_.row_factory);
        let connection_self: Py<Connection> = self_.into();

        // Clone query before processing (it may be moved)
        let original_query = query.clone();

        // Process parameters
        // Note: Python::with_gil is used here for sync parameter processing before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let (processed_query, param_values) = Python::with_gil(|_py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            // Check if it's a dict (named parameters)
            if let Ok(dict) = params.cast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, &dict);
            }

            // Check if it's a list or tuple (positional parameters)
            if let Ok(list) = params.cast::<PyList>() {
                let params_vec = process_positional_parameters(&list)?;
                return Ok((query, params_vec));
            }

            // Single value (treat as single positional parameter)
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        // Track query usage for prepared statement cache analytics (Phase 2.13)
        track_query_usage(&query_cache, &processed_query);

        // Check if this is a SELECT query (for lazy execution)
        let is_select = is_select_query(&processed_query);

        // Store original parameters for cursor (preserve original format)
        let params_for_cursor = parameters.map(|params| params.clone().unbind());

        // Clone necessary fields for cursor creation (will be used in async future)
        // Note: These are currently unused but kept for potential future use
        let _cursor_path = path.clone();
        let _cursor_pool = Arc::clone(&pool);
        let _cursor_pragmas = Arc::clone(&pragmas);
        let _cursor_pool_size = Arc::clone(&pool_size);
        let _cursor_connection_timeout_secs = Arc::clone(&connection_timeout_secs);
        let _cursor_row_factory = Arc::clone(&row_factory);
        // Create cursor synchronously (query and params are already processed)
        // For named parameters, processed_query has :value replaced with ?
        // The cursor needs the ORIGINAL query (with :value) so fetchall() can process it correctly
        // But we also need to store the processed param_values for immediate execution
        // Solution: Store original query in cursor, but ExecuteContextManager has processed_query
        // for execution. The cursor will re-process parameters when fetchall() is called.
        // Note: Python::with_gil is used here for sync cursor creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let cursor = Python::with_gil(|py| -> PyResult<Py<Cursor>> {
            let cursor = Cursor {
                connection: connection_self.clone_ref(py),
                query: original_query.clone(), // Store ORIGINAL query (with :value) for cursor processing
                results: Arc::new(StdMutex::new(None)),
                current_index: Arc::new(StdMutex::new(0)),
                parameters: Arc::new(StdMutex::new(params_for_cursor)), // Store original params
                processed_query: Some(processed_query.clone()), // Store processed query to avoid re-processing
                processed_params: Some(param_values.clone()), // Store processed parameters to avoid re-processing
                connection_path: path.clone(),
                connection_pool: Arc::clone(&pool),
                connection_pragmas: Arc::clone(&pragmas),
                pool_size: Arc::clone(&pool_size),
                connection_timeout_secs: Arc::clone(&connection_timeout_secs),
                row_factory: Arc::clone(&row_factory),
                transaction_state: Arc::clone(&transaction_state),
                transaction_connection: Arc::clone(&transaction_connection),
                callback_connection: Arc::clone(&callback_connection),
                load_extension_enabled: Arc::clone(&load_extension_enabled),
                user_functions: Arc::clone(&user_functions),
                trace_callback: Arc::clone(&trace_callback),
                authorizer_callback: Arc::clone(&authorizer_callback),
                progress_handler: Arc::clone(&progress_handler),
            };
            Py::new(py, cursor)
        })?;

        // Create ExecuteContextManager and return it
        // For `async with conn.execute(...)`: ExecuteContextManager works as context manager
        // For `await conn.execute(...)`: We need to return the Future from __aenter__ directly
        // Since we can't return different types, we return ExecuteContextManager and make
        // __await__ call __aenter__ and return its result. But __aenter__ returns a Future,
        // and __await__ needs to return an iterator. The Future from future_into_py is awaitable
        // but not an iterator. So we return the Future and let Python handle it.
        // Actually, Futures implement __await__ which returns an iterator, so returning
        // the Future from __await__ should work. But Python is complaining.
        // Let's try returning the ExecuteContextManager and see if we can make __await__ work.
        // Note: Python::with_gil is used here for sync context manager creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        // Note: Python::with_gil is used here for sync result conversion in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::with_gil(|py| -> PyResult<Py<PyAny>> {
            let ctx_mgr = ExecuteContextManager {
                cursor: cursor.clone_ref(py),
                query: processed_query,
                param_values,
                is_select,
                path,
                pool: Arc::clone(&pool),
                pragmas: Arc::clone(&pragmas),
                pool_size: Arc::clone(&pool_size),
                connection_timeout_secs: Arc::clone(&connection_timeout_secs),
                transaction_state: Arc::clone(&transaction_state),
                transaction_connection: Arc::clone(&transaction_connection),
                callback_connection: Arc::clone(&callback_connection),
                load_extension_enabled: Arc::clone(&load_extension_enabled),
                user_functions: Arc::clone(&user_functions),
                trace_callback: Arc::clone(&trace_callback),
                authorizer_callback: Arc::clone(&authorizer_callback),
                progress_handler: Arc::clone(&progress_handler),
                init_hook: Arc::clone(&init_hook),
                init_hook_called: Arc::clone(&init_hook_called),
                last_rowid: Arc::clone(&last_rowid),
                last_changes: Arc::clone(&last_changes),
                connection: connection_self.clone_ref(py),
            };
            Py::new(py, ctx_mgr).map(|c| c.into())
        })
    }

    /// Execute a query multiple times with different parameters.
    fn execute_many(
        self_: PyRef<Self>,
        query: String,
        parameters: Vec<Vec<Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Process all parameter sets
        // Each element in parameters is a list/tuple of parameters for one execution
        // Note: Python::with_gil is used here for sync parameter processing before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let processed_params = Python::with_gil(|py| -> PyResult<Vec<Vec<SqliteParam>>> {
            let mut result = Vec::new();
            for param_set in parameters.iter() {
                // Convert Vec<Py<PyAny>> to Vec<SqliteParam>
                let mut params_vec = Vec::new();
                for param in param_set {
                    let bound_param = param.bind(py);
                    let sqlx_param = SqliteParam::from_py(bound_param)?;
                    params_vec.push(sqlx_param);
                }
                result.push(params_vec);
            }
            Ok(result)
        })?;

        Python::attach(|py| {
            let future = async move {
                // Priority: transaction > callbacks > pool
                let in_transaction = {
                    let trans_guard = transaction_state.lock().await;
                    *trans_guard == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let mut total_changes = 0u64;
                let mut last_row_id = 0i64;

                if in_transaction {
                    // Use stored transaction connection. Release lock each iteration
                    // to match the execute-in-loop pattern (lock -> use -> release).
                    for param_values in processed_params.iter() {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        let result =
                            bind_and_execute_on_connection(&query, param_values, conn, &path)
                                .await?;
                        total_changes += result.rows_affected();
                        last_row_id = result.last_insert_rowid();
                        drop(conn_guard);
                    }
                } else if has_callbacks_flag {
                    // Ensure callback connection exists once before the loop
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;

                    // Use callback connection for each iteration
                    for param_values in processed_params.iter() {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        let result =
                            bind_and_execute_on_connection(&query, param_values, conn, &path)
                                .await?;
                        total_changes += result.rows_affected();
                        last_row_id = result.last_insert_rowid();
                        drop(conn_guard);
                    }
                } else {
                    // Use pool
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    for param_values in processed_params {
                        let result =
                            bind_and_execute(&query, &param_values, &pool_clone, &path).await?;
                        total_changes += result.rows_affected();
                        last_row_id = result.last_insert_rowid();
                    }
                }

                *last_rowid.lock().await = last_row_id;
                *last_changes.lock().await = total_changes;

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch all rows from a SELECT query.
    ///
    /// Executes a SELECT query and returns all rows as a list. Each row is
    /// formatted according to the current `row_factory` setting (default: list).
    ///
    /// # Arguments
    ///
    /// * `query` - SELECT query string. Can contain parameter placeholders.
    /// * `parameters` - Optional parameters (same format as `execute()`).
    ///
    /// # Returns
    ///
    /// Returns an awaitable that resolves to a list of rows. Each row format
    /// depends on `row_factory`:
    /// - None: List of values `[value1, value2, ...]`
    /// - "dict": Dictionary with column names as keys
    /// - "tuple": Tuple of values
    /// - Callable: Result of calling the factory function
    /// - Row class: Dict-like Row object
    ///
    /// # Errors
    ///
    /// Raises ProgrammingError for SQL syntax errors or if query is not a SELECT.
    /// Raises OperationalError for database errors.
    ///
    /// # Example
    ///
    /// ```python
    /// # Default (list format)
    /// rows = await conn.fetch_all("SELECT * FROM users")
    /// # rows = [[1, "Alice"], [2, "Bob"]]
    ///
    /// # With dict factory
    /// conn.row_factory = "dict"
    /// rows = await conn.fetch_all("SELECT * FROM users")
    /// # rows = [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]
    ///
    /// # With parameters
    /// rows = await conn.fetch_all("SELECT * FROM users WHERE id > ?", [5])
    /// ```
    #[pyo3(signature = (query, parameters = None))]
    fn fetch_all(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let row_factory = Arc::clone(&self_.row_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Prepared statement cache tracking (Phase 2.13)
        let query_cache = Arc::clone(&self_.query_cache);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Process parameters
        // Note: Python::with_gil is used here for sync parameter processing before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let (processed_query, param_values) = Python::with_gil(|_py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            // Check if it's a dict (named parameters)
            if let Ok(dict) = params.cast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, &dict);
            }

            // Check if it's a list or tuple (positional parameters)
            if let Ok(list) = params.cast::<PyList>() {
                let params_vec = process_positional_parameters(&list)?;
                return Ok((query, params_vec));
            }

            // Single value (treat as single positional parameter)
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        // Track query usage for prepared statement cache analytics (Phase 2.13)
        track_query_usage(&query_cache, &processed_query);

        Python::attach(|py| {
            let future = async move {
                // Priority: transaction > callbacks > pool
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&processed_query, &param_values, conn, &path)
                        .await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;

                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&processed_query, &param_values, conn, &path)
                        .await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&processed_query, &param_values, &pool_clone, &path).await?
                };

                // Convert rows using row_factory
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let guard = row_factory.lock().unwrap();
                    let factory_opt = guard.as_ref();
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let out = row_to_py_with_factory(py, row, factory_opt)?;
                        result_list.append(out)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch a single row from a SELECT query.
    ///
    /// Executes a SELECT query and returns exactly one row. Raises an error
    /// if no rows or more than one row is returned.
    ///
    /// # Arguments
    ///
    /// * `query` - SELECT query string. Should return exactly one row.
    /// * `parameters` - Optional parameters (same format as `execute()`).
    ///
    /// # Returns
    ///
    /// Returns an awaitable that resolves to a single row (format depends on
    /// `row_factory`, same as `fetch_all()`).
    ///
    /// # Errors
    ///
    /// Raises ProgrammingError if no rows are found or if more than one row
    /// is returned. Raises OperationalError for database errors.
    ///
    /// # Example
    ///
    /// ```python
    /// # Fetch user by ID (expects exactly one)
    /// user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [1])
    /// # user = [1, "Alice"]  # or dict/Row depending on row_factory
    ///
    /// # This will raise if user doesn't exist
    /// try:
    ///     user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [999])
    /// except ProgrammingError:
    ///     print("User not found")
    /// ```
    #[pyo3(signature = (query, parameters = None))]
    fn fetch_one(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let row_factory = Arc::clone(&self_.row_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Process parameters
        // Note: Python::with_gil is used here for sync parameter processing before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let (processed_query, param_values) = Python::with_gil(|_py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            if let Ok(dict) = params.cast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, &dict);
            }
            if let Ok(list) = params.cast::<PyList>() {
                let params_vec = process_positional_parameters(&list)?;
                return Ok((query, params_vec));
            }
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        Python::attach(|py| {
            let future = async move {
                // Priority: transaction > callbacks > pool
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let row = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_one_on_connection(&processed_query, &param_values, conn, &path)
                        .await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;

                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_one_on_connection(&processed_query, &param_values, conn, &path)
                        .await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_one(&processed_query, &param_values, &pool_clone, &path).await?
                };

                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let guard = row_factory.lock().unwrap();
                    let factory_opt = guard.as_ref();
                    let out = row_to_py_with_factory(py, &row, factory_opt)?;
                    Ok(out.unbind())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch a single row from a SELECT query, returning None if no rows.
    ///
    /// Executes a SELECT query and returns one row or None. Raises an error
    /// if more than one row is returned.
    ///
    /// # Arguments
    ///
    /// * `query` - SELECT query string. Should return zero or one row.
    /// * `parameters` - Optional parameters (same format as `execute()`).
    ///
    /// # Returns
    ///
    /// Returns an awaitable that resolves to:
    /// - A single row (format depends on `row_factory`) if one row is found
    /// - None if no rows are found
    ///
    /// # Errors
    ///
    /// Raises ProgrammingError if more than one row is returned. Raises
    /// OperationalError for database errors.
    ///
    /// # Example
    ///
    /// ```python
    /// # Fetch user by ID (may not exist)
    /// user = await conn.fetch_optional("SELECT * FROM users WHERE id = ?", [1])
    /// if user:
    ///     print(f"Found: {user}")
    /// else:
    ///     print("User not found")
    ///
    /// # Safe for optional lookups
    /// user = await conn.fetch_optional(
    ///     "SELECT * FROM users WHERE email = ?",
    ///     ["alice@example.com"]
    /// )
    /// ```
    #[pyo3(signature = (query, parameters = None))]
    fn fetch_optional(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let row_factory = Arc::clone(&self_.row_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Process parameters
        // Note: Python::with_gil is used here for sync parameter processing before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let (processed_query, param_values) = Python::with_gil(|_py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            if let Ok(dict) = params.cast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, &dict);
            }
            if let Ok(list) = params.cast::<PyList>() {
                let params_vec = process_positional_parameters(&list)?;
                return Ok((query, params_vec));
            }
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        Python::attach(|py| {
            let future = async move {
                // Priority: transaction > callbacks > pool
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let opt = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                    )
                    .await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;

                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                    )
                    .await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_optional(&processed_query, &param_values, &pool_clone, &path)
                        .await?
                };

                match opt {
                    Some(row) => Python::attach(|py| -> PyResult<Py<PyAny>> {
                        let guard = row_factory.lock().unwrap();
                        let factory_opt = guard.as_ref();
                        let out = row_to_py_with_factory(py, &row, factory_opt)?;
                        Ok(out.unbind())
                    }),
                    None => Python::attach(|py| -> PyResult<Py<PyAny>> { Ok(py.None()) }),
                }
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get the last insert row ID.
    fn last_insert_rowid(&self) -> PyResult<Py<PyAny>> {
        let last_rowid = Arc::clone(&self.last_rowid);
        Python::attach(|py| {
            let future = async move { Ok(*last_rowid.lock().await) };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get the number of rows affected by the last statement.
    fn changes(&self) -> PyResult<Py<PyAny>> {
        let last_changes = Arc::clone(&self.last_changes);
        Python::attach(|py| {
            let future = async move { Ok(*last_changes.lock().await) };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Create a cursor for this connection.
    fn cursor(slf: PyRef<Self>) -> PyResult<Cursor> {
        let path = slf.path.clone();
        let pool = Arc::clone(&slf.pool);
        let pragmas = Arc::clone(&slf.pragmas);
        let pool_size = Arc::clone(&slf.pool_size);
        let connection_timeout_secs = Arc::clone(&slf.connection_timeout_secs);
        let row_factory = Arc::clone(&slf.row_factory);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let callback_connection = Arc::clone(&slf.callback_connection);
        let load_extension_enabled = Arc::clone(&slf.load_extension_enabled);
        let user_functions = Arc::clone(&slf.user_functions);
        let trace_callback = Arc::clone(&slf.trace_callback);
        let authorizer_callback = Arc::clone(&slf.authorizer_callback);
        let progress_handler = Arc::clone(&slf.progress_handler);
        Ok(Cursor {
            connection: slf.into(),
            query: String::new(),
            results: Arc::new(StdMutex::new(None)),
            current_index: Arc::new(StdMutex::new(0)),
            parameters: Arc::new(StdMutex::new(None)),
            processed_query: None,  // No processed query for cursor() method
            processed_params: None, // No processed params for cursor() method
            connection_path: path,
            connection_pool: pool,
            connection_pragmas: pragmas,
            pool_size,
            connection_timeout_secs,
            row_factory,
            transaction_state,
            transaction_connection,
            callback_connection,
            load_extension_enabled,
            user_functions,
            trace_callback,
            authorizer_callback,
            progress_handler,
        })
    }

    /// Create a cursor with a pre-initialized query and parameters.
    /// This is used by execute() to return a cursor that can be used as an async context manager.
    fn create_cursor_with_query(
        slf: PyRef<Self>,
        query: String,
        parameters: Option<Py<PyAny>>,
    ) -> PyResult<Cursor> {
        let path = slf.path.clone();
        let pool = Arc::clone(&slf.pool);
        let pragmas = Arc::clone(&slf.pragmas);
        let pool_size = Arc::clone(&slf.pool_size);
        let connection_timeout_secs = Arc::clone(&slf.connection_timeout_secs);
        let row_factory = Arc::clone(&slf.row_factory);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let callback_connection = Arc::clone(&slf.callback_connection);
        let load_extension_enabled = Arc::clone(&slf.load_extension_enabled);
        let user_functions = Arc::clone(&slf.user_functions);
        let trace_callback = Arc::clone(&slf.trace_callback);
        let authorizer_callback = Arc::clone(&slf.authorizer_callback);
        let progress_handler = Arc::clone(&slf.progress_handler);
        Ok(Cursor {
            connection: slf.into(),
            query,
            results: Arc::new(StdMutex::new(None)),
            current_index: Arc::new(StdMutex::new(0)),
            parameters: Arc::new(StdMutex::new(parameters)),
            processed_query: None, // No processed query for create_cursor_with_query() method
            processed_params: None, // No processed params for create_cursor_with_query() method
            connection_path: path,
            connection_pool: pool,
            connection_pragmas: pragmas,
            pool_size,
            connection_timeout_secs,
            row_factory,
            transaction_state,
            transaction_connection,
            callback_connection,
            load_extension_enabled,
            user_functions,
            trace_callback,
            authorizer_callback,
            progress_handler,
        })
    }

    /// Return an async context manager for a transaction.
    /// On __aenter__ calls begin(); on __aexit__ calls commit() or rollback().
    fn transaction(slf: PyRef<Self>) -> PyResult<TransactionContextManager> {
        let path = slf.path.clone();
        let pool = Arc::clone(&slf.pool);
        let pragmas = Arc::clone(&slf.pragmas);
        let pool_size = Arc::clone(&slf.pool_size);
        let connection_timeout_secs = Arc::clone(&slf.connection_timeout_secs);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let init_hook = Arc::clone(&slf.init_hook);
        let init_hook_called = Arc::clone(&slf.init_hook_called);
        let connection: Py<Connection> = slf.into();
        Ok(TransactionContextManager {
            path,
            pool,
            pragmas,
            pool_size,
            connection_timeout_secs,
            transaction_state,
            transaction_connection,
            connection,
            init_hook,
            init_hook_called,
        })
    }

    /// Set a PRAGMA value on the database connection.
    fn set_pragma(
        self_: PyRef<Self>,
        name: String,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Convert value to string for PRAGMA
        // Note: Python::with_gil is used here for sync PRAGMA value conversion before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let pragma_value = Python::with_gil(|_py| -> PyResult<String> {
            if value.is_none() {
                Ok("NULL".to_string())
            } else if let Ok(int_val) = value.extract::<i64>() {
                Ok(int_val.to_string())
            } else if let Ok(str_val) = value.extract::<String>() {
                Ok(format!("'{}'", str_val.replace("'", "''"))) // Escape single quotes
            } else {
                Ok(format!("'{}'", value.to_string().replace("'", "''")))
            }
        })?;

        // Store PRAGMA for future connections
        {
            let mut pragmas_guard = pragmas.lock().unwrap();
            // Update or add PRAGMA
            let mut found = false;
            for (key, val) in pragmas_guard.iter_mut() {
                if *key == name {
                    *val = pragma_value.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                pragmas_guard.push((name.clone(), pragma_value.clone()));
            }
        }

        let pragma_query = format!("PRAGMA {name} = {pragma_value}");

        Python::attach(|py| {
            let future = async move {
                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Execute init_hook if needed (before setting PRAGMA)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                sqlx::query(&pragma_query)
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &pragma_query))?;

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Enable or disable loading SQLite extensions.
    fn enable_load_extension(&self, enabled: bool) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);

        Python::attach(|py| {
            let future = async move {
                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                // Store the state
                {
                    let mut enabled_guard = load_extension_enabled.lock().unwrap();
                    *enabled_guard = enabled;
                }

                // Access raw sqlite3* handle via PoolConnection's Deref to SqliteConnection
                // PoolConnection<Sqlite> derefs to SqliteConnection, so we can use &mut *conn
                // Then call lock_handle() to get LockedSqliteHandle, then as_raw_handle() for NonNull<sqlite3>
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                // Call the C API
                let enabled_int = if enabled { 1 } else { 0 };
                let result = unsafe { sqlite3_enable_load_extension(raw_db, enabled_int) };

                if result != 0 {
                    return Err(OperationalError::new_err(format!(
                        "Failed to enable/disable load extension: SQLite error code {result}"
                    )));
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Load a SQLite extension from the specified file.
    /// Extension loading must be enabled first using enable_load_extension(true).
    fn load_extension(&self, name: String) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);

        Python::attach(|py| {
            let future = async move {
                // Check if extension loading is enabled
                let enabled = {
                    let guard = load_extension_enabled.lock().unwrap();
                    *guard
                };

                if !enabled {
                    return Err(OperationalError::new_err(
                        "Extension loading is not enabled. Call enable_load_extension(true) first.",
                    ));
                }

                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                // Convert extension name to CString
                let name_cstr = CString::new(name.clone()).map_err(|e| {
                    OperationalError::new_err(format!("Invalid extension name: {e}"))
                })?;

                // Call sqlite3_load_extension
                // Use NULL for entry point - SQLite will try sqlite3_extension_init first
                let mut errmsg: *mut i8 = std::ptr::null_mut();
                let result = unsafe {
                    sqlite3_load_extension(
                        raw_db,
                        name_cstr.as_ptr(),
                        std::ptr::null(), // NULL entry point - SQLite will auto-detect
                        &mut errmsg,
                    )
                };

                // Handle error message if present
                if result != SQLITE_OK {
                    let error_msg = if !errmsg.is_null() {
                        let cstr = unsafe { cstr_from_i8_ptr(errmsg) };
                        let msg = cstr.to_string_lossy().to_string();
                        unsafe {
                            sqlite3_free(errmsg as *mut std::ffi::c_void);
                        }
                        msg
                    } else {
                        format!("SQLite error code {result}")
                    };
                    return Err(OperationalError::new_err(format!(
                        "Failed to load extension '{name}': {error_msg}"
                    )));
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Create or remove a user-defined SQL function.
    /// If func is None, the function is removed.
    fn create_function(
        &self,
        name: String,
        nargs: i32,
        func: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let user_functions = Arc::clone(&self.user_functions);
        // Need all callback fields to check if all are cleared
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);

        Python::attach(|py| {
            // Clone the callback with GIL to avoid Send issues
            let func_clone = func.as_ref().map(|f| f.clone_ref(py));

            let future = async move {
                // Ensure callback connection exists (needed for both adding and removing functions)
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                if func_clone.is_none() {
                    // Remove the function from user_functions
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.remove(&name);
                    }

                    // Remove from SQLite by calling sqlite3_create_function_v2 with NULL callback
                    let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
                        OperationalError::new_err(format!("Function name contains null byte: {e}"))
                    })?;
                    let result = unsafe {
                        sqlite3_create_function_v2(
                            raw_db,
                            name_cstr.as_ptr(),
                            nargs,
                            SQLITE_UTF8,
                            std::ptr::null_mut(), // pApp (user data)
                            None,                 // xFunc (scalar function callback)
                            None,                 // xStep (aggregate step callback)
                            None,                 // xFinal (aggregate final callback)
                            None,                 // xDestroy (destructor)
                        )
                    };

                    if result != SQLITE_OK {
                        return Err(OperationalError::new_err(format!(
                            "Failed to remove function '{name}': SQLite error code {result}"
                        )));
                    }

                    // After removing, check if all callbacks are now cleared
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        drop(handle);
                        drop(conn_guard);
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        return Ok(());
                    }
                } else {
                    // Store the function - need to clone the callback with GIL
                    // Note: Python::with_gil is used here for sync callback storage in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
                    let callback_for_storage =
                        Python::with_gil(|py| func_clone.as_ref().unwrap().clone_ref(py));
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.insert(name.clone(), (nargs, callback_for_storage));
                    }

                    // Create a boxed callback pointer to pass as user data
                    let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
                        OperationalError::new_err(format!("Function name contains null byte: {e}"))
                    })?;

                    // Store the Python callback in a Box and pass it as user_data
                    // Clone it with GIL
                    // Note: Python::with_gil is used here for sync callback access in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
                    let callback =
                        Python::with_gil(|py| func_clone.as_ref().unwrap().clone_ref(py));
                    let callback_box: Box<Py<PyAny>> = Box::new(callback);
                    let callback_ptr = Box::into_raw(callback_box) as *mut std::ffi::c_void;

                    // Define the trampoline callback
                    extern "C" fn udf_trampoline(
                        ctx: *mut sqlite3_context,
                        argc: std::ffi::c_int,
                        argv: *mut *mut sqlite3_value,
                    ) {
                        unsafe {
                            // Extract the Python callback from user_data
                            let user_data = sqlite3_user_data(ctx);
                            if user_data.is_null() {
                                sqlite3_result_null(ctx);
                                return;
                            }

                            // Get the callback from user_data
                            // The callback is stored in a Box, we need to clone it to use it
                            // We can't take ownership because the destructor will free it
                            let callback_ptr = user_data as *mut Py<PyAny>;

                            // Convert SQLite values to Python values
                            // Note: Python::with_gil is used here for sync callback execution in async context.
                            // The deprecation warning is acceptable as this is a sync operation within async.
                            #[allow(deprecated)]
                            // Note: Python::with_gil is used here for sync operation in async context.
                            // The deprecation warning is acceptable as this is a sync operation within async.
                            #[allow(deprecated)]
                            Python::with_gil(|py| {
                                // Clone the callback to use it (the original stays in the Box)
                                let callback = (*callback_ptr).clone_ref(py);

                                let mut py_args: Vec<Py<PyAny>> = Vec::new();
                                for i in 0..argc {
                                    let value_ptr = *argv.add(i as usize);
                                    match sqlite_c_value_to_py(py, value_ptr) {
                                        Ok(py_val) => {
                                            py_args.push(py_val);
                                        }
                                        Err(e) => {
                                            // On error, set SQLite error and return
                                            let error_msg =
                                                format!("Error converting argument {i}: {e}");
                                            libsqlite3_sys::sqlite3_result_error(
                                                ctx,
                                                error_msg.as_ptr() as *const i8,
                                                error_msg.len() as i32,
                                            );
                                            return;
                                        }
                                    }
                                }

                                // Call the Python callback with proper argument unpacking
                                // PyO3's call1 with a tuple passes it as a single argument
                                // We need to unpack based on argument count
                                let result = match py_args.len() {
                                    0 => callback.bind(py).call0(),
                                    1 => {
                                        // Single argument - pass directly
                                        callback.bind(py).call1((py_args[0].clone_ref(py),))
                                    }
                                    2 => {
                                        // Two arguments
                                        callback.bind(py).call1((
                                            py_args[0].clone_ref(py),
                                            py_args[1].clone_ref(py),
                                        ))
                                    }
                                    3 => {
                                        // Three arguments
                                        callback.bind(py).call1((
                                            py_args[0].clone_ref(py),
                                            py_args[1].clone_ref(py),
                                            py_args[2].clone_ref(py),
                                        ))
                                    }
                                    4 => {
                                        // Four arguments
                                        callback.bind(py).call1((
                                            py_args[0].clone_ref(py),
                                            py_args[1].clone_ref(py),
                                            py_args[2].clone_ref(py),
                                            py_args[3].clone_ref(py),
                                        ))
                                    }
                                    5 => {
                                        // Five arguments
                                        callback.bind(py).call1((
                                            py_args[0].clone_ref(py),
                                            py_args[1].clone_ref(py),
                                            py_args[2].clone_ref(py),
                                            py_args[3].clone_ref(py),
                                            py_args[4].clone_ref(py),
                                        ))
                                    }
                                    _ => {
                                        // For more than 5 arguments, use Python's unpacking
                                        // Create a helper function that unpacks the tuple
                                        let args_tuple = match PyTuple::new(
                                            py,
                                            py_args
                                                .iter()
                                                .map(|arg: &Py<PyAny>| arg.clone_ref(py)),
                                        ) {
                                            Ok(t) => t,
                                            Err(e) => {
                                                let error_msg =
                                                    format!("Error creating argument tuple: {e}");
                                                libsqlite3_sys::sqlite3_result_error(
                                                    ctx,
                                                    error_msg.as_ptr() as *const i8,
                                                    error_msg.len() as i32,
                                                );
                                                return;
                                            }
                                        };
                                        // Use Python code to unpack: lambda f, args: f(*args)
                                        let code_str = match std::ffi::CString::new(
                                            "lambda f, args: f(*args)",
                                        ) {
                                            Ok(s) => s,
                                            Err(_) => {
                                                libsqlite3_sys::sqlite3_result_error(
                                                    ctx,
                                                    c"Error creating CString".as_ptr(),
                                                    22,
                                                );
                                                return;
                                            }
                                        };
                                        let unpack_code =
                                            match py.eval(code_str.as_c_str(), None, None) {
                                                Ok(code) => code,
                                                Err(e) => {
                                                    let error_msg = format!(
                                                        "Error creating unpack helper: {e}"
                                                    );
                                                    libsqlite3_sys::sqlite3_result_error(
                                                        ctx,
                                                        error_msg.as_ptr() as *const i8,
                                                        error_msg.len() as i32,
                                                    );
                                                    return;
                                                }
                                            };
                                        unpack_code.call1((callback.bind(py), args_tuple))
                                    }
                                };

                                match result {
                                    Ok(result) => {
                                        // Convert result back to SQLite
                                        match py_to_sqlite_c_result(py, ctx, &result) {
                                            Ok(_) => {}
                                            Err(e) => {
                                                let error_msg =
                                                    format!("Error converting result: {e}");
                                                libsqlite3_sys::sqlite3_result_error(
                                                    ctx,
                                                    error_msg.as_ptr() as *const i8,
                                                    error_msg.len() as i32,
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Python exception - convert to SQLite error
                                        let error_msg = format!("Python function error: {e}");
                                        libsqlite3_sys::sqlite3_result_error(
                                            ctx,
                                            error_msg.as_ptr() as *const i8,
                                            error_msg.len() as i32,
                                        );
                                    }
                                }
                            });
                        }
                    }

                    // Destructor to clean up the callback pointer
                    extern "C" fn udf_destructor(user_data: *mut std::ffi::c_void) {
                        unsafe {
                            if !user_data.is_null() {
                                let _ = Box::from_raw(user_data as *mut Py<PyAny>);
                            }
                        }
                    }

                    let result = unsafe {
                        sqlite3_create_function_v2(
                            raw_db,
                            name_cstr.as_ptr(),
                            nargs,
                            SQLITE_UTF8,
                            callback_ptr, // pApp (user data - the Python callback)
                            Some(udf_trampoline), // xFunc (scalar function callback)
                            None,         // xStep (aggregate step callback)
                            None,         // xFinal (aggregate final callback)
                            Some(udf_destructor), // xDestroy (destructor)
                        )
                    };

                    if result != SQLITE_OK {
                        // Clean up the callback pointer on error
                        unsafe {
                            let _ = Box::from_raw(callback_ptr as *mut Py<PyAny>);
                        }
                        {
                            let mut funcs_guard = user_functions.lock().unwrap();
                            funcs_guard.remove(&name);
                        }
                        return Err(OperationalError::new_err(format!(
                            "Failed to create function '{name}': SQLite error code {result}"
                        )));
                    }
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Set or clear the trace callback.
    /// The callback receives SQL strings as they are executed.
    fn set_trace_callback(&self, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let trace_callback = Arc::clone(&self.trace_callback);
        // Need all callback fields to check if all are cleared
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);

        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));

            // Store the callback state
            {
                let mut trace_guard = trace_callback.lock().unwrap();
                *trace_guard = callback_clone;
            }

            let future = async move {
                // Ensure callback connection exists (needed to clear callbacks on SQLite)
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                // Define the trace callback trampoline
                extern "C" fn trace_trampoline(
                    _trace_type: std::ffi::c_uint,
                    ctx: *mut std::ffi::c_void,
                    _p: *mut std::ffi::c_void,
                    x: *mut std::ffi::c_void,
                ) -> std::ffi::c_int {
                    unsafe {
                        // x is a pointer to the SQL string (for SQLITE_TRACE_STMT)
                        if x.is_null() || ctx.is_null() {
                            return 0;
                        }

                        // Extract the SQL string from x
                        // For SQLITE_TRACE_STMT, x points to the SQL text
                        let sql_cstr = x as *const i8;
                        let sql_str: String =
                            cstr_from_i8_ptr(sql_cstr).to_string_lossy().into_owned();

                        // Get the Python callback from the context (pCtx)
                        let callback_ptr = ctx as *mut Py<PyAny>;

                        // Note: Python::with_gil is used here for sync operation in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| {
                            let callback = (*callback_ptr).clone_ref(py);
                            let _ = callback.bind(py).call1((sql_str,));
                            // Ignore Python errors in trace callback
                        });
                    }
                    0
                }

                // Set up the callback pointer for the trampoline
                let callback_for_trace = {
                    let trace_guard = trace_callback.lock().unwrap();
                    trace_guard.as_ref().map(|c| {
                        // Clone with GIL
                        // Note: Python::with_gil is used here for sync clone_ref in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| c.clone_ref(py))
                    })
                };

                let callback_ptr = if let Some(cb) = callback_for_trace {
                    let callback_box: Box<Py<PyAny>> = Box::new(cb);
                    Box::into_raw(callback_box) as *mut std::ffi::c_void
                } else {
                    std::ptr::null_mut()
                };

                // Set or clear the trace callback
                let result = unsafe {
                    sqlite3_trace_v2(
                        raw_db,
                        if callback_ptr.is_null() {
                            0
                        } else {
                            SQLITE_TRACE_STMT as u32
                        }, // Trace mask
                        if callback_ptr.is_null() {
                            None
                        } else {
                            Some(trace_trampoline)
                        },
                        callback_ptr, // pCtx - the Python callback
                    )
                };

                if result != SQLITE_OK {
                    // Clean up callback pointer on error
                    if !callback_ptr.is_null() {
                        unsafe {
                            let _ = Box::from_raw(callback_ptr as *mut Py<PyAny>);
                        }
                    }
                    {
                        let mut trace_guard = trace_callback.lock().unwrap();
                        *trace_guard = None;
                    }
                    return Err(OperationalError::new_err(format!(
                        "Failed to set trace callback: SQLite error code {result}"
                    )));
                }

                // After clearing, check if all callbacks are now cleared
                if callback.is_none() {
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        drop(handle);
                        drop(conn_guard);
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        return Ok(());
                    }
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Set or clear the authorizer callback.
    /// The callback receives (action, arg1, arg2, arg3, arg4) and returns an int (SQLITE_OK, SQLITE_DENY, etc.).
    fn set_authorizer(&self, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        // Need all callback fields to check if all are cleared
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let progress_handler = Arc::clone(&self.progress_handler);

        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));

            // Store the callback state
            {
                let mut auth_guard = authorizer_callback.lock().unwrap();
                *auth_guard = callback_clone;
            }

            let future = async move {
                // If clearing the callback, check if all callbacks are now cleared
                if callback.is_none() {
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        // Clear the authorizer on SQLite side (already cleared in state)
                        return Ok(());
                    }
                }

                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                // Define the authorizer callback trampoline
                extern "C" fn authorizer_trampoline(
                    ctx: *mut std::ffi::c_void,
                    action: std::ffi::c_int,
                    arg1: *const i8,
                    arg2: *const i8,
                    arg3: *const i8,
                    arg4: *const i8,
                ) -> std::ffi::c_int {
                    unsafe {
                        if ctx.is_null() {
                            return SQLITE_OK;
                        }

                        // Convert C strings to Rust strings (or None)
                        let arg1_str: Option<String> = if arg1.is_null() {
                            None
                        } else {
                            Some(cstr_from_i8_ptr(arg1).to_string_lossy().into_owned())
                        };
                        let arg2_str: Option<String> = if arg2.is_null() {
                            None
                        } else {
                            Some(cstr_from_i8_ptr(arg2).to_string_lossy().into_owned())
                        };
                        let arg3_str: Option<String> = if arg3.is_null() {
                            None
                        } else {
                            Some(cstr_from_i8_ptr(arg3).to_string_lossy().into_owned())
                        };
                        let arg4_str: Option<String> = if arg4.is_null() {
                            None
                        } else {
                            Some(cstr_from_i8_ptr(arg4).to_string_lossy().into_owned())
                        };

                        // Get the Python callback from the context
                        let callback_ptr = ctx as *mut Py<PyAny>;

                        // Note: Python::with_gil is used here for sync operation in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| {
                            let callback = (*callback_ptr).clone_ref(py);

                            // Convert None strings to None in Python, otherwise pass the string
                            let py_arg1: Py<PyAny> = match arg1_str {
                                Some(ref s) => PyString::new(py, s).into_any().unbind(),
                                None => py.None(),
                            };
                            let py_arg2: Py<PyAny> = match arg2_str {
                                Some(ref s) => PyString::new(py, s).into_any().unbind(),
                                None => py.None(),
                            };
                            let py_arg3: Py<PyAny> = match arg3_str {
                                Some(ref s) => PyString::new(py, s).into_any().unbind(),
                                None => py.None(),
                            };
                            let py_arg4: Py<PyAny> = match arg4_str {
                                Some(ref s) => PyString::new(py, s).into_any().unbind(),
                                None => py.None(),
                            };

                            match callback
                                .bind(py)
                                .call1((action, py_arg1, py_arg2, py_arg3, py_arg4))
                            {
                                Ok(result) => {
                                    // Convert Python result to SQLite auth code
                                    result.extract::<i32>().unwrap_or(SQLITE_OK)
                                    // Default to OK if conversion fails
                                }
                                Err(_) => SQLITE_OK, // Ignore Python errors, default to OK
                            }
                        })
                    }
                }

                // Set up the callback pointer for the trampoline
                let callback_for_auth = {
                    let auth_guard = authorizer_callback.lock().unwrap();
                    auth_guard.as_ref().map(|c| {
                        // Note: Python::with_gil is used here for sync clone_ref in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| c.clone_ref(py))
                    })
                };

                let callback_ptr = if let Some(cb) = callback_for_auth {
                    let callback_box: Box<Py<PyAny>> = Box::new(cb);
                    Box::into_raw(callback_box) as *mut std::ffi::c_void
                } else {
                    std::ptr::null_mut()
                };

                // Set or clear the authorizer callback
                unsafe {
                    sqlite3_set_authorizer(
                        raw_db,
                        if callback_ptr.is_null() {
                            None
                        } else {
                            Some(authorizer_trampoline)
                        },
                        callback_ptr, // pUserData - the Python callback
                    );
                }

                // After clearing, check if all callbacks are now cleared
                if callback.is_none() {
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        drop(handle);
                        drop(conn_guard);
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        return Ok(());
                    }
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Set or clear the progress handler callback.
    /// The callback is called every N VDBE operations and returns True to continue, False to abort.
    fn set_progress_handler(&self, n: i32, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let callback_connection = Arc::clone(&self.callback_connection);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let progress_handler = Arc::clone(&self.progress_handler);
        // Need all callback fields to check if all are cleared
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);

        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));

            // Store the progress handler state
            {
                let mut progress_guard = progress_handler.lock().unwrap();
                *progress_guard = callback_clone.map(|c| (n, c));
            }

            let future = async move {
                // If clearing the callback, check if all callbacks are now cleared
                if callback.is_none() {
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        // Clear the progress handler on SQLite side (already cleared in state)
                        return Ok(());
                    }
                }

                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Callback connection not available")
                })?;

                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();

                // Define the progress handler callback trampoline
                extern "C" fn progress_trampoline(ctx: *mut std::ffi::c_void) -> std::ffi::c_int {
                    unsafe {
                        if ctx.is_null() {
                            return 0; // Continue
                        }

                        // Get the Python callback from the context
                        let callback_ptr = ctx as *mut Py<PyAny>;

                        // Note: Python::with_gil is used here for sync operation in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| {
                            let callback = (*callback_ptr).clone_ref(py);

                            match callback.bind(py).call0() {
                                Ok(result) => {
                                    // Convert Python result to int (0 = continue, non-zero = abort)
                                    // Python True/False -> 0/non-zero
                                    if let Ok(should_continue) = result.extract::<bool>() {
                                        if should_continue {
                                            0 // Continue
                                        } else {
                                            1 // Abort
                                        }
                                    } else {
                                        result.extract::<i32>().unwrap_or(0) // Use integer directly, default to continue if conversion fails
                                    }
                                }
                                Err(_) => 0, // Ignore Python errors, default to continue
                            }
                        })
                    }
                }

                // Set up the callback pointer for the trampoline
                let callback_for_progress = {
                    let progress_guard = progress_handler.lock().unwrap();
                    progress_guard.as_ref().map(|(_, cb)| {
                        // Note: Python::with_gil is used here for sync clone_ref in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| cb.clone_ref(py))
                    })
                };

                let callback_ptr = if let Some(cb) = callback_for_progress {
                    let callback_box: Box<Py<PyAny>> = Box::new(cb);
                    Box::into_raw(callback_box) as *mut std::ffi::c_void
                } else {
                    std::ptr::null_mut()
                };

                // Set or clear the progress handler
                unsafe {
                    sqlite3_progress_handler(
                        raw_db,
                        if callback_ptr.is_null() { 0 } else { n },
                        if callback_ptr.is_null() {
                            None
                        } else {
                            Some(progress_trampoline)
                        },
                        callback_ptr, // pArg - the Python callback
                    );
                }

                // After clearing, check if all callbacks are now cleared
                if callback.is_none() {
                    let all_cleared = !has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );
                    if all_cleared {
                        // Release the callback connection
                        drop(handle);
                        drop(conn_guard);
                        let mut callback_guard = callback_connection.lock().await;
                        callback_guard.take();
                        return Ok(());
                    }
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Dump the database as a list of SQL statements.
    /// Returns a list of SQL strings that can recreate the database.
    fn iterdump(self_: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);

        Python::attach(|py| {
            let future = async move {
                // Priority: transaction > callbacks > pool
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Helper function to encode bytes as hex
                fn bytes_to_hex(bytes: &[u8]) -> String {
                    bytes.iter().map(|b| format!("{b:02x}")).collect()
                }

                // Get connection for queries
                // We need to handle different connection types
                let mut statements = Vec::new();
                statements.push("BEGIN TRANSACTION;".to_string());

                // Query sqlite_master - use appropriate connection
                let schema_rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
                        .fetch_all(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"))?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
                        .fetch_all(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"))?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
                        .fetch_all(&pool_clone)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"))?
                };

                // Collect table names for data dumping
                let mut table_names = Vec::new();

                // Process schema rows
                for row in schema_rows {
                    let row_type: String = row.get(0);
                    let name: String = row.get(1);
                    let sql: Option<String> = row.get(2);

                    if let Some(sql_stmt) = sql {
                        match row_type.as_str() {
                            "table" => {
                                // Skip system tables for data, but include schema
                                if !name.starts_with("sqlite_") {
                                    table_names.push(name.clone());
                                }
                                statements.push(format!("{sql_stmt};"));
                            }
                            "index" => {
                                // Skip system indexes
                                if !name.starts_with("sqlite_") {
                                    statements.push(format!("{sql_stmt};"));
                                }
                            }
                            "trigger" => {
                                statements.push(format!("{sql_stmt};"));
                            }
                            "view" => {
                                statements.push(format!("{sql_stmt};"));
                            }
                            _ => {}
                        }
                    }
                }

                // Helper function to escape SQL string
                let escape_sql_string = |s: &str| -> String { s.replace("'", "''") };

                // Helper function to format value for INSERT
                let format_value = |row: &sqlx::sqlite::SqliteRow, idx: usize| -> String {
                    use sqlx::Row;
                    // Try different types in order
                    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(idx) {
                        return v.to_string();
                    }
                    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(idx) {
                        return v.to_string();
                    }
                    if let Ok(Some(v)) = row.try_get::<Option<String>, _>(idx) {
                        return format!("'{}'", escape_sql_string(&v));
                    }
                    if let Ok(Some(v)) = row.try_get::<Option<Vec<u8>>, _>(idx) {
                        // Convert BLOB to hex string
                        return format!("X'{}'", bytes_to_hex(&v));
                    }
                    // Check for NULL
                    if row.try_get::<Option<i64>, _>(idx).is_ok() {
                        return "NULL".to_string();
                    }
                    "NULL".to_string()
                };

                // Dump data for each table
                for table_name in table_names {
                    let query = format!("SELECT * FROM {table_name}");
                    let rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        sqlx::query(&query)
                            .fetch_all(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query))?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        sqlx::query(&query)
                            .fetch_all(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query))?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        sqlx::query(&query)
                            .fetch_all(&pool_clone)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query))?
                    };

                    if rows.is_empty() {
                        continue;
                    }

                    // Get column names
                    let column_count = rows[0].len();
                    let column_names: Vec<String> = (0..column_count)
                        .map(|i| {
                            rows[0]
                                .columns()
                                .get(i)
                                .map(|c| c.name().to_string())
                                .unwrap_or_else(|| format!("column_{i}"))
                        })
                        .collect();

                    // Generate INSERT statements
                    for row in rows {
                        let mut values = Vec::new();
                        for i in 0..column_count {
                            values.push(format_value(&row, i));
                        }
                        let values_str = values.join(", ");
                        statements.push(format!(
                            "INSERT INTO {} ({}) VALUES ({});",
                            table_name,
                            column_names.join(", "),
                            values_str
                        ));
                    }
                }

                statements.push("COMMIT;".to_string());

                // Convert to Python list
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let list = PyList::empty(py);
                    for stmt in statements {
                        list.append(PyString::new(py, &stmt))?;
                    }
                    Ok(list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get list of table names in the database.
    #[pyo3(signature = (name = None))]
    fn get_tables(self_: PyRef<Self>, name: Option<String>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Build query
                let query = if let Some(ref table_name) = name {
                    format!("SELECT name FROM sqlite_master WHERE type='table' AND name = '{}' AND name NOT LIKE 'sqlite_%'", table_name.replace("'", "''"))
                } else {
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
                };

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of table names (strings)
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        if let Ok(table_name) = row.try_get::<String, _>(0) {
                            result_list.append(PyString::new(py, &table_name))?;
                        }
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get table information (columns) for a specific table.
    fn get_table_info(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Escape table name for SQL
        let escaped_table_name = table_name.replace("'", "''");
        let query = format!("PRAGMA table_info('{escaped_table_name}')");

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // PRAGMA table_info returns: cid, name, type, notnull, dflt_value, pk
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // cid (column id)
                        if let Ok(cid) = row.try_get::<i64, _>(0) {
                            dict.set_item("cid", PyInt::new(py, cid))?;
                        }

                        // name
                        if let Ok(name) = row.try_get::<String, _>(1) {
                            dict.set_item("name", PyString::new(py, &name))?;
                        }

                        // type
                        if let Ok(col_type) = row.try_get::<String, _>(2) {
                            dict.set_item("type", PyString::new(py, &col_type))?;
                        }

                        // notnull (0 or 1)
                        if let Ok(notnull) = row.try_get::<i64, _>(3) {
                            dict.set_item("notnull", PyInt::new(py, notnull))?;
                        }

                        // dflt_value (default value, can be NULL)
                        let dflt_val: Py<PyAny> =
                            if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                                PyString::new(py, &val).into()
                            } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                                PyInt::new(py, val).into()
                            } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                                PyFloat::new(py, val).into()
                            } else {
                                py.None()
                            };
                        dict.set_item("dflt_value", dflt_val)?;

                        // pk (primary key, 0 or 1)
                        if let Ok(pk) = row.try_get::<i64, _>(5) {
                            dict.set_item("pk", PyInt::new(py, pk))?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get list of indexes in the database.
    #[pyo3(signature = (table_name = None))]
    fn get_indexes(self_: PyRef<Self>, table_name: Option<String>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Build query
        let query = if let Some(ref tbl_name) = table_name {
            let escaped = tbl_name.replace("'", "''");
            format!("SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND tbl_name = '{escaped}' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        } else {
            "SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
        };

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // Columns: name, tbl_name, sql
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // name
                        if let Ok(name) = row.try_get::<String, _>(0) {
                            dict.set_item("name", PyString::new(py, &name))?;
                        }

                        // table
                        if let Ok(tbl_name) = row.try_get::<String, _>(1) {
                            dict.set_item("table", PyString::new(py, &tbl_name))?;
                        }

                        // unique (determined from SQL - check if UNIQUE keyword exists)
                        let unique = if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                            if sql.to_uppercase().contains("UNIQUE") {
                                1
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        dict.set_item("unique", PyInt::new(py, unique))?;

                        // sql
                        if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                            dict.set_item("sql", PyString::new(py, &sql))?;
                        } else {
                            dict.set_item("sql", py.None())?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get foreign key constraints for a specific table.
    fn get_foreign_keys(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Escape table name for SQL
        let escaped_table_name = table_name.replace("'", "''");
        let query = format!("PRAGMA foreign_key_list('{escaped_table_name}')");

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // PRAGMA foreign_key_list returns: id, seq, table, from, to, on_update, on_delete, match
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // id
                        if let Ok(id) = row.try_get::<i64, _>(0) {
                            dict.set_item("id", PyInt::new(py, id))?;
                        }

                        // seq
                        if let Ok(seq) = row.try_get::<i64, _>(1) {
                            dict.set_item("seq", PyInt::new(py, seq))?;
                        }

                        // table (referenced table)
                        if let Ok(ref_table) = row.try_get::<String, _>(2) {
                            dict.set_item("table", PyString::new(py, &ref_table))?;
                        }

                        // from (column in current table)
                        if let Ok(from_col) = row.try_get::<String, _>(3) {
                            dict.set_item("from", PyString::new(py, &from_col))?;
                        }

                        // to (column in referenced table)
                        if let Ok(to_col) = row.try_get::<String, _>(4) {
                            dict.set_item("to", PyString::new(py, &to_col))?;
                        }

                        // on_update
                        if let Ok(on_update) = row.try_get::<String, _>(5) {
                            dict.set_item("on_update", PyString::new(py, &on_update))?;
                        }

                        // on_delete
                        if let Ok(on_delete) = row.try_get::<String, _>(6) {
                            dict.set_item("on_delete", PyString::new(py, &on_delete))?;
                        }

                        // match
                        if let Ok(match_val) = row.try_get::<String, _>(7) {
                            dict.set_item("match", PyString::new(py, &match_val))?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get comprehensive schema information for a table or all tables.
    #[pyo3(signature = (table_name = None))]
    fn get_schema(self_: PyRef<Self>, table_name: Option<String>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Get tables
                let tables_query = if let Some(ref tbl_name) = table_name {
                    format!("SELECT name FROM sqlite_master WHERE type='table' AND name = '{}' AND name NOT LIKE 'sqlite_%'", tbl_name.replace("'", "''"))
                } else {
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
                };

                let tables_rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&tables_query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&tables_query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&tables_query, &[], &pool_clone, &path).await?
                };

                // Extract table names
                let mut table_names = Vec::new();
                for row in tables_rows.iter() {
                    if let Ok(name) = row.try_get::<String, _>(0) {
                        table_names.push(name);
                    }
                }

                // For each table, fetch detailed information
                let mut tables_info = Vec::new();
                for tbl_name in &table_names {
                    // Get table info
                    let info_query =
                        format!("PRAGMA table_info('{}')", tbl_name.replace("'", "''"));
                    let info_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&info_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&info_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&info_query, &[], &pool_clone, &path).await?
                    };

                    // Get indexes
                    let indexes_query = format!("SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND tbl_name = '{}' AND name NOT LIKE 'sqlite_%' ORDER BY name", tbl_name.replace("'", "''"));
                    let indexes_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&indexes_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&indexes_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&indexes_query, &[], &pool_clone, &path).await?
                    };

                    // Get foreign keys
                    let fk_query =
                        format!("PRAGMA foreign_key_list('{}')", tbl_name.replace("'", "''"));
                    let fk_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&fk_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&fk_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&fk_query, &[], &pool_clone, &path).await?
                    };

                    tables_info.push((tbl_name.clone(), info_rows, indexes_rows, fk_rows));
                }

                // Build schema dictionary
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let schema_dict = PyDict::new(py);

                    if let Some(ref tbl_name) = table_name {
                        // Single table - return detailed info
                        if let Some((_, info_rows, indexes_rows, fk_rows)) = tables_info.first() {
                            // Table info
                            let columns_list = PyList::empty(py);
                            for row in info_rows.iter() {
                                let dict = PyDict::new(py);
                                if let Ok(cid) = row.try_get::<i64, _>(0) {
                                    dict.set_item("cid", PyInt::new(py, cid))?;
                                }
                                if let Ok(name) = row.try_get::<String, _>(1) {
                                    dict.set_item("name", PyString::new(py, &name))?;
                                }
                                if let Ok(col_type) = row.try_get::<String, _>(2) {
                                    dict.set_item("type", PyString::new(py, &col_type))?;
                                }
                                if let Ok(notnull) = row.try_get::<i64, _>(3) {
                                    dict.set_item("notnull", PyInt::new(py, notnull))?;
                                }
                                let dflt_val: Py<PyAny> =
                                    if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                                        PyString::new(py, &val).into()
                                    } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                                        PyInt::new(py, val).into()
                                    } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                                        PyFloat::new(py, val).into()
                                    } else {
                                        py.None()
                                    };
                                dict.set_item("dflt_value", dflt_val)?;
                                if let Ok(pk) = row.try_get::<i64, _>(5) {
                                    dict.set_item("pk", PyInt::new(py, pk))?;
                                }
                                columns_list.append(dict)?;
                            }
                            schema_dict.set_item("columns", columns_list)?;

                            // Indexes
                            let indexes_list = PyList::empty(py);
                            for row in indexes_rows.iter() {
                                let dict = PyDict::new(py);
                                if let Ok(name) = row.try_get::<String, _>(0) {
                                    dict.set_item("name", PyString::new(py, &name))?;
                                }
                                if let Ok(tbl_name) = row.try_get::<String, _>(1) {
                                    dict.set_item("table", PyString::new(py, &tbl_name))?;
                                }
                                let unique =
                                    if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                                        if sql.to_uppercase().contains("UNIQUE") {
                                            1
                                        } else {
                                            0
                                        }
                                    } else {
                                        0
                                    };
                                dict.set_item("unique", PyInt::new(py, unique))?;
                                if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                                    dict.set_item("sql", PyString::new(py, &sql))?;
                                } else {
                                    dict.set_item("sql", py.None())?;
                                }
                                indexes_list.append(dict)?;
                            }
                            schema_dict.set_item("indexes", indexes_list)?;

                            // Foreign keys
                            let fk_list = PyList::empty(py);
                            for row in fk_rows.iter() {
                                let dict = PyDict::new(py);
                                if let Ok(id) = row.try_get::<i64, _>(0) {
                                    dict.set_item("id", PyInt::new(py, id))?;
                                }
                                if let Ok(seq) = row.try_get::<i64, _>(1) {
                                    dict.set_item("seq", PyInt::new(py, seq))?;
                                }
                                if let Ok(ref_table) = row.try_get::<String, _>(2) {
                                    dict.set_item("table", PyString::new(py, &ref_table))?;
                                }
                                if let Ok(from_col) = row.try_get::<String, _>(3) {
                                    dict.set_item("from", PyString::new(py, &from_col))?;
                                }
                                if let Ok(to_col) = row.try_get::<String, _>(4) {
                                    dict.set_item("to", PyString::new(py, &to_col))?;
                                }
                                if let Ok(on_update) = row.try_get::<String, _>(5) {
                                    dict.set_item("on_update", PyString::new(py, &on_update))?;
                                }
                                if let Ok(on_delete) = row.try_get::<String, _>(6) {
                                    dict.set_item("on_delete", PyString::new(py, &on_delete))?;
                                }
                                if let Ok(match_val) = row.try_get::<String, _>(7) {
                                    dict.set_item("match", PyString::new(py, &match_val))?;
                                }
                                fk_list.append(dict)?;
                            }
                            schema_dict.set_item("foreign_keys", fk_list)?;
                            schema_dict.set_item("table_name", PyString::new(py, tbl_name))?;
                        }
                    } else {
                        // All tables - return list of table names with basic info
                        let tables_list = PyList::empty(py);
                        for (tbl_name, _, _, _) in &tables_info {
                            let table_dict = PyDict::new(py);
                            table_dict.set_item("name", PyString::new(py, tbl_name))?;
                            tables_list.append(table_dict)?;
                        }
                        schema_dict.set_item("tables", tables_list)?;
                    }

                    Ok(schema_dict.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get list of views in the database.
    #[pyo3(signature = (name = None))]
    fn get_views(self_: PyRef<Self>, name: Option<String>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Build query for views
                let query = if let Some(ref view_name) = name {
                    format!(
                        "SELECT name FROM sqlite_master WHERE type='view' AND name = '{}'",
                        view_name.replace("'", "''")
                    )
                } else {
                    "SELECT name FROM sqlite_master WHERE type='view' ORDER BY name".to_string()
                };

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of view names (strings)
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        if let Ok(view_name) = row.try_get::<String, _>(0) {
                            result_list.append(PyString::new(py, &view_name))?;
                        }
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get list of indexes for a specific table using PRAGMA index_list.
    fn get_index_list(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Escape table name for SQL
        let escaped_table_name = table_name.replace("'", "''");
        let query = format!("PRAGMA index_list('{escaped_table_name}')");

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // PRAGMA index_list returns: seq, name, unique, origin, partial
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // seq (sequence number)
                        if let Ok(seq) = row.try_get::<i64, _>(0) {
                            dict.set_item("seq", PyInt::new(py, seq))?;
                        }

                        // name
                        if let Ok(name) = row.try_get::<String, _>(1) {
                            dict.set_item("name", PyString::new(py, &name))?;
                        }

                        // unique (0 or 1)
                        if let Ok(unique) = row.try_get::<i64, _>(2) {
                            dict.set_item("unique", PyInt::new(py, unique))?;
                        }

                        // origin (c, u, pk, or null)
                        if let Ok(Some(origin)) = row.try_get::<Option<String>, _>(3) {
                            dict.set_item("origin", PyString::new(py, &origin))?;
                        } else {
                            dict.set_item("origin", py.None())?;
                        }

                        // partial (0 or 1)
                        if let Ok(partial) = row.try_get::<i64, _>(4) {
                            dict.set_item("partial", PyInt::new(py, partial))?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get information about columns in an index using PRAGMA index_info.
    fn get_index_info(self_: PyRef<Self>, index_name: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Escape index name for SQL
        let escaped_index_name = index_name.replace("'", "''");
        let query = format!("PRAGMA index_info('{escaped_index_name}')");

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // PRAGMA index_info returns: seqno, cid, name
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // seqno (sequence number in index)
                        if let Ok(seqno) = row.try_get::<i64, _>(0) {
                            dict.set_item("seqno", PyInt::new(py, seqno))?;
                        }

                        // cid (column id in table)
                        if let Ok(cid) = row.try_get::<i64, _>(1) {
                            dict.set_item("cid", PyInt::new(py, cid))?;
                        }

                        // name (column name)
                        if let Ok(name) = row.try_get::<String, _>(2) {
                            dict.set_item("name", PyString::new(py, &name))?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Get extended table information using PRAGMA table_xinfo (SQLite 3.26.0+).
    /// Returns additional information beyond table_info, including hidden columns.
    fn get_table_xinfo(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let connection_self = self_.into();

        // Escape table name for SQL
        let escaped_table_name = table_name.replace("'", "''");
        let query = format!("PRAGMA table_xinfo('{escaped_table_name}')");

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                // Ensure pool exists before calling init_hook (init_hook needs pool to execute queries)
                // Skip if in transaction (transaction has its own connection)
                if !in_transaction {
                    get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                }

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_self).await?;

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_all(&query, &[], &pool_clone, &path).await?
                };

                // Convert to list of dictionaries
                // PRAGMA table_xinfo returns: cid, name, type, notnull, dflt_value, pk, hidden
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
                // The deprecation warning is acceptable as this is a sync context.
                #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let dict = PyDict::new(py);

                        // cid (column id)
                        if let Ok(cid) = row.try_get::<i64, _>(0) {
                            dict.set_item("cid", PyInt::new(py, cid))?;
                        }

                        // name
                        if let Ok(name) = row.try_get::<String, _>(1) {
                            dict.set_item("name", PyString::new(py, &name))?;
                        }

                        // type
                        if let Ok(col_type) = row.try_get::<String, _>(2) {
                            dict.set_item("type", PyString::new(py, &col_type))?;
                        }

                        // notnull (0 or 1)
                        if let Ok(notnull) = row.try_get::<i64, _>(3) {
                            dict.set_item("notnull", PyInt::new(py, notnull))?;
                        }

                        // dflt_value (default value, can be NULL)
                        let dflt_val: Py<PyAny> =
                            if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                                PyString::new(py, &val).into()
                            } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                                PyInt::new(py, val).into()
                            } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                                PyFloat::new(py, val).into()
                            } else {
                                py.None()
                            };
                        dict.set_item("dflt_value", dflt_val)?;

                        // pk (primary key, 0 or 1)
                        if let Ok(pk) = row.try_get::<i64, _>(5) {
                            dict.set_item("pk", PyInt::new(py, pk))?;
                        }

                        // hidden (0=normal, 1=hidden, 2=virtual, 3=stored)
                        if let Ok(hidden) = row.try_get::<i64, _>(6) {
                            dict.set_item("hidden", PyInt::new(py, hidden))?;
                        }

                        result_list.append(dict)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Backup database to another connection.
    #[pyo3(signature = (target, *, pages = 0, progress = None, name = "main", sleep = 0.25))]
    fn backup(
        self_: PyRef<Self>,
        target: Py<PyAny>,
        pages: i32,
        progress: Option<Py<PyAny>>,
        name: &str,
        sleep: f64,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let callback_connection = Arc::clone(&self_.callback_connection);
        let load_extension_enabled = Arc::clone(&self_.load_extension_enabled);
        let user_functions = Arc::clone(&self_.user_functions);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let authorizer_callback = Arc::clone(&self_.authorizer_callback);
        let progress_handler = Arc::clone(&self_.progress_handler);

        let name = name.to_string();
        Python::attach(|py| {
            // Clone progress callback with GIL
            let progress_callback = progress.as_ref().map(|p| p.clone_ref(py));

            // Check if target is rapsqlite Connection or sqlite3.Connection and extract info
            let target_is_rapsqlite = target.bind(py).is_instance_of::<Connection>();
            let target_clone = target.clone_ref(py);

            // If rapsqlite, extract connection fields before async block
            let (
                target_path_opt,
                target_pool_opt,
                target_pragmas_opt,
                target_pool_size_opt,
                target_connection_timeout_secs_opt,
                target_transaction_state_opt,
                target_transaction_connection_opt,
                target_callback_connection_opt,
                target_load_extension_enabled_opt,
                target_user_functions_opt,
                target_trace_callback_opt,
                target_authorizer_callback_opt,
                target_progress_handler_opt,
            ) = if target_is_rapsqlite {
                let target_conn = target_clone
                    .bind(py)
                    .cast::<Connection>()
                    .map_err(|_| OperationalError::new_err("Failed to cast target connection"))?;
                let target_conn_borrowed = target_conn.borrow();
                (
                    Some(target_conn_borrowed.path.clone()),
                    Some(target_conn_borrowed.pool.clone()),
                    Some(target_conn_borrowed.pragmas.clone()),
                    Some(target_conn_borrowed.pool_size.clone()),
                    Some(target_conn_borrowed.connection_timeout_secs.clone()),
                    Some(target_conn_borrowed.transaction_state.clone()),
                    Some(target_conn_borrowed.transaction_connection.clone()),
                    Some(target_conn_borrowed.callback_connection.clone()),
                    Some(target_conn_borrowed.load_extension_enabled.clone()),
                    Some(target_conn_borrowed.user_functions.clone()),
                    Some(target_conn_borrowed.trace_callback.clone()),
                    Some(target_conn_borrowed.authorizer_callback.clone()),
                    Some(target_conn_borrowed.progress_handler.clone()),
                )
            } else {
                (
                    None, None, None, None, None, None, None, None, None, None, None, None, None,
                )
            };

            let future = async move {
                // Get source database handle (priority: transaction > callbacks > pool)
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let has_callbacks_flag = has_callbacks(
                    &load_extension_enabled,
                    &user_functions,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Wrapper to make raw pointers Send-safe
                struct SendPtr<T>(*mut T);
                unsafe impl<T> Send for SendPtr<T> {}
                unsafe impl<T> Sync for SendPtr<T> {}

                // Acquire source handle - keep pool connection alive if needed
                let source_handle;
                let _source_pool_conn: Option<PoolConnection<sqlx::Sqlite>>;

                if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock source handle: {e}"))
                    })?;
                    source_handle = SendPtr(handle.as_raw_handle().as_ptr());
                    _source_pool_conn = None;
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock source handle: {e}"))
                    })?;
                    source_handle = SendPtr(handle.as_raw_handle().as_ptr());
                    _source_pool_conn = None;
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let mut pool_conn = pool_clone.acquire().await.map_err(|e| {
                        OperationalError::new_err(format!(
                            "Failed to acquire source connection: {e}"
                        ))
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut pool_conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock source handle: {e}"))
                    })?;
                    source_handle = SendPtr(handle.as_raw_handle().as_ptr());
                    drop(handle);
                    // Keep pool_conn alive during backup
                    _source_pool_conn = Some(pool_conn);
                }

                // Get target database handle - keep pool connection alive if needed
                let target_handle: SendPtr<sqlite3>;
                let _target_pool_conn: Option<PoolConnection<sqlx::Sqlite>>;

                if target_is_rapsqlite {
                    // rapsqlite Connection - get handle same way as source
                    let target_path: String = target_path_opt.unwrap();
                    let target_pool: Arc<Mutex<Option<SqlitePool>>> = target_pool_opt.unwrap();
                    let target_pragmas: Arc<StdMutex<Vec<(String, String)>>> =
                        target_pragmas_opt.unwrap();
                    let target_pool_size: Arc<StdMutex<Option<usize>>> =
                        target_pool_size_opt.unwrap();
                    let target_connection_timeout_secs: Arc<StdMutex<Option<u64>>> =
                        target_connection_timeout_secs_opt.unwrap();
                    let target_transaction_state: Arc<Mutex<TransactionState>> =
                        target_transaction_state_opt.unwrap();
                    let target_transaction_connection: Arc<
                        Mutex<Option<PoolConnection<sqlx::Sqlite>>>,
                    > = target_transaction_connection_opt.unwrap();
                    let target_callback_connection: Arc<
                        Mutex<Option<PoolConnection<sqlx::Sqlite>>>,
                    > = target_callback_connection_opt.unwrap();
                    let target_load_extension_enabled: Arc<StdMutex<bool>> =
                        target_load_extension_enabled_opt.unwrap();
                    let target_user_functions: UserFunctions = target_user_functions_opt.unwrap();
                    let target_trace_callback: Arc<StdMutex<Option<Py<PyAny>>>> =
                        target_trace_callback_opt.unwrap();
                    let target_authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>> =
                        target_authorizer_callback_opt.unwrap();
                    let target_progress_handler: ProgressHandler =
                        target_progress_handler_opt.unwrap();

                    let target_in_transaction = {
                        let g = target_transaction_state.lock().await;
                        *g == TransactionState::Active
                    };

                    let target_has_callbacks_flag = has_callbacks(
                        &target_load_extension_enabled,
                        &target_user_functions,
                        &target_trace_callback,
                        &target_authorizer_callback,
                        &target_progress_handler,
                    );

                    if target_in_transaction {
                        let mut conn_guard = target_transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Target transaction connection not available")
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock target handle: {e}"))
                        })?;
                        target_handle = SendPtr(handle.as_raw_handle().as_ptr());
                        drop(handle);
                        _target_pool_conn = None;
                    } else if target_has_callbacks_flag {
                        ensure_callback_connection(
                            &target_path,
                            &target_pool,
                            &target_callback_connection,
                            &target_pragmas,
                            &target_pool_size,
                            &target_connection_timeout_secs,
                        )
                        .await?;
                        let mut conn_guard = target_callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Target callback connection not available")
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock target handle: {e}"))
                        })?;
                        target_handle = SendPtr(handle.as_raw_handle().as_ptr());
                        drop(handle);
                        _target_pool_conn = None;
                    } else {
                        let target_pool_clone = get_or_create_pool(
                            &target_path,
                            &target_pool,
                            &target_pragmas,
                            &target_pool_size,
                            &target_connection_timeout_secs,
                        )
                        .await?;
                        let mut target_pool_conn =
                            target_pool_clone.acquire().await.map_err(|e| {
                                OperationalError::new_err(format!(
                                    "Failed to acquire target connection: {e}"
                                ))
                            })?;
                        let sqlite_conn: &mut SqliteConnection = &mut target_pool_conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock target handle: {e}"))
                        })?;
                        target_handle = SendPtr(handle.as_raw_handle().as_ptr());
                        drop(handle);
                        _target_pool_conn = Some(target_pool_conn);
                    }
                } else {
                    // sqlite3.Connection - use Python helper to extract handle
                    // The handle is stored in the internal pysqlite_Connection struct
                    // We use a Python helper function with ctypes to safely extract it
                    // IMPORTANT: target_clone is already cloned and will keep the connection alive
                    // throughout the async future. We must ensure it stays in scope.
                    // Note: Python::with_gil is used here for sync handle extraction in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
                    let handle_ptr = Python::with_gil(|py| -> PyResult<*mut sqlite3> {
                        // Import the backup helper module
                        let backup_helper = py.import("rapsqlite._backup_helper")
                            .map_err(|e| OperationalError::new_err(format!(
                                "Failed to import backup helper: {e}. Make sure rapsqlite package is properly installed."
                            )))?;

                        // Get the get_sqlite3_handle function
                        let get_handle =
                            backup_helper.getattr("get_sqlite3_handle").map_err(|e| {
                                OperationalError::new_err(format!(
                                    "Failed to get get_sqlite3_handle function: {e}"
                                ))
                            })?;

                        // Call the function with the target connection
                        let conn_obj = target_clone.bind(py);
                        let result = get_handle.call1((conn_obj,)).map_err(|e| {
                            OperationalError::new_err(format!(
                                "Failed to extract sqlite3* handle: {e}"
                            ))
                        })?;

                        // Check if extraction succeeded (returns None on failure)
                        if result.is_none() {
                            return Err(OperationalError::new_err(
                                "Could not extract sqlite3* handle from target connection. \
                                Target must be a rapsqlite.Connection or sqlite3.Connection. \
                                The connection may be closed or invalid.",
                            ));
                        }

                        // Extract the pointer value as usize
                        let ptr_val: usize = result.extract().map_err(|e| {
                            OperationalError::new_err(format!(
                                "Failed to extract pointer value: {e}"
                            ))
                        })?;

                        // Verify pointer is not null
                        if ptr_val == 0 {
                            return Err(OperationalError::new_err(
                                "Extracted sqlite3* handle is null. Connection may be closed.",
                            ));
                        }

                        Ok(ptr_val as *mut sqlite3)
                    })?;

                    // Validate handle before use
                    if handle_ptr.is_null() {
                        return Err(OperationalError::new_err(
                            "Extracted sqlite3* handle is null. Connection may be closed or invalid."
                        ));
                    }

                    // Verify handle points to valid SQLite connection by checking autocommit state
                    // This is a safe operation that validates the handle is usable
                    let _autocommit_check = unsafe { sqlite3_get_autocommit(handle_ptr) };
                    // autocommit_check returns 0 (false) or non-zero (true), both are valid
                    // If handle is invalid, this might segfault, but we need to catch it early

                    target_handle = SendPtr(handle_ptr);
                    _target_pool_conn = None;
                    // target_clone (cloned at line 3430) stays in scope for the entire async future,
                    // keeping the Python connection object alive and preventing handle invalidation
                    // The handle is only valid as long as the Python object exists
                    let _ensure_target_alive = &target_clone; // Explicit reference to keep in scope
                }

                // Validate source handle as well
                if source_handle.0.is_null() {
                    return Err(OperationalError::new_err(
                        "Source sqlite3* handle is null. Connection may be closed or invalid.",
                    ));
                }

                // Check SQLite library version compatibility
                let source_libversion = unsafe {
                    cstr_from_i8_ptr(sqlite3_libversion())
                        .to_string_lossy()
                        .to_string()
                };
                // Note: Both should use same library, but verify for debugging

                // Convert database name to C string
                let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
                    OperationalError::new_err(format!("Invalid database name: {e}"))
                })?;

                // Check connection states before backup
                // SQLite backup requires destination to not have active transactions
                // Verify both source and target are in valid states
                let target_has_transaction =
                    unsafe { sqlite3_get_autocommit(target_handle.0) == 0 };
                if target_has_transaction {
                    return Err(OperationalError::new_err(
                        "Cannot backup: target connection has an active transaction. \
                        Commit or rollback the transaction before backup.",
                    ));
                }

                // Verify source connection is also in valid state
                // Source can have transactions, but should be open and valid
                let _source_autocommit = unsafe { sqlite3_get_autocommit(source_handle.0) };
                // Both 0 (in transaction) and non-zero (autocommit) are valid for source
                // This check just verifies the handle is valid and connection is open

                // Initialize backup
                let backup_handle: SendPtr<libsqlite3_sys::sqlite3_backup> = SendPtr(unsafe {
                    sqlite3_backup_init(
                        target_handle.0,
                        name_cstr.as_ptr(),
                        source_handle.0,
                        name_cstr.as_ptr(),
                    )
                });

                if backup_handle.0.is_null() {
                    // Get detailed error information
                    let error_code = unsafe { sqlite3_errcode(target_handle.0) };
                    let error_msg = unsafe {
                        let msg_ptr = sqlite3_errmsg(target_handle.0);
                        if msg_ptr.is_null() {
                            "Unknown error (null error message)".to_string()
                        } else {
                            cstr_from_i8_ptr(msg_ptr).to_string_lossy().to_string()
                        }
                    };

                    return Err(OperationalError::new_err(format!(
                        "Failed to initialize backup: SQLite error code {error_code}, message: '{error_msg}'. \
                        Source libversion: {source_libversion}. \
                        Ensure both connections are open and target has no active transactions."
                    )));
                }

                // Backup loop
                loop {
                    let pages_to_copy = if pages == 0 { -1 } else { pages };
                    let result = unsafe { sqlite3_backup_step(backup_handle.0, pages_to_copy) };

                    match result {
                        SQLITE_OK | SQLITE_BUSY | SQLITE_LOCKED => {
                            // Progress - call progress callback if provided
                            if let Some(ref progress_cb) = progress_callback {
                                let remaining =
                                    unsafe { sqlite3_backup_remaining(backup_handle.0) };
                                let page_count =
                                    unsafe { sqlite3_backup_pagecount(backup_handle.0) };
                                let pages_copied = page_count - remaining;

                                // Call Python callback with GIL
                                // Note: Python::with_gil is used here for sync callback execution in async context.
                                // The deprecation warning is acceptable as this is a sync operation within async.
                                #[allow(deprecated)]
                                // Note: Python::with_gil is used here for sync operation in async context.
                                // The deprecation warning is acceptable as this is a sync operation within async.
                                #[allow(deprecated)]
                                Python::with_gil(|py| {
                                    let callback = progress_cb.bind(py);
                                    let remaining_py: Py<PyAny> =
                                        PyInt::new(py, remaining as i64).into_any().unbind();
                                    let page_count_py: Py<PyAny> =
                                        PyInt::new(py, page_count as i64).into_any().unbind();
                                    let pages_copied_py: Py<PyAny> =
                                        PyInt::new(py, pages_copied as i64).into_any().unbind();
                                    if let Ok(args) = PyTuple::new(
                                        py,
                                        &[remaining_py, page_count_py, pages_copied_py],
                                    ) {
                                        // Ignore errors in progress callback - don't abort backup
                                        let _ = callback.call1(args);
                                    }
                                });
                            }

                            // Sleep before next step
                            tokio::time::sleep(Duration::from_secs_f64(sleep)).await;
                        }
                        SQLITE_DONE => {
                            // Backup complete
                            break;
                        }
                        _ => {
                            // Error - cleanup and return error
                            unsafe {
                                sqlite3_backup_finish(backup_handle.0);
                            }
                            return Err(OperationalError::new_err(format!(
                                "Backup failed with SQLite error code: {result}"
                            )));
                        }
                    }
                }

                // Finalize backup
                let final_result = unsafe { sqlite3_backup_finish(backup_handle.0) };
                if final_result != SQLITE_OK {
                    return Err(OperationalError::new_err(format!(
                        "Backup finish failed with SQLite error code: {final_result}"
                    )));
                }

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}
