//! Async context-manager helper types (`ExecuteContextManager`, `TransactionContextManager`).

#![allow(non_local_definitions)]

use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::pool::PoolConnection;
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

static SAVEPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn next_savepoint_name() -> String {
    format!("sp_{}", SAVEPOINT_COUNTER.fetch_add(1, Ordering::Relaxed))
}

use crate::connection::ensure_not_closed;
use crate::conversion::{build_description_tuple, row_to_py_with_factory};
use crate::pool::{
    ensure_callback_connection, execute_init_hook_if_needed, get_or_create_pool, has_callbacks,
    pool_acquisition_error,
};
use crate::query::{
    bind_and_execute, bind_and_execute_on_connection, bind_and_fetch_all,
    bind_and_fetch_all_on_connection,
};
use crate::types::{ProgressHandler, SqliteParam, TransactionState, UserFunctions};
use crate::utils::is_dml_query;
use crate::{map_sqlx_error, Connection, Cursor, OperationalError};

/// Execute context manager returned by `Connection::execute()`.
/// Allows `async with db.execute(...)` pattern by being both awaitable and an async context manager.
#[pyclass]
pub(crate) struct ExecuteContextManager {
    pub(crate) cursor: Py<Cursor>,
    pub(crate) query: String,
    pub(crate) param_values: Vec<SqliteParam>,
    pub(crate) is_select: bool,
    // Connection state needed for execution
    pub(crate) path: String,
    pub(crate) pool: Arc<Mutex<Option<SqlitePool>>>,
    pub(crate) pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub(crate) pool_size: Arc<StdMutex<Option<usize>>>,
    pub(crate) connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) transaction_state: Arc<Mutex<TransactionState>>,
    pub(crate) transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) load_extension_enabled: Arc<StdMutex<bool>>,
    pub(crate) user_functions: UserFunctions,
    pub(crate) trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) progress_handler: ProgressHandler,
    pub(crate) init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) init_hook_called: Arc<StdMutex<bool>>,
    pub(crate) last_rowid: Arc<Mutex<i64>>,
    pub(crate) last_changes: Arc<Mutex<u64>>,
    pub(crate) connection: Py<Connection>,
    pub(crate) timeout: Arc<StdMutex<f64>>,
    pub(crate) isolation_level: Arc<StdMutex<Option<String>>>,
    pub(crate) closed: Arc<StdMutex<bool>>,
}

