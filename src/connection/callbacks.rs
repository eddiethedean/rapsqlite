//! UDF and callback implementation (create_function, create_aggregate, create_collation,
//! set_trace_callback, set_authorizer, set_progress_handler). Single responsibility: manage
//! callback connection and SQLite C API for these operations.

use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use libsqlite3_sys::{
    sqlite3_aggregate_context, sqlite3_context, sqlite3_create_collation_v2,
    sqlite3_create_function_v2, sqlite3_progress_handler, sqlite3_result_error,
    sqlite3_result_null, sqlite3_set_authorizer, sqlite3_user_data, sqlite3_value, SQLITE_DENY,
    SQLITE_DETERMINISTIC, SQLITE_OK, SQLITE_UTF8,
};
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};
use sqlx::sqlite::SqliteConnection;

use crate::conversion::{py_to_sqlite_c_result, sqlite_c_value_to_py};
use crate::pool::{ensure_callback_connection, has_callbacks, PoolConnectionSlot, PoolSlot};
use crate::types::{ProgressHandler, UserAggregates, UserCollations, UserFunctions};
use crate::utils::cstr_from_c_char_ptr;
use crate::OperationalError;

use super::ensure_not_closed;

/// Context for running callback/UDF operations. Built from Connection and passed
/// into callback async functions.
pub(crate) struct CallbackContext {
    pub closed: Arc<StdMutex<bool>>,
    pub path: String,
    pub pool: Arc<Mutex<PoolSlot>>,
    pub pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub pool_size: Arc<StdMutex<Option<usize>>>,
    pub connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub load_extension_enabled: Arc<StdMutex<bool>>,
    pub user_functions: UserFunctions,
    pub user_aggregates: UserAggregates,
    pub user_collations: UserCollations,
    pub trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub progress_handler: ProgressHandler,
    pub authorizer_callback_ctx_ptr: Arc<StdMutex<usize>>,
    pub progress_handler_ctx_ptr: Arc<StdMutex<usize>>,
}

#[inline]
fn drop_py_callback_ptr(ptr_usize: usize) {
    if ptr_usize == 0 {
        return;
    }
    // Context pointers are Arc<Py<PyAny>> stored as raw pointers so they can be
    // safely replaced/cleared without UAF if SQLite invokes callbacks concurrently.
    //
    // Dropping Py-owned values must happen with the GIL held.
    #[allow(deprecated)]
    Python::with_gil(|_py| unsafe {
        drop(Arc::from_raw(ptr_usize as *const Py<PyAny>));
    });
}

/// Set or clear the progress handler. Runs on the callback connection.
pub(crate) async fn set_progress_handler_impl(
    ctx: CallbackContext,
    n: i32,
    callback: Option<Py<PyAny>>,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    if callback.is_none() {
        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
            return Ok(());
        }
    }

    ensure_callback_connection(
        &ctx.path,
        &ctx.pool,
        &ctx.callback_connection,
        &ctx.pragmas,
        &ctx.pool_size,
        &ctx.connection_timeout_secs,
        &ctx.idle_timeout_secs,
    )
    .await?;

    let mut conn_guard = ctx.callback_connection.lock().await;
    let conn = conn_guard
        .0
        .as_mut()
        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;

    let sqlite_conn: &mut SqliteConnection = conn;
    let mut handle = sqlite_conn
        .lock_handle()
        .await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
    let raw_db = handle.as_raw_handle().as_ptr();

    extern "C" fn progress_trampoline(progress_ctx: *mut std::ffi::c_void) -> std::ffi::c_int {
        unsafe {
            if progress_ctx.is_null() {
                return 0;
            }
            let callback_ptr = progress_ctx as *const Py<PyAny>;
            #[allow(deprecated)]
            Python::with_gil(|py| {
                let arc = Arc::from_raw(callback_ptr);
                let callback = arc.clone();
                std::mem::forget(arc);
                let callback = callback.clone_ref(py);
                match callback.bind(py).call0() {
                    Ok(result) => {
                        if let Ok(should_continue) = result.extract::<bool>() {
                            if should_continue {
                                0
                            } else {
                                1
                            }
                        } else {
                            result.extract::<i32>().unwrap_or(0)
                        }
                    }
                    Err(_) => 0,
                }
            })
        }
    }

    let callback_for_progress = {
        let progress_guard = ctx.progress_handler.lock().unwrap();
        progress_guard.as_ref().map(|(_, cb)| {
            #[allow(deprecated)]
            Python::with_gil(|py| cb.clone_ref(py))
        })
    };

    let new_ptr_usize: usize = if let Some(cb) = callback_for_progress {
        let arc: Arc<Py<PyAny>> = Arc::new(cb);
        Arc::into_raw(arc) as usize
    } else {
        0
    };
    let callback_ptr = new_ptr_usize as *mut std::ffi::c_void;

    unsafe {
        sqlite3_progress_handler(
            raw_db,
            if callback_ptr.is_null() { 0 } else { n },
            if callback_ptr.is_null() {
                None
            } else {
                Some(progress_trampoline)
            },
            callback_ptr,
        );
    }

    // Swap pointer after SQLite install, free previous allocation if replaced/cleared.
    let old_ptr = {
        let mut g = ctx.progress_handler_ctx_ptr.lock().unwrap();
        std::mem::replace(&mut *g, new_ptr_usize)
    };
    if old_ptr != new_ptr_usize {
        drop_py_callback_ptr(old_ptr);
    }

    if callback.is_none() {
        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            drop(handle);
            drop(conn_guard);
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
            return Ok(());
        }
    }

    Ok(())
}

