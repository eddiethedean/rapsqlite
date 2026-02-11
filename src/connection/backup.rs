//! Raw SQLite backup loop (sqlite3_backup_*) and full backup flow. Used by Connection::backup().

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use libsqlite3_sys::{
    sqlite3, sqlite3_backup_finish, sqlite3_backup_init, sqlite3_backup_pagecount,
    sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_busy_timeout, sqlite3_errcode,
    sqlite3_errmsg, sqlite3_get_autocommit, sqlite3_libversion, SQLITE_BUSY, SQLITE_DONE,
    SQLITE_LOCKED, SQLITE_OK,
};
use pyo3::prelude::*;
use pyo3::types::{PyInt, PyTuple};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqliteConnection;
use sqlx::Sqlite;

use crate::pool::{
    acquire_with_pragmas, ensure_callback_connection, get_or_create_pool, has_callbacks,
    PoolConnectionSlot, PoolSlot, TakenConnectionGuard,
};
use crate::types::{
    ProgressHandler, TransactionState, UserAggregates, UserCollations, UserFunctions,
};
use crate::utils::cstr_from_c_char_ptr;
use crate::{InternalError, OperationalError};

use super::ensure_not_closed;

/// Wrapper to make raw pointers Send + Sync for use across await points.
pub(crate) struct SendPtr<T>(pub(crate) *mut T);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

/// Parameters for the backup loop (avoids too many function arguments).
pub(crate) struct BackupParams<'a> {
    pub(crate) source_handle: SendPtr<sqlite3>,
    pub(crate) target_handle: SendPtr<sqlite3>,
    pub(crate) name: &'a str,
    pub(crate) pages: i32,
    pub(crate) sleep: f64,
    pub(crate) progress_callback: Option<Py<PyAny>>,
    pub(crate) backup_busy_timeout_secs: u64,
    pub(crate) source_libversion: String,
}

/// Run the sqlite3 backup loop: init, step (with progress callback and sleep), finish.
/// Caller must ensure source_handle and target_handle are valid and exclusive for the duration.
pub(crate) async fn run_backup_loop(params: BackupParams<'_>) -> Result<(), PyErr> {
    let timeout_ms: std::ffi::c_int = (params.backup_busy_timeout_secs * 1000) as std::ffi::c_int;
    unsafe {
        sqlite3_busy_timeout(params.source_handle.0, timeout_ms);
        sqlite3_busy_timeout(params.target_handle.0, timeout_ms);
    }

    let name_cstr = std::ffi::CString::new(params.name.to_string())
        .map_err(|e| OperationalError::new_err(format!("Invalid database name: {e}")))?;
    let backup_handle = SendPtr(unsafe {
        sqlite3_backup_init(
            params.target_handle.0,
            name_cstr.as_ptr(),
            params.source_handle.0,
            name_cstr.as_ptr(),
        )
    });

    if backup_handle.0.is_null() {
        let error_code = unsafe { sqlite3_errcode(params.target_handle.0) };
        let error_msg = unsafe {
            let msg_ptr = sqlite3_errmsg(params.target_handle.0);
            if msg_ptr.is_null() {
                "Unknown error (null error message)".to_string()
            } else {
                cstr_from_c_char_ptr(msg_ptr as *const std::ffi::c_char)
                    .to_string_lossy()
                    .to_string()
            }
        };
        return Err(OperationalError::new_err(format!(
            "Failed to initialize backup: SQLite error code {error_code}, message: '{error_msg}'. \
            Source libversion: {}. \
            Ensure both connections are open and target has no active transactions.",
            params.source_libversion
        )));
    }

    let backup_start = Instant::now();
    loop {
        let pages_to_copy = if params.pages == 0 { -1 } else { params.pages };
        let step_result = unsafe { sqlite3_backup_step(backup_handle.0, pages_to_copy) };

        match step_result {
            SQLITE_OK | SQLITE_BUSY | SQLITE_LOCKED => {
                if (step_result == SQLITE_BUSY || step_result == SQLITE_LOCKED)
                    && backup_start.elapsed() > Duration::from_secs(params.backup_busy_timeout_secs)
                {
                    unsafe {
                        sqlite3_backup_finish(backup_handle.0);
                    }
                    return Err(OperationalError::new_err(format!(
                        "Backup timed out: database busy or locked after {} seconds",
                        params.backup_busy_timeout_secs
                    )));
                }

                if let Some(ref progress_cb) = params.progress_callback {
                    let remaining = unsafe { sqlite3_backup_remaining(backup_handle.0) };
                    let page_count = unsafe { sqlite3_backup_pagecount(backup_handle.0) };
                    let pages_copied = page_count - remaining;
                    #[allow(deprecated)]
                    Python::with_gil(|py| {
                        let callback = progress_cb.bind(py);
                        let remaining_py: Py<PyAny> =
                            PyInt::new(py, remaining as i64).into_any().unbind();
                        let page_count_py: Py<PyAny> =
                            PyInt::new(py, page_count as i64).into_any().unbind();
                        let pages_copied_py: Py<PyAny> =
                            PyInt::new(py, pages_copied as i64).into_any().unbind();
                        if let Ok(args) =
                            PyTuple::new(py, &[remaining_py, page_count_py, pages_copied_py])
                        {
                            let _ = callback.call1(args);
                        }
                    });
                }

                if step_result == SQLITE_BUSY || step_result == SQLITE_LOCKED {
                    tokio::time::sleep(Duration::from_secs_f64(params.sleep)).await;
                }
            }
            SQLITE_DONE => {
                if let Some(ref progress_cb) = params.progress_callback {
                    let page_count = unsafe { sqlite3_backup_pagecount(backup_handle.0) };
                    #[allow(deprecated)]
                    Python::with_gil(|py| {
                        let callback = progress_cb.bind(py);
                        let remaining_py: Py<PyAny> = PyInt::new(py, 0i64).into_any().unbind();
                        let page_count_py: Py<PyAny> =
                            PyInt::new(py, page_count as i64).into_any().unbind();
                        let pages_copied_py: Py<PyAny> =
                            PyInt::new(py, page_count as i64).into_any().unbind();
                        if let Ok(args) =
                            PyTuple::new(py, &[remaining_py, page_count_py, pages_copied_py])
                        {
                            let _ = callback.call1(args);
                        }
                    });
                }
                break;
            }
            _ => {
                unsafe {
                    sqlite3_backup_finish(backup_handle.0);
                }
                return Err(OperationalError::new_err(format!(
                    "Backup failed with SQLite error code: {step_result}"
                )));
            }
        }
    }

    let final_result = unsafe { sqlite3_backup_finish(backup_handle.0) };
    if final_result != SQLITE_OK {
        return Err(OperationalError::new_err(format!(
            "Backup finish failed with SQLite error code: {final_result}"
        )));
    }

    Ok(())
}