#[pymethods]
impl ExecuteContextManager {
    /// Async context manager entry - executes query if non-SELECT, then returns the cursor.
    fn __aenter__(slf: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        Python::attach(|py| {
            // Extract all fields before moving into async
            let query = slf.borrow(py).query.clone();
            let param_values = slf.borrow(py).param_values.clone();
            let is_select = slf.borrow(py).is_select;
            let path = slf.borrow(py).path.clone();
            let pool = Arc::clone(&slf.borrow(py).pool);
            let pragmas = Arc::clone(&slf.borrow(py).pragmas);
            let pool_size = Arc::clone(&slf.borrow(py).pool_size);
            let connection_timeout_secs = Arc::clone(&slf.borrow(py).connection_timeout_secs);
            let idle_timeout_secs = Arc::clone(&slf.borrow(py).idle_timeout_secs);
            let transaction_state = Arc::clone(&slf.borrow(py).transaction_state);
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let callback_connection = Arc::clone(&slf.borrow(py).callback_connection);
            let load_extension_enabled = Arc::clone(&slf.borrow(py).load_extension_enabled);
            let user_functions = Arc::clone(&slf.borrow(py).user_functions);
            let trace_callback = Arc::clone(&slf.borrow(py).trace_callback);
            let authorizer_callback = Arc::clone(&slf.borrow(py).authorizer_callback);
            let progress_handler = Arc::clone(&slf.borrow(py).progress_handler);
            let init_hook = Arc::clone(&slf.borrow(py).init_hook);
            let init_hook_called = Arc::clone(&slf.borrow(py).init_hook_called);
            let last_rowid = Arc::clone(&slf.borrow(py).last_rowid);
            let last_changes = Arc::clone(&slf.borrow(py).last_changes);
            let connection = slf.borrow(py).connection.clone_ref(py);
            let connection_for_fetch = connection.clone_ref(py);
            let cursor = slf.borrow(py).cursor.clone_ref(py);
            let timeout = Arc::clone(&slf.borrow(py).timeout);
            let isolation_level = Arc::clone(&slf.borrow(py).isolation_level);
            let closed = Arc::clone(&slf.borrow(py).closed);
            // Get cursor's results Arc to mark it as executed for non-SELECT queries
            // Note: Python::with_gil is used here for sync result caching in async context.
            // The deprecation warning is acceptable as this is a sync operation within async.
            #[allow(deprecated)]
            let _cursor_results = Python::with_gil(
                |_py| -> PyResult<Arc<StdMutex<Option<Vec<sqlx::sqlite::SqliteRow>>>>> {
                    // We can't easily get the results Arc from Py<Cursor>
                    // Instead, we'll handle this in fetchall() by checking if it's non-SELECT
                    // For now, we'll pass None and handle it in fetchall()
                    Ok(Arc::new(StdMutex::new(None))) // Placeholder - won't be used
                },
            )
            .unwrap_or_else(|_| Arc::new(StdMutex::new(None)));

            let future = async move {
                ensure_not_closed(&closed)?;
                // For non-SELECT queries, execute immediately when entering context
                if !is_select {
                    // Check if we're currently executing init_hook FIRST (before checking transaction state)
                    // If we're inside init_hook, we should use pool connection, not transaction connection
                    let hook_already_called = {
                        let guard = init_hook_called.lock().unwrap();
                        *guard
                    };

                    // Only check for Active state, not Starting (Starting means transaction is being set up).
                    // When inside init_hook and transaction is Active, use transaction_connection so we don't
                    // try to acquire a second connection from the pool (pool may have size 1).
                    let in_transaction = {
                        let g = transaction_state.lock().await;
                        *g == TransactionState::Active
                    };

                    if !in_transaction {
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                    }

                    // Only call init_hook if not already called (avoid re-entry during init_hook execution)
                    // This prevents deadlocks when init_hook calls conn.execute() which triggers __aenter__
                    execute_init_hook_if_needed(&init_hook, &init_hook_called, connection).await?;

                    // Re-check transaction state after init_hook (state may have changed during hook execution).
                    // When inside init_hook and transaction is Active, use transaction_connection so init_hook's
                    // conn.execute() runs on the same connection (avoids pool timeout when pool size is 1).
                    let in_transaction_after_hook = {
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

                    let result = if in_transaction_after_hook {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_execute_on_connection(&query, &param_values, conn, &path).await?
                    } else if has_callbacks_flag {
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;

                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_execute_on_connection(&query, &param_values, conn, &path).await?
                    } else if hook_already_called || !is_dml_query(&query) {
                        // Inside init_hook, or DDL (CREATE/DROP etc.): use pool only; no implicit transaction.
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                        bind_and_execute(&query, &param_values, &pool_clone, &path).await?
                    } else {
                        // Implicit transaction (aiosqlite compat): first DML (INSERT/UPDATE/DELETE)
                        // without explicit begin() starts a transaction; commit()/rollback() end it.
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                        let pool_size_val = {
                            let g = pool_size.lock().unwrap();
                            *g
                        };
                        let timeout_val = {
                            let g = connection_timeout_secs.lock().unwrap();
                            *g
                        };
                        let mut conn = pool_clone.acquire().await.map_err(|e| {
                            pool_acquisition_error(&path, &e, pool_size_val, timeout_val)
                        })?;
                        let timeout_ms = {
                            let g = timeout.lock().unwrap();
                            (*g * 1000.0) as i64
                        };
                        let busy = format!("PRAGMA busy_timeout = {}", timeout_ms);
                        sqlx::query(&busy)
                            .execute(&mut *conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &busy))?;
                        let level = isolation_level
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| "IMMEDIATE".to_string());
                        let begin_sql = format!("BEGIN {level}");
                        sqlx::query(&begin_sql)
                            .execute(&mut *conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &begin_sql))?;
                        let result =
                            bind_and_execute_on_connection(&query, &param_values, &mut conn, &path)
                                .await?;
                        {
                            let mut g = transaction_state.lock().await;
                            *g = TransactionState::Active;
                        }
                        {
                            let mut g = transaction_connection.lock().await;
                            *g = Some(conn);
                        }
                        result
                    };

                    let rowid = result.last_insert_rowid();
                    let changes = result.rows_affected();

                    *last_rowid.lock().await = rowid;
                    *last_changes.lock().await = changes;

                    // Update cursor lastrowid/rowcount (Phase 3.9)
                    #[allow(deprecated)]
                    let _ = Python::with_gil(|py| -> pyo3::PyResult<()> {
                        cursor
                            .bind(py)
                            .call_method1("_update_last_result", (rowid, changes as i64))?;
                        Ok(())
                    });

                    // Mark cursor results as cached (empty for non-SELECT) to prevent re-execution
                    // The fetchall() method will check if it's non-SELECT and results are None,
                    // and return empty results without executing. This is handled in fetchall().
                } else {
                    // For SELECT queries, ensure pool exists for lazy execution
                    // Only check for Active state, not Starting (Starting means transaction is being set up,
                    // and init_hook may need to execute queries using pool connection)
                    let in_transaction = {
                        let g = transaction_state.lock().await;
                        *g == TransactionState::Active
                    };

                    // Check if init_hook is already being executed (to avoid deadlock)
                    // If init_hook is already called, we're likely inside an init_hook execution
                    // In this case, we should skip pool operations to avoid deadlock with begin()/transaction()
                    let hook_already_called = {
                        let guard = init_hook_called.lock().unwrap();
                        *guard
                    };

                    // Only get/create pool if not in transaction and hook not already called
                    // If hook is already called, we're inside init_hook execution and should
                    // skip pool operations to avoid deadlock (begin()/transaction() will handle pool)
                    if !in_transaction && !hook_already_called {
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                    }

                    // Only call init_hook if not already called (avoid re-entry during init_hook execution)
                    // This prevents deadlocks when init_hook calls conn.execute() which triggers __aenter__
                    // Note: If hook is already called, we skip calling it again (returns early)
                    execute_init_hook_if_needed(&init_hook, &init_hook_called, connection).await?;

                    // If hook was already called and we're not in transaction, we need to ensure pool exists
                    // for the actual query execution (hook_already_called means we're inside hook execution,
                    // but the query still needs a connection)
                    if !in_transaction && hook_already_called {
                        // Pool should already exist (created by begin()/transaction()), but ensure it does
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                    }

                    // Eager execution for SELECT: fetch now so "async for row in cursor" works (DBAPI spec).
                    // When inside init_hook and transaction is Active, use transaction_connection (same as non-SELECT).
                    let in_transaction_after_hook = {
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
                    let rows = if in_transaction_after_hook {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&query, &param_values, conn, &path).await?
                    } else if has_callbacks_flag {
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(&query, &param_values, conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&query, &param_values, &pool_clone, &path).await?
                    };
                    #[allow(deprecated)]
                    let _ = Python::with_gil(|py| -> PyResult<()> {
                        let cur = cursor.bind(py);
                        let conn = connection_for_fetch.bind(py);
                        let rf = cur.getattr("row_factory").ok().map(|o| o.unbind());
                        let tf = conn.getattr("text_factory").ok().map(|o| o.unbind());
                        let rf_bound = rf.as_ref().map(|r| r.bind(py));
                        let tf_bound = tf.as_ref().map(|t| t.bind(py));
                        let mut py_vec = Vec::with_capacity(rows.len());
                        for row in &rows {
                            let out = row_to_py_with_factory(
                                py,
                                row,
                                rf_bound.as_ref().map(|b| b.as_ref()),
                                tf_bound.as_ref().map(|b| b.as_ref()),
                            )?;
                            py_vec.push(out.unbind());
                        }
                        let list = PyList::empty(py);
                        for o in &py_vec {
                            list.append(o.bind(py))?;
                        }
                        let desc: Py<PyAny> = if let Some(first) = rows.first() {
                            build_description_tuple(py, first)?.unbind().into()
                        } else {
                            py.None()
                        };
                        cur.call_method1("_set_select_results", (list, desc))?;
                        Ok(())
                    });
                }

                Ok(cursor)
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Async context manager exit - does nothing (cursor cleanup is automatic).
    fn __aexit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        Python::attach(|py| {
            let future = async move {
                Ok(false) // Return False to not suppress exceptions
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Make ExecuteContextManager awaitable - when awaited, calls __aenter__ and returns cursor.
    /// This allows both `await conn.execute(...)` and `async with conn.execute(...)` patterns.
    /// Python's __await__ must return an iterator. The Future from __aenter__ has __await__ which
    /// returns an iterator. So we call the Future's __await__ to get the iterator.
    fn __await__(slf: PyRef<Self>) -> PyResult<Py<PyAny>> {
        // Call __aenter__ to get the Future, then call its __await__ to get the iterator
        let slf: Py<Self> = slf.into();
        // Note: Python::with_gil is used here for sync operation in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::with_gil(|py| {
            let ctx_mgr = slf.bind(py);
            // Call __aenter__ to get the Future
            let future = ctx_mgr.call_method0("__aenter__")?;
            // Call the Future's __await__ to get the iterator
            future.call_method0("__await__").map(|bound| bound.unbind())
        })
    }
}

/// Transaction context manager returned by `Connection::transaction()`.
/// Runs begin on __aenter__ and commit/rollback on __aexit__ using the same
/// connection state as the Connection.
#[pyclass]
pub(crate) struct TransactionContextManager {
    pub(crate) path: String,
    pub(crate) pool: Arc<Mutex<Option<SqlitePool>>>,
    pub(crate) pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub(crate) pool_size: Arc<StdMutex<Option<usize>>>,
    pub(crate) connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) transaction_state: Arc<Mutex<TransactionState>>,
    pub(crate) transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) connection: Py<Connection>,
    pub(crate) init_hook: Arc<StdMutex<Option<Py<PyAny>>>>, // Optional initialization hook
    pub(crate) init_hook_called: Arc<StdMutex<bool>>,       // Track if init_hook has been executed
    pub(crate) timeout: Arc<StdMutex<f64>>,                 // SQLite busy_timeout in seconds
    pub(crate) isolation_level: Arc<StdMutex<Option<String>>>, // Phase 3.9: None | DEFERRED | IMMEDIATE | EXCLUSIVE
    pub(crate) explicit_transaction: Arc<Mutex<bool>>, // in_transaction() true only when explicit
}

#[pymethods]
impl TransactionContextManager {
    fn __aenter__(slf: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        Python::attach(|py| {
            let path = slf.borrow(py).path.clone();
            let pool = Arc::clone(&slf.borrow(py).pool);
            let pragmas = Arc::clone(&slf.borrow(py).pragmas);
            let pool_size = Arc::clone(&slf.borrow(py).pool_size);
            let connection_timeout_secs = Arc::clone(&slf.borrow(py).connection_timeout_secs);
            let idle_timeout_secs = Arc::clone(&slf.borrow(py).idle_timeout_secs);
            let transaction_state = Arc::clone(&slf.borrow(py).transaction_state);
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let connection = slf.borrow(py).connection.clone_ref(py);
            let init_hook = Arc::clone(&slf.borrow(py).init_hook);
            let init_hook_called = Arc::clone(&slf.borrow(py).init_hook_called);
            let timeout = Arc::clone(&slf.borrow(py).timeout);
            let isolation_level = Arc::clone(&slf.borrow(py).isolation_level);
            let explicit_transaction = Arc::clone(&slf.borrow(py).explicit_transaction);
            let future = async move {
                // Implicit → explicit: if we have implicit transaction only, commit it and reuse conn for explicit.
                {
                    let trans_guard = transaction_state.lock().await;
                    let ex_guard = explicit_transaction.lock().await;
                    let implicit_only = *trans_guard == TransactionState::Active && !*ex_guard;
                    drop(ex_guard);
                    if implicit_only {
                        let mut conn_guard = transaction_connection.lock().await;
                        if let Some(mut conn) = conn_guard.take() {
                            drop(trans_guard);
                            let _ = sqlx::query("COMMIT").execute(&mut *conn).await;
                            let timeout_ms = {
                                let g = timeout.lock().unwrap();
                                (*g * 1000.0) as i64
                            };
                            let _ = sqlx::query(&format!("PRAGMA busy_timeout = {timeout_ms}"))
                                .execute(&mut *conn)
                                .await;
                            let level = isolation_level
                                .lock()
                                .unwrap()
                                .clone()
                                .unwrap_or_else(|| "IMMEDIATE".to_string());
                            let begin_sql = format!("BEGIN {level}");
                            sqlx::query(&begin_sql)
                                .execute(&mut *conn)
                                .await
                                .map_err(|e| map_sqlx_error(e, &path, &begin_sql))?;
                            *conn_guard = Some(conn);
                            let mut trans_guard = transaction_state.lock().await;
                            *trans_guard = TransactionState::Active;
                            drop(trans_guard);
                            let mut ex_guard = explicit_transaction.lock().await;
                            *ex_guard = true;
                            return Ok(connection.into());
                        }
                        // No conn despite implicit state; fall through to normal path
                    } else if trans_guard.is_active() {
                        return Err(OperationalError::new_err("Transaction already in progress"));
                    }
                } // Lock released

                let result: Result<Py<PyAny>, PyErr> = async {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                        &idle_timeout_secs,
                    )
                    .await?;

                    // Atomically reserve the transaction slot (init_hook runs after transaction is active)
                    {
                        let mut trans_guard = transaction_state.lock().await;
                        if trans_guard.is_active() {
                            return Err(OperationalError::new_err(
                                "Transaction already in progress",
                            ));
                        }
                        *trans_guard = TransactionState::Starting;
                    } // Lock released

                    let pool_size_val = {
                        let g = pool_size.lock().unwrap();
                        *g
                    };
                    let timeout_val = {
                        let g = connection_timeout_secs.lock().unwrap();
                        *g
                    };
                    let mut conn = pool_clone.acquire().await.map_err(|e| {
                        pool_acquisition_error(&path, &e, pool_size_val, timeout_val)
                    })?;
                    // Set PRAGMA busy_timeout on this connection to handle lock contention
                    // Convert timeout from seconds (float) to milliseconds (integer) for SQLite
                    let timeout_ms = {
                        let timeout_guard = timeout.lock().unwrap();
                        (*timeout_guard * 1000.0) as i64
                    };
                    let busy_timeout_query = format!("PRAGMA busy_timeout = {}", timeout_ms);
                    sqlx::query(&busy_timeout_query)
                        .execute(&mut *conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, &busy_timeout_query))?;
                    let level = isolation_level
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "IMMEDIATE".to_string());
                    let begin_sql = format!("BEGIN {level}");
                    sqlx::query(&begin_sql)
                        .execute(&mut *conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, &begin_sql))?;
                    {
                        let mut conn_guard = transaction_connection.lock().await;
                        *conn_guard = Some(conn);
                    }
                    // Re-acquire lock to set transaction state and mark explicit
                    {
                        let mut trans_guard = transaction_state.lock().await;
                        *trans_guard = TransactionState::Active;
                    }
                    {
                        let mut ex_guard = explicit_transaction.lock().await;
                        *ex_guard = true;
                    }
                    // Run init_hook after transaction is active so hook's conn.execute() uses this connection
                    #[allow(deprecated)]
                    let connection_for_hook = Python::with_gil(|py| connection.clone_ref(py));
                    execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_for_hook)
                        .await?;
                    Ok(connection.into())
                }
                .await;