/// Set or clear the trace callback. Runs on the callback connection.
pub(crate) async fn set_trace_callback_impl(
    ctx: CallbackContext,
    _callback: Option<Py<PyAny>>,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    Ok(())
}

/// Create or remove a custom collation. Runs on the callback connection.
pub(crate) async fn create_collation_impl(
    ctx: CallbackContext,
    name: String,
    callable: Option<Py<PyAny>>,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    ensure_callback_connection(
        &ctx.path,
        &ctx.pool,
        &ctx.callback_connection,
        &ctx.pragmas,
        &ctx.pool_size,
        &ctx.connection_timeout_secs,
        &ctx.idle_timeout_secs,
    )
    .await?;

    let mut conn_guard = ctx.callback_connection.lock().await;
    let conn = conn_guard
        .0
        .as_mut()
        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;

    let sqlite_conn: &mut SqliteConnection = conn;
    let mut handle = sqlite_conn
        .lock_handle()
        .await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
    let raw_db = handle.as_raw_handle().as_ptr();

    if callable.is_none() {
        let old_ptr = {
            let mut guard = ctx.user_collations.lock().unwrap();
            guard.remove(&name)
        };

        let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
            OperationalError::new_err(format!("Collation name contains null byte: {e}"))
        })?;
        let result = unsafe {
            sqlite3_create_collation_v2(
                raw_db,
                name_cstr.as_ptr(),
                SQLITE_UTF8,
                std::ptr::null_mut(),
                None,
                None,
            )
        };
        if result != SQLITE_OK {
            // Restore map entry on error.
            if let Some(old_ptr) = old_ptr {
                ctx.user_collations
                    .lock()
                    .unwrap()
                    .insert(name.clone(), old_ptr);
            }
            return Err(OperationalError::new_err(format!(
                "Failed to remove collation '{name}': SQLite error code {result}"
            )));
        }
        if let Some(old_ptr) = old_ptr {
            #[allow(deprecated)]
            Python::with_gil(|_py| unsafe {
                let _ = Box::from_raw(old_ptr as *mut Py<PyAny>);
            });
        }

        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            drop(handle);
            drop(conn_guard);
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
        }
        return Ok(());
    }

    #[allow(deprecated)]
    let callback_for_storage = Python::with_gil(|py| callable.as_ref().unwrap().clone_ref(py));
    let callback_box: Box<Py<PyAny>> = Box::new(callback_for_storage);
    let callback_ptr = Box::into_raw(callback_box) as *mut std::ffi::c_void;
    let callback_ptr_usize = callback_ptr as usize;

    extern "C" fn collation_trampoline(
        p_arg: *mut std::ffi::c_void,
        len1: std::ffi::c_int,
        ptr1: *const std::ffi::c_void,
        len2: std::ffi::c_int,
        ptr2: *const std::ffi::c_void,
    ) -> std::ffi::c_int {
        unsafe {
            if p_arg.is_null() || ptr1.is_null() || ptr2.is_null() {
                return 0;
            }
            let len1 = len1 as usize;
            let len2 = len2 as usize;
            let s1 = if len1 == 0 {
                ""
            } else {
                let slice = std::slice::from_raw_parts(ptr1 as *const u8, len1);
                std::str::from_utf8(slice).unwrap_or("")
            };
            let s2 = if len2 == 0 {
                ""
            } else {
                let slice = std::slice::from_raw_parts(ptr2 as *const u8, len2);
                std::str::from_utf8(slice).unwrap_or("")
            };

            #[allow(deprecated)]
            let result = Python::with_gil(|py| {
                let callback_ptr = p_arg as *mut Py<PyAny>;
                let callback = (*callback_ptr).clone_ref(py);
                let s1_py = PyString::new(py, s1);
                let s2_py = PyString::new(py, s2);
                match callback.bind(py).call1((s1_py, s2_py)) {
                    Ok(ret) => {
                        let cmp: i32 = ret.extract().unwrap_or(0);
                        if cmp < 0 {
                            -1
                        } else if cmp > 0 {
                            1
                        } else {
                            0
                        }
                    }
                    Err(_) => 0,
                }
            });
            result
        }
    }

    let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
        OperationalError::new_err(format!("Collation name contains null byte: {e}"))
    })?;

    let result = unsafe {
        sqlite3_create_collation_v2(
            raw_db,
            name_cstr.as_ptr(),
            SQLITE_UTF8,
            callback_ptr,
            Some(collation_trampoline),
            None,
        )
    };

    if result != SQLITE_OK {
        unsafe {
            let _ = Box::from_raw(callback_ptr as *mut Py<PyAny>);
        }
        return Err(OperationalError::new_err(format!(
            "Failed to create collation '{name}': SQLite error code {result}"
        )));
    }

    // SQLite accepted the collation; take ownership of the pointer for explicit cleanup.
    let old_ptr = {
        let mut guard = ctx.user_collations.lock().unwrap();
        guard.insert(name.clone(), callback_ptr_usize)
    };
    if let Some(old_ptr) = old_ptr {
        #[allow(deprecated)]
        Python::with_gil(|_py| unsafe {
            let _ = Box::from_raw(old_ptr as *mut Py<PyAny>);
        });
    }

    Ok(())
}