// --- Full backup flow: acquire source/target connections, run loop, restore. ---

/// Context for the source connection (self) when running backup.
pub(crate) struct BackupSourceContext {
    pub closed: Arc<StdMutex<bool>>,
    pub path: String,
    pub pool: Arc<Mutex<PoolSlot>>,
    pub pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub pool_size: Arc<StdMutex<Option<usize>>>,
    pub connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub transaction_state: Arc<Mutex<TransactionState>>,
    pub transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub load_extension_enabled: Arc<StdMutex<bool>>,
    pub user_functions: UserFunctions,
    pub user_aggregates: UserAggregates,
    pub user_collations: UserCollations,
    pub trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub progress_handler: ProgressHandler,
}

/// Context for the target connection when it is a rapsqlite Connection.
pub(crate) struct BackupTargetRapsqliteContext {
    pub path: String,
    pub pool: Arc<Mutex<PoolSlot>>,
    pub pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub pool_size: Arc<StdMutex<Option<usize>>>,
    pub connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub transaction_state: Arc<Mutex<TransactionState>>,
    pub transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub load_extension_enabled: Arc<StdMutex<bool>>,
    pub user_functions: UserFunctions,
    pub user_aggregates: UserAggregates,
    pub user_collations: UserCollations,
    pub trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub progress_handler: ProgressHandler,
}

/// Target of backup: either a rapsqlite Connection (context) or a sqlite3.Connection (Python object).
pub(crate) enum BackupTarget {
    Rapsqlite(BackupTargetRapsqliteContext),
    Sqlite3(Py<PyAny>),
}

