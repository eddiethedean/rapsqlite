//! Raw SQLite backup loop (sqlite3_backup_*). Used by Connection::backup().

use std::time::{Duration, Instant};

use libsqlite3_sys::{
    sqlite3, sqlite3_backup_finish, sqlite3_backup_init, sqlite3_backup_pagecount,
    sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_busy_timeout, sqlite3_errcode,
    sqlite3_errmsg, SQLITE_BUSY, SQLITE_DONE, SQLITE_LOCKED, SQLITE_OK,
};
use pyo3::prelude::*;
use pyo3::types::{PyInt, PyTuple};

use crate::utils::cstr_from_c_char_ptr;
use crate::OperationalError;

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