/// Set or clear the authorizer callback. Runs on the callback connection.
pub(crate) async fn set_authorizer_impl(
    ctx: CallbackContext,
    callback: Option<Py<PyAny>>,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    if callback.is_none() {
        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
            return Ok(());
        }
    }

    ensure_callback_connection(
        &ctx.path,
        &ctx.pool,
        &ctx.callback_connection,
        &ctx.pragmas,
        &ctx.pool_size,
        &ctx.connection_timeout_secs,
        &ctx.idle_timeout_secs,
    )
    .await?;

    let mut conn_guard = ctx.callback_connection.lock().await;
    let conn = conn_guard
        .0
        .as_mut()
        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;

    let sqlite_conn: &mut SqliteConnection = conn;
    let mut handle = sqlite_conn
        .lock_handle()
        .await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
    let raw_db = handle.as_raw_handle().as_ptr();

    extern "C" fn authorizer_trampoline(
        authorizer_ctx: *mut std::ffi::c_void,
        action: std::ffi::c_int,
        arg1: *const std::ffi::c_char,
        arg2: *const std::ffi::c_char,
        arg3: *const std::ffi::c_char,
        arg4: *const std::ffi::c_char,
    ) -> std::ffi::c_int {
        unsafe {
            if authorizer_ctx.is_null() {
                return SQLITE_OK;
            }
            let arg1_str: Option<String> = if arg1.is_null() {
                None
            } else {
                Some(cstr_from_c_char_ptr(arg1).to_string_lossy().into_owned())
            };
            let arg2_str: Option<String> = if arg2.is_null() {
                None
            } else {
                Some(cstr_from_c_char_ptr(arg2).to_string_lossy().into_owned())
            };
            let arg3_str: Option<String> = if arg3.is_null() {
                None
            } else {
                Some(cstr_from_c_char_ptr(arg3).to_string_lossy().into_owned())
            };
            let arg4_str: Option<String> = if arg4.is_null() {
                None
            } else {
                Some(cstr_from_c_char_ptr(arg4).to_string_lossy().into_owned())
            };
            let callback_ptr = authorizer_ctx as *const Py<PyAny>;
            #[allow(deprecated)]
            Python::with_gil(|py| {
                let arc = Arc::from_raw(callback_ptr);
                let callback = arc.clone();
                std::mem::forget(arc);
                let callback = callback.clone_ref(py);
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
                    Ok(result) => result.extract::<i32>().unwrap_or(SQLITE_DENY),
                    Err(_) => SQLITE_DENY,
                }
            })
        }
    }

    let callback_for_auth = {
        let auth_guard = ctx.authorizer_callback.lock().unwrap();
        auth_guard.as_ref().map(|c| {
            #[allow(deprecated)]
            Python::with_gil(|py| c.clone_ref(py))
        })
    };

    let new_ptr_usize: usize = if let Some(cb) = callback_for_auth {
        let arc: Arc<Py<PyAny>> = Arc::new(cb);
        Arc::into_raw(arc) as usize
    } else {
        0
    };
    let callback_ptr = new_ptr_usize as *mut std::ffi::c_void;

    unsafe {
        sqlite3_set_authorizer(
            raw_db,
            if callback_ptr.is_null() {
                None
            } else {
                Some(authorizer_trampoline)
            },
            callback_ptr,
        );
    }

    // Swap pointer after SQLite install, free previous allocation if replaced/cleared.
    let old_ptr = {
        let mut g = ctx.authorizer_callback_ctx_ptr.lock().unwrap();
        std::mem::replace(&mut *g, new_ptr_usize)
    };
    if old_ptr != new_ptr_usize {
        drop_py_callback_ptr(old_ptr);
    }

    if callback.is_none() {
        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            drop(handle);
            drop(conn_guard);
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
            return Ok(());
        }
    }

    Ok(())
}