                // On failure, release the reservation.
                if result.is_err() {
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::None;
                    let mut ex_guard = explicit_transaction.lock().await;
                    *ex_guard = false;
                    let mut conn_guard = transaction_connection.lock().await;
                    conn_guard.take();
                }

                result
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    fn __aexit__(
        slf: PyRef<Self>,
        exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        let rollback = exc_type.is_some();
        Python::attach(|py| {
            let path = slf.borrow(py).path.clone();
            let transaction_state = Arc::clone(&slf.borrow(py).transaction_state);
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let explicit_transaction = Arc::clone(&slf.borrow(py).explicit_transaction);
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    // No-op when !Active (match Connection.commit/rollback; sqlite3/aiosqlite compat)
                    return Ok(());
                }
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard.take().ok_or_else(|| {
                    OperationalError::new_err("Transaction connection not available")
                })?;
                let query = if rollback { "ROLLBACK" } else { "COMMIT" };
                sqlx::query(query)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, query))?;
                drop(conn);
                *trans_guard = TransactionState::None;
                drop(trans_guard);
                let mut ex_guard = explicit_transaction.lock().await;
                *ex_guard = false;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}

/// Savepoint context manager returned by `Connection::savepoint()`.
/// SAVEPOINT on enter; RELEASE on clean exit, ROLLBACK TO on exception.
#[pyclass]
pub(crate) struct SavepointContextManager {
    pub(crate) path: String,
    pub(crate) transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) transaction_state: Arc<Mutex<TransactionState>>,
    pub(crate) name: String,
}