/// Run the full backup: ensure not closed, acquire source and target connections,
/// get raw handles, run backup loop, restore connections.
pub(crate) async fn run_backup(
    source: BackupSourceContext,
    target: BackupTarget,
    name: String,
    pages: i32,
    progress_callback: Option<Py<PyAny>>,
    sleep: f64,
) -> Result<(), PyErr> {
    ensure_not_closed(&source.closed)?;

    let mut source_taken = TakenConnectionGuard::default();
    let mut target_taken = TakenConnectionGuard::default();
    let mut source_pool_conn = PoolConnectionSlot::default();
    let mut target_pool_conn = PoolConnectionSlot::default();

    let result: Result<(), PyErr> = async {
        let in_transaction = {
            let g = source.transaction_state.lock().await;
            g.is_active()
        };
        let has_callbacks_flag = has_callbacks(
            &source.load_extension_enabled,
            &source.user_functions,
            &source.user_aggregates,
            &source.user_collations,
            &source.trace_callback,
            &source.authorizer_callback,
            &source.progress_handler,
        );

        // Acquire source connection
        if in_transaction {
            let mut guard = source.transaction_connection.lock().await;
            let conn = guard.0.take().ok_or_else(|| {
                OperationalError::new_err("Transaction connection not available")
            })?;
            source_taken = TakenConnectionGuard::new(Arc::clone(&source.transaction_connection), conn);
        } else if has_callbacks_flag {
            ensure_callback_connection(
                &source.path,
                &source.pool,
                &source.callback_connection,
                &source.pragmas,
                &source.pool_size,
                &source.connection_timeout_secs,
                &source.idle_timeout_secs,
            )
            .await?;
            let mut guard = source.callback_connection.lock().await;
            let conn = guard.0.take().ok_or_else(|| {
                OperationalError::new_err("Callback connection not available")
            })?;
            source_taken = TakenConnectionGuard::new(Arc::clone(&source.callback_connection), conn);
        } else {
            let pool_clone = get_or_create_pool(
                &source.path,
                &source.pool,
                &source.pragmas,
                &source.pool_size,
                &source.connection_timeout_secs,
                &source.idle_timeout_secs,
            )
            .await?;
            let pool_size_val = *source.pool_size.lock().unwrap();
            let timeout_val = *source.connection_timeout_secs.lock().unwrap();
            source_pool_conn.0 = Some(
                acquire_with_pragmas(
                    &pool_clone,
                    &source.pragmas,
                    &source.path,
                    pool_size_val,
                    timeout_val,
                )
                .await?,
            );
        }

        let source_conn: &mut PoolConnection<Sqlite> = if let Some(conn) = source_taken.as_mut() {
            conn
        } else {
            source_pool_conn.0.as_mut().ok_or_else(|| {
                InternalError::new_err("internal error: source_pool_conn must exist")
            })?
        };

        // Acquire target handle
        let target_handle: SendPtr<sqlite3> = match &target {
            BackupTarget::Rapsqlite(t) => {
                let target_in_transaction = {
                    let g = t.transaction_state.lock().await;
                    g.is_active()
                };
                let target_has_callbacks_flag = has_callbacks(
                    &t.load_extension_enabled,
                    &t.user_functions,
                    &t.user_aggregates,
                    &t.user_collations,
                    &t.trace_callback,
                    &t.authorizer_callback,
                    &t.progress_handler,
                );

                if target_in_transaction {
                    let mut guard = t.transaction_connection.lock().await;
                    let conn = guard.0.take().ok_or_else(|| {
                        OperationalError::new_err("Target transaction connection not available")
                    })?;
                    target_taken = TakenConnectionGuard::new(Arc::clone(&t.transaction_connection), conn);
                } else if target_has_callbacks_flag {
                    ensure_callback_connection(
                        &t.path,
                        &t.pool,
                        &t.callback_connection,
                        &t.pragmas,
                        &t.pool_size,
                        &t.connection_timeout_secs,
                        &t.idle_timeout_secs,
                    )
                    .await?;
                    let mut guard = t.callback_connection.lock().await;
                    let conn = guard.0.take().ok_or_else(|| {
                        OperationalError::new_err("Target callback connection not available")
                    })?;
                    target_taken = TakenConnectionGuard::new(Arc::clone(&t.callback_connection), conn);
                } else {
                    let target_pool_clone = get_or_create_pool(
                        &t.path,
                        &t.pool,
                        &t.pragmas,
                        &t.pool_size,
                        &t.connection_timeout_secs,
                        &t.idle_timeout_secs,
                    )
                    .await?;
                    let target_pool_size_val = *t.pool_size.lock().unwrap();
                    let target_timeout_val = *t.connection_timeout_secs.lock().unwrap();
                    target_pool_conn.0 = Some(
                        acquire_with_pragmas(
                            &target_pool_clone,
                            &t.pragmas,
                            &t.path,
                            target_pool_size_val,
                            target_timeout_val,
                        )
                        .await?,
                    );
                }

                let target_conn: &mut PoolConnection<Sqlite> =
                    if let Some(conn) = target_taken.as_mut() {
                        conn
                    } else {
                        target_pool_conn.0.as_mut().ok_or_else(|| {
                            InternalError::new_err("internal error: target_pool_conn must exist")
                        })?
                    };

                let sqlite_conn: &mut SqliteConnection = &mut *target_conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock target handle: {e}"))
                })?;
                SendPtr(handle.as_raw_handle().as_ptr())
            }
            BackupTarget::Sqlite3(target_clone) => {
                #[allow(deprecated)]
                let handle_ptr = Python::with_gil(|py| -> PyResult<*mut sqlite3> {
                    let backup_helper = py.import("rapsqlite._backup_helper").map_err(|e| {
                        OperationalError::new_err(format!(
                            "Failed to import backup helper: {e}. Make sure rapsqlite package is properly installed."
                        ))
                    })?;
                    let get_handle = backup_helper.getattr("get_sqlite3_handle").map_err(|e| {
                        OperationalError::new_err(format!(
                            "Failed to get get_sqlite3_handle function: {e}"
                        ))
                    })?;
                    let conn_obj = target_clone.bind(py);
                    let result = get_handle.call1((conn_obj,)).map_err(|e| {
                        OperationalError::new_err(format!("Failed to extract sqlite3* handle: {e}"))
                    })?;
                    if result.is_none() {
                        return Err(OperationalError::new_err(
                            "Could not extract sqlite3* handle from target connection. \
                            Target must be a rapsqlite.Connection or sqlite3.Connection. \
                            The connection may be closed or invalid.",
                        ));
                    }
                    let ptr_val: usize = result.extract().map_err(|e| {
                        OperationalError::new_err(format!("Failed to extract pointer value: {e}"))
                    })?;
                    if ptr_val == 0 {
                        return Err(OperationalError::new_err(
                            "Extracted sqlite3* handle is null. Connection may be closed.",
                        ));
                    }
                    Ok(ptr_val as *mut sqlite3)
                })?;

                if handle_ptr.is_null() {
                    return Err(OperationalError::new_err(
                        "Extracted sqlite3* handle is null. Connection may be closed or invalid.",
                    ));
                }
                let _ensure_target_alive = target_clone;
                SendPtr(handle_ptr)
            }
        };

        // Source handle (must not hold lock across await)
        let source_handle = {
            let sqlite_conn: &mut SqliteConnection = &mut *source_conn;
            let mut guard = sqlite_conn.lock_handle().await.map_err(|e| {
                OperationalError::new_err(format!("Failed to lock source handle: {e}"))
            })?;
            SendPtr(guard.as_raw_handle().as_ptr())
        };

        if source_handle.0.is_null() {
            return Err(OperationalError::new_err(
                "Source sqlite3* handle is null. Connection may be closed or invalid.",
            ));
        }
        if target_handle.0.is_null() {
            return Err(OperationalError::new_err(
                "Target sqlite3* handle is null. Connection may be closed or invalid.",
            ));
        }

        let source_libversion = unsafe {
            cstr_from_c_char_ptr(sqlite3_libversion() as *const std::ffi::c_char)
                .to_string_lossy()
                .to_string()
        };

        let target_has_transaction = unsafe { sqlite3_get_autocommit(target_handle.0) == 0 };
        if target_has_transaction {
            return Err(OperationalError::new_err(
                "Cannot backup: target connection has an active transaction. \
                Commit or rollback the transaction before backup.",
            ));
        }

        let backup_busy_timeout_secs: u64 = source
            .connection_timeout_secs
            .lock()
            .unwrap()
            .unwrap_or(5)
            .clamp(5, 120);

        run_backup_loop(BackupParams {
            source_handle,
            target_handle,
            name: &name,
            pages,
            sleep,
            progress_callback,
            backup_busy_timeout_secs,
            source_libversion,
        })
        .await?;

        Ok(())
    }
    .await;

    if let Some((slot, conn)) = source_taken.take_for_restore() {
        let mut g = slot.lock().await;
        g.0 = Some(conn);
    }
    if let Some((slot, conn)) = target_taken.take_for_restore() {
        let mut g = slot.lock().await;
        g.0 = Some(conn);
    }

    result
}