/// Create or remove a user-defined SQL function. Uses transaction_connection when present, else callback_connection.
pub(crate) async fn create_function_impl(
    ctx: CallbackContext,
    name: String,
    nargs: i32,
    func: Option<Py<PyAny>>,
    deterministic: bool,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    extern "C" fn udf_trampoline(
        udf_ctx: *mut sqlite3_context,
        argc: std::ffi::c_int,
        argv: *mut *mut sqlite3_value,
    ) {
        unsafe {
            let user_data = sqlite3_user_data(udf_ctx);
            if user_data.is_null() {
                sqlite3_result_null(udf_ctx);
                return;
            }
            let callback_ptr = user_data as *mut Py<PyAny>;
            #[allow(deprecated)]
            Python::with_gil(|py| {
                let callback = (*callback_ptr).clone_ref(py);
                let mut py_args: Vec<Py<PyAny>> = Vec::new();
                for i in 0..argc {
                    let value_ptr = *argv.add(i as usize);
                    match sqlite_c_value_to_py(py, value_ptr) {
                        Ok(py_val) => py_args.push(py_val),
                        Err(e) => {
                            let msg = format!("Error converting argument {i}: {e}");
                            sqlite3_result_error(
                                udf_ctx,
                                msg.as_ptr() as *const std::ffi::c_char,
                                msg.len() as i32,
                            );
                            return;
                        }
                    }
                }
                let result = match py_args.len() {
                    0 => callback.bind(py).call0(),
                    1 => callback.bind(py).call1((py_args[0].clone_ref(py),)),
                    2 => callback
                        .bind(py)
                        .call1((py_args[0].clone_ref(py), py_args[1].clone_ref(py))),
                    3 => callback.bind(py).call1((
                        py_args[0].clone_ref(py),
                        py_args[1].clone_ref(py),
                        py_args[2].clone_ref(py),
                    )),
                    4 => callback.bind(py).call1((
                        py_args[0].clone_ref(py),
                        py_args[1].clone_ref(py),
                        py_args[2].clone_ref(py),
                        py_args[3].clone_ref(py),
                    )),
                    5 => callback.bind(py).call1((
                        py_args[0].clone_ref(py),
                        py_args[1].clone_ref(py),
                        py_args[2].clone_ref(py),
                        py_args[3].clone_ref(py),
                        py_args[4].clone_ref(py),
                    )),
                    _ => {
                        let args_tuple = PyTuple::new(
                            py,
                            py_args.iter().map(|arg: &Py<PyAny>| arg.clone_ref(py)),
                        )
                        .map_err(|e| format!("Error creating tuple: {e}"));
                        match args_tuple {
                            Ok(t) => {
                                let code = py
                                    .eval(
                                        std::ffi::CString::new("lambda f, args: f(*args)")
                                            .unwrap()
                                            .as_c_str(),
                                        None,
                                        None,
                                    )
                                    .map_err(|e| format!("Error: {e}"));
                                match code {
                                    Ok(unpack_code) => unpack_code.call1((callback.bind(py), t)),
                                    Err(e) => {
                                        let msg = format!("Error: {e}");
                                        sqlite3_result_error(
                                            udf_ctx,
                                            msg.as_ptr() as *const std::ffi::c_char,
                                            msg.len() as i32,
                                        );
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                sqlite3_result_error(
                                    udf_ctx,
                                    e.as_ptr() as *const std::ffi::c_char,
                                    e.len() as i32,
                                );
                                return;
                            }
                        }
                    }
                };
                match result {
                    Ok(r) => {
                        let _ = py_to_sqlite_c_result(py, udf_ctx, &r);
                    }
                    Err(e) => {
                        let msg = format!("Python function error: {e}");
                        sqlite3_result_error(
                            udf_ctx,
                            msg.as_ptr() as *const std::ffi::c_char,
                            msg.len() as i32,
                        );
                    }
                }
            });
        }
    }
    extern "C" fn udf_destructor(user_data: *mut std::ffi::c_void) {
        if user_data.is_null() {
            return;
        }
        #[allow(deprecated)]
        Python::with_gil(|_py| unsafe {
            let _ = Box::from_raw(user_data as *mut Py<PyAny>);
        });
    }

    let trans_has_conn = {
        let g = ctx.transaction_connection.lock().await;
        g.0.is_some()
    };

    if trans_has_conn {
        let mut trans_guard = ctx.transaction_connection.lock().await;
        let conn = trans_guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
        let sqlite_conn: &mut SqliteConnection = conn;
        let mut handle = sqlite_conn
            .lock_handle()
            .await
            .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
        let raw_db = handle.as_raw_handle().as_ptr();

        if func.is_none() {
            {
                let mut funcs_guard = ctx.user_functions.lock().unwrap();
                funcs_guard.remove(&name);
            }
            let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
                OperationalError::new_err(format!("Function name contains null byte: {e}"))
            })?;
            let result = unsafe {
                sqlite3_create_function_v2(
                    raw_db,
                    name_cstr.as_ptr(),
                    nargs,
                    SQLITE_UTF8,
                    std::ptr::null_mut(),
                    None,
                    None,
                    None,
                    None,
                )
            };
            if result != SQLITE_OK {
                return Err(OperationalError::new_err(format!(
                    "Failed to remove function '{name}': SQLite error code {result}"
                )));
            }
            let all_cleared = !has_callbacks(
                &ctx.load_extension_enabled,
                &ctx.user_functions,
                &ctx.user_aggregates,
                &ctx.user_collations,
                &ctx.trace_callback,
                &ctx.authorizer_callback,
                &ctx.progress_handler,
            );
            if all_cleared {
                drop(handle);
                return Ok(());
            }
        } else {
            let enc = if deterministic {
                SQLITE_UTF8 | SQLITE_DETERMINISTIC
            } else {
                SQLITE_UTF8
            };
            #[allow(deprecated)]
            let callback_for_storage = Python::with_gil(|py| func.as_ref().unwrap().clone_ref(py));
            {
                let mut funcs_guard = ctx.user_functions.lock().unwrap();
                funcs_guard.insert(name.clone(), (nargs, callback_for_storage));
            }
            let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
                OperationalError::new_err(format!("Function name contains null byte: {e}"))
            })?;
            #[allow(deprecated)]
            let callback = Python::with_gil(|py| func.as_ref().unwrap().clone_ref(py));
            let callback_box: Box<Py<PyAny>> = Box::new(callback);
            let callback_ptr = Box::into_raw(callback_box) as *mut std::ffi::c_void;
            let result = unsafe {
                sqlite3_create_function_v2(
                    raw_db,
                    name_cstr.as_ptr(),
                    nargs,
                    enc,
                    callback_ptr,
                    Some(udf_trampoline),
                    None,
                    None,
                    Some(udf_destructor),
                )
            };
            if result != SQLITE_OK {
                unsafe {
                    let _ = Box::from_raw(callback_ptr as *mut Py<PyAny>);
                }
                {
                    let mut funcs_guard = ctx.user_functions.lock().unwrap();
                    funcs_guard.remove(&name);
                }
                return Err(OperationalError::new_err(format!(
                    "Failed to create function '{name}': SQLite error code {result}"
                )));
            }
        }
        return Ok(());
    }

    ensure_callback_connection(
        &ctx.path,
        &ctx.pool,
        &ctx.callback_connection,
        &ctx.pragmas,
        &ctx.pool_size,
        &ctx.connection_timeout_secs,
        &ctx.idle_timeout_secs,
    )
    .await?;

    let mut cb_guard = ctx.callback_connection.lock().await;
    let conn = cb_guard
        .0
        .as_mut()
        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;

    let sqlite_conn: &mut SqliteConnection = conn;
    let mut handle = sqlite_conn
        .lock_handle()
        .await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
    let raw_db = handle.as_raw_handle().as_ptr();

    if func.is_none() {
        {
            let mut funcs_guard = ctx.user_functions.lock().unwrap();
            funcs_guard.remove(&name);
        }
        let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
            OperationalError::new_err(format!("Function name contains null byte: {e}"))
        })?;
        let result = unsafe {
            sqlite3_create_function_v2(
                raw_db,
                name_cstr.as_ptr(),
                nargs,
                SQLITE_UTF8,
                std::ptr::null_mut(),
                None,
                None,
                None,
                None,
            )
        };
        if result != SQLITE_OK {
            return Err(OperationalError::new_err(format!(
                "Failed to remove function '{name}': SQLite error code {result}"
            )));
        }
        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            drop(handle);
            drop(cb_guard);
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
            return Ok(());
        }
    } else {
        let enc = if deterministic {
            SQLITE_UTF8 | SQLITE_DETERMINISTIC
        } else {
            SQLITE_UTF8
        };
        #[allow(deprecated)]
        let callback_for_storage = Python::with_gil(|py| func.as_ref().unwrap().clone_ref(py));
        {
            let mut funcs_guard = ctx.user_functions.lock().unwrap();
            funcs_guard.insert(name.clone(), (nargs, callback_for_storage));
        }
        let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
            OperationalError::new_err(format!("Function name contains null byte: {e}"))
        })?;
        #[allow(deprecated)]
        let callback = Python::with_gil(|py| func.as_ref().unwrap().clone_ref(py));
        let callback_box: Box<Py<PyAny>> = Box::new(callback);
        let callback_ptr = Box::into_raw(callback_box) as *mut std::ffi::c_void;
        let result = unsafe {
            sqlite3_create_function_v2(
                raw_db,
                name_cstr.as_ptr(),
                nargs,
                enc,
                callback_ptr,
                Some(udf_trampoline),
                None,
                None,
                Some(udf_destructor),
            )
        };
        if result != SQLITE_OK {
            unsafe {
                let _ = Box::from_raw(callback_ptr as *mut Py<PyAny>);
            }
            {
                let mut funcs_guard = ctx.user_functions.lock().unwrap();
                funcs_guard.remove(&name);
            }
            return Err(OperationalError::new_err(format!(
                "Failed to create function '{name}': SQLite error code {result}"
            )));
        }
    }

    Ok(())
}