#[pymethods]
impl SavepointContextManager {
    fn __aenter__(slf: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        Python::attach(|py| {
            let path = slf.borrow(py).path.clone();
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let transaction_state = Arc::clone(&slf.borrow(py).transaction_state);
            let name = slf.borrow(py).name.clone();
            let future = async move {
                {
                    let g = transaction_state.lock().await;
                    if *g != TransactionState::Active {
                        return Err(OperationalError::new_err(
                            "Savepoint requires an active transaction. Call begin() or use transaction() first.",
                        ));
                    }
                }
                let mut conn_guard = transaction_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Transaction connection not available")
                })?;
                let save_sql = format!("SAVEPOINT {name}");
                sqlx::query(&save_sql)
                    .execute(&mut **conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &save_sql))?;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    fn __aexit__(
        slf: PyRef<Self>,
        exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let slf: Py<Self> = slf.into();
        let rollback = exc_type.is_some();
        Python::attach(|py| {
            let path = slf.borrow(py).path.clone();
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let name = slf.borrow(py).name.clone();
            let future = async move {
                let mut conn_guard = transaction_connection.lock().await;
                let conn = conn_guard.as_mut().ok_or_else(|| {
                    OperationalError::new_err("Transaction connection not available")
                })?;
                let sql = if rollback {
                    format!("ROLLBACK TO SAVEPOINT {name}")
                } else {
                    format!("RELEASE SAVEPOINT {name}")
                };
                sqlx::query(&sql)
                    .execute(&mut **conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &sql))?;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}