/// Create or remove a custom SQL aggregate function. Runs on the callback connection.
pub(crate) async fn create_aggregate_impl(
    ctx: CallbackContext,
    name: String,
    num_params: i32,
    aggregate_class: Option<Py<PyAny>>,
) -> Result<(), PyErr> {
    ensure_not_closed(&ctx.closed)?;

    ensure_callback_connection(
        &ctx.path,
        &ctx.pool,
        &ctx.callback_connection,
        &ctx.pragmas,
        &ctx.pool_size,
        &ctx.connection_timeout_secs,
        &ctx.idle_timeout_secs,
    )
    .await?;

    let mut conn_guard = ctx.callback_connection.lock().await;
    let conn = conn_guard
        .0
        .as_mut()
        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;

    let sqlite_conn: &mut SqliteConnection = conn;
    let mut handle = sqlite_conn
        .lock_handle()
        .await
        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
    let raw_db = handle.as_raw_handle().as_ptr();

    if aggregate_class.is_none() {
        let old = {
            let mut guard = ctx.user_aggregates.lock().unwrap();
            guard.remove(&name)
        };
        let old_ptr = old.map(|(_, ptr)| ptr).unwrap_or(0);

        let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
            OperationalError::new_err(format!("Aggregate name contains null byte: {e}"))
        })?;
        let result = unsafe {
            sqlite3_create_function_v2(
                raw_db,
                name_cstr.as_ptr(),
                num_params,
                SQLITE_UTF8,
                std::ptr::null_mut(),
                None,
                None,
                None,
                None,
            )
        };
        if result != SQLITE_OK {
            if let Some((old_n, old_ptr_usize)) = old {
                ctx.user_aggregates
                    .lock()
                    .unwrap()
                    .insert(name.clone(), (old_n, old_ptr_usize));
            }
            return Err(OperationalError::new_err(format!(
                "Failed to remove aggregate '{name}': SQLite error code {result}"
            )));
        }
        if old_ptr != 0 {
            #[allow(deprecated)]
            Python::with_gil(|_py| unsafe {
                let _ = Box::from_raw(old_ptr as *mut Py<PyAny>);
            });
        }

        let all_cleared = !has_callbacks(
            &ctx.load_extension_enabled,
            &ctx.user_functions,
            &ctx.user_aggregates,
            &ctx.user_collations,
            &ctx.trace_callback,
            &ctx.authorizer_callback,
            &ctx.progress_handler,
        );
        if all_cleared {
            drop(handle);
            drop(conn_guard);
            let mut callback_guard = ctx.callback_connection.lock().await;
            callback_guard.0.take();
        }
        return Ok(());
    }

    #[allow(deprecated)]
    let class_for_storage = Python::with_gil(|py| aggregate_class.as_ref().unwrap().clone_ref(py));
    let class_box: Box<Py<PyAny>> = Box::new(class_for_storage);
    let class_ptr = Box::into_raw(class_box) as *mut std::ffi::c_void;
    let class_ptr_usize = class_ptr as usize;

    #[repr(C)]
    struct AggState {
        magic: u64,
        instance_ptr: *mut Py<PyAny>,
    }
    const AGG_MAGIC: u64 = 0x_72_61_70_73_71_6C_78_01; // "rapsqlx" + version byte
    const AGG_CTX_SIZE: i32 = std::mem::size_of::<AggState>() as i32;

    extern "C" fn aggregate_step(
        agg_ctx: *mut sqlite3_context,
        argc: std::ffi::c_int,
        argv: *mut *mut sqlite3_value,
    ) {
        unsafe {
            let user_data = sqlite3_user_data(agg_ctx);
            if user_data.is_null() {
                return;
            }
            let class_ptr = user_data as *mut Py<PyAny>;

            let ctx_buf = sqlite3_aggregate_context(agg_ctx, AGG_CTX_SIZE);
            if ctx_buf.is_null() {
                return;
            }
            let state = &mut *(ctx_buf as *mut AggState);
            if state.magic != AGG_MAGIC {
                state.magic = AGG_MAGIC;
                state.instance_ptr = std::ptr::null_mut();
            }

            #[allow(deprecated)]
            Python::with_gil(|py| {
                if state.instance_ptr.is_null() {
                    let class = (*class_ptr).clone_ref(py);
                    let instance = match class.call0(py) {
                        Ok(inst) => inst,
                        Err(e) => {
                            let msg = format!("Python aggregate error: {e}");
                            sqlite3_result_error(
                                agg_ctx,
                                msg.as_ptr() as *const std::ffi::c_char,
                                msg.len() as i32,
                            );
                            return;
                        }
                    };
                    let instance_box: Box<Py<PyAny>> = Box::new(instance);
                    state.instance_ptr = Box::into_raw(instance_box);
                }

                let instance = (*state.instance_ptr).clone_ref(py);
                let mut py_args: Vec<Py<PyAny>> = Vec::new();
                for i in 0..argc {
                    let value_ptr = *argv.add(i as usize);
                    match sqlite_c_value_to_py(py, value_ptr) {
                        Ok(py_val) => py_args.push(py_val),
                        Err(e) => {
                            let msg = format!("Error converting argument {i}: {e}");
                            sqlite3_result_error(
                                agg_ctx,
                                msg.as_ptr() as *const std::ffi::c_char,
                                msg.len() as i32,
                            );
                            return;
                        }
                    }
                }

                let step_result = match py_args.len() {
                    0 => instance.bind(py).call_method0("step"),
                    1 => instance
                        .bind(py)
                        .call_method1("step", (py_args[0].clone_ref(py),)),
                    2 => instance
                        .bind(py)
                        .call_method1("step", (py_args[0].clone_ref(py), py_args[1].clone_ref(py))),
                    3 => instance.bind(py).call_method1(
                        "step",
                        (
                            py_args[0].clone_ref(py),
                            py_args[1].clone_ref(py),
                            py_args[2].clone_ref(py),
                        ),
                    ),
                    4 => instance.bind(py).call_method1(
                        "step",
                        (
                            py_args[0].clone_ref(py),
                            py_args[1].clone_ref(py),
                            py_args[2].clone_ref(py),
                            py_args[3].clone_ref(py),
                        ),
                    ),
                    5 => instance.bind(py).call_method1(
                        "step",
                        (
                            py_args[0].clone_ref(py),
                            py_args[1].clone_ref(py),
                            py_args[2].clone_ref(py),
                            py_args[3].clone_ref(py),
                            py_args[4].clone_ref(py),
                        ),
                    ),
                    _ => {
                        let args_tuple = PyTuple::new(py, py_args.iter().map(|a| a.clone_ref(py)));
                        match args_tuple {
                            Ok(t) => instance.bind(py).call_method1("step", (t,)),
                            Err(e) => {
                                let msg = format!("Error building step args: {e}");
                                sqlite3_result_error(
                                    agg_ctx,
                                    msg.as_ptr() as *const std::ffi::c_char,
                                    msg.len() as i32,
                                );
                                return;
                            }
                        }
                    }
                };

                if let Err(e) = step_result {
                    let msg = format!("Python aggregate step error: {e}");
                    sqlite3_result_error(
                        agg_ctx,
                        msg.as_ptr() as *const std::ffi::c_char,
                        msg.len() as i32,
                    );
                }
            });
        }
    }

    extern "C" fn aggregate_final(agg_ctx: *mut sqlite3_context) {
        unsafe {
            let ctx_buf = sqlite3_aggregate_context(agg_ctx, AGG_CTX_SIZE);
            if ctx_buf.is_null() {
                sqlite3_result_null(agg_ctx);
                return;
            }
            let state = &mut *(ctx_buf as *mut AggState);
            if state.magic != AGG_MAGIC || state.instance_ptr.is_null() {
                sqlite3_result_null(agg_ctx);
                return;
            }

            #[allow(deprecated)]
            Python::with_gil(|py| {
                let instance = Box::from_raw(state.instance_ptr);
                state.instance_ptr = std::ptr::null_mut();

                let result = instance.bind(py).call_method0("finalize");
                match result {
                    Ok(r) => {
                        let _ = py_to_sqlite_c_result(py, agg_ctx, &r);
                    }
                    Err(e) => {
                        let msg = format!("Python aggregate finalize error: {e}");
                        sqlite3_result_error(
                            agg_ctx,
                            msg.as_ptr() as *const std::ffi::c_char,
                            msg.len() as i32,
                        );
                    }
                }
            });
        }
    }

    let name_cstr = std::ffi::CString::new(name.clone()).map_err(|e| {
        OperationalError::new_err(format!("Aggregate name contains null byte: {e}"))
    })?;

    let result = unsafe {
        sqlite3_create_function_v2(
            raw_db,
            name_cstr.as_ptr(),
            num_params,
            SQLITE_UTF8,
            class_ptr,
            None,
            Some(aggregate_step),
            Some(aggregate_final),
            None,
        )
    };

    if result != SQLITE_OK {
        unsafe {
            let _ = Box::from_raw(class_ptr as *mut Py<PyAny>);
        }
        return Err(OperationalError::new_err(format!(
            "Failed to create aggregate '{name}': SQLite error code {result}"
        )));
    }

    let old = {
        let mut guard = ctx.user_aggregates.lock().unwrap();
        guard.insert(name.clone(), (num_params, class_ptr_usize))
    };
    if let Some((_, old_ptr)) = old {
        #[allow(deprecated)]
        Python::with_gil(|_py| unsafe {
            let _ = Box::from_raw(old_ptr as *mut Py<PyAny>);
        });
    }

    Ok(())
}
