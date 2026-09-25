//! Synchronous batch execution via raw SQLite C API.
//! Used by execute_many (pool path) to run the insert loop without per-row await.

use libsqlite3_sys::{
    sqlite3, sqlite3_bind_blob, sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null,
    sqlite3_bind_text, sqlite3_column_blob, sqlite3_column_bytes, sqlite3_column_count,
    sqlite3_column_double, sqlite3_column_int64, sqlite3_column_text, sqlite3_column_type,
    sqlite3_errmsg, sqlite3_finalize, sqlite3_last_insert_rowid, sqlite3_prepare_v2, sqlite3_reset,
    sqlite3_step, SQLITE_BLOB, SQLITE_DONE, SQLITE_ERROR, SQLITE_FLOAT, SQLITE_INTEGER,
    SQLITE_MISMATCH, SQLITE_MISUSE, SQLITE_NULL, SQLITE_OK, SQLITE_ROW, SQLITE_STATIC, SQLITE_TEXT,
};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::types::SqliteParam;

/// A scalar returned by the narrow raw SQLite path. It is converted to Python
/// only after the handle lock and blocking SQLite call are complete.
pub(crate) enum RawScalar {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

fn errmsg_from_db(db: *mut sqlite3) -> String {
    if db.is_null() {
        return "SQLite error".to_string();
    }
    let c_msg = unsafe { sqlite3_errmsg(db) };
    if c_msg.is_null() {
        "SQLite error".to_string()
    } else {
        unsafe { CStr::from_ptr(c_msg).to_string_lossy().into_owned() }
    }
}

/// Run a single SQL statement (no parameters). Used for BEGIN/COMMIT.
fn exec_simple(db: *mut sqlite3, sql: &str) -> Result<(), (i32, String)> {
    let c = CString::new(sql)
        .map_err(|e| (libsqlite3_sys::SQLITE_ERROR, format!("Invalid SQL: {e}")))?;
    let mut stmt = std::ptr::null_mut();
    let rc = unsafe { sqlite3_prepare_v2(db, c.as_ptr(), -1_i32, &mut stmt, std::ptr::null_mut()) };
    if rc != SQLITE_OK {
        return Err((rc, errmsg_from_db(db)));
    }
    if stmt.is_null() {
        return Err((rc, "sqlite3_prepare_v2 returned null statement".to_string()));
    }
    loop {
        let step_rc = unsafe { sqlite3_step(stmt) };
        match step_rc {
            SQLITE_ROW => continue,
            SQLITE_DONE => break,
            _ => {
                let _ = unsafe { sqlite3_finalize(stmt) };
                return Err((step_rc, errmsg_from_db(db)));
            }
        }
    }
    let _ = unsafe { sqlite3_finalize(stmt) };
    Ok(())
}

/// Core batch loop: BEGIN, prepare, bind/step/reset per row, finalize, COMMIT.
/// Single transaction for the whole batch (matches aiosqlite / sqlite3.executemany).
/// Returns (total_changes, last_insert_rowid) or (rc, error_message). No PyErr (Send-safe).
pub(crate) fn execute_many_raw_core(
    db: *mut sqlite3,
    query: &str,
    params: &[Vec<SqliteParam>],
) -> Result<(u64, i64), (i32, String)> {
    if params.is_empty() {
        return Ok((0, 0));
    }
    if db.is_null() {
        return Err((
            libsqlite3_sys::SQLITE_MISUSE,
            "db pointer is null".to_string(),
        ));
    }

    exec_simple(db, "BEGIN")?;

    let query_c = CString::new(query).map_err(|e| {
        (
            libsqlite3_sys::SQLITE_ERROR,
            format!("Invalid query string: {e}"),
        )
    })?;

    let mut stmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            query_c.as_ptr(),
            -1_i32,
            &mut stmt,
            std::ptr::null_mut(),
        )
    };
    if rc != SQLITE_OK {
        let _ = exec_simple(db, "ROLLBACK");
        return Err((rc, errmsg_from_db(db)));
    }
    if stmt.is_null() {
        let _ = exec_simple(db, "ROLLBACK");
        return Err((rc, "sqlite3_prepare_v2 returned null statement".to_string()));
    }

    for param_set in params.iter() {
        let rc_reset = unsafe { sqlite3_reset(stmt) };
        if rc_reset != SQLITE_OK {
            let _ = unsafe { sqlite3_finalize(stmt) };
            let _ = exec_simple(db, "ROLLBACK");
            return Err((rc_reset, errmsg_from_db(db)));
        }

        for (i, p) in param_set.iter().enumerate() {
            let idx = (i + 1) as c_int;
            let rc_bind = match p {
                SqliteParam::Null => unsafe { sqlite3_bind_null(stmt, idx) },
                SqliteParam::Int(v) => unsafe { sqlite3_bind_int64(stmt, idx, *v) },
                SqliteParam::Real(v) => unsafe { sqlite3_bind_double(stmt, idx, *v) },
                SqliteParam::Text(s) => {
                    let bytes = s.as_bytes();
                    // SQLITE_STATIC: buffer valid until sqlite3_step() returns; no copy.
                    // libsqlite3-sys bindings: Linux aarch64 (manylinux) expects *const u8; others *const c_char (i8).
                    let text_ptr = bytes.as_ptr();
                    let ptr = {
                        #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
                        {
                            text_ptr
                        }
                        #[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
                        {
                            text_ptr as *const c_char
                        }
                    };
                    unsafe {
                        sqlite3_bind_text(stmt, idx, ptr, bytes.len() as c_int, SQLITE_STATIC())
                    }
                }
                SqliteParam::Blob(b) => unsafe {
                    // SQLITE_STATIC: buffer valid until sqlite3_step() returns; no copy.
                    sqlite3_bind_blob(
                        stmt,
                        idx,
                        b.as_ptr() as *const std::ffi::c_void,
                        b.len() as c_int,
                        SQLITE_STATIC(),
                    )
                },
            };
            if rc_bind != SQLITE_OK {
                let _ = unsafe { sqlite3_finalize(stmt) };
                let _ = exec_simple(db, "ROLLBACK");
                return Err((rc_bind, errmsg_from_db(db)));
            }
        }

        loop {
            let rc_step = unsafe { sqlite3_step(stmt) };
            match rc_step {
                SQLITE_ROW => continue,
                SQLITE_DONE => break,
                _ => {
                    let _ = unsafe { sqlite3_finalize(stmt) };
                    let _ = exec_simple(db, "ROLLBACK");
                    return Err((rc_step, errmsg_from_db(db)));
                }
            }
        }
    }

    let total_changes = params.len() as u64;
    let last_rowid = unsafe { sqlite3_last_insert_rowid(db) };

    let rc_fin = unsafe { sqlite3_finalize(stmt) };
    if rc_fin != SQLITE_OK {
        let _ = exec_simple(db, "ROLLBACK");
        return Err((rc_fin, errmsg_from_db(db)));
    }

    exec_simple(db, "COMMIT")?;
    Ok((total_changes, last_rowid))
}

/// Execute one prepared statement and return zero or one scalar column.
///
/// This deliberately has a narrow contract: exactly one result column is
/// required, and only the first row is returned. The caller owns the SQLite
/// handle lock and should run this in a blocking Tokio section because SQLite's
/// C API is synchronous.
pub(crate) fn fetch_scalar_raw_core(
    db: *mut sqlite3,
    query: &str,
    params: &[SqliteParam],
    blob_only: bool,
) -> Result<Option<RawScalar>, (i32, String)> {
    if db.is_null() {
        return Err((SQLITE_MISUSE, "db pointer is null".to_string()));
    }
    let query_c = CString::new(query).map_err(|e| (SQLITE_ERROR, e.to_string()))?;
    let mut stmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            query_c.as_ptr(),
            -1_i32,
            &mut stmt,
            std::ptr::null_mut(),
        )
    };
    if rc != SQLITE_OK {
        return Err((rc, errmsg_from_db(db)));
    }
    if stmt.is_null() {
        return Err((
            SQLITE_ERROR,
            "sqlite3_prepare_v2 returned null statement".to_string(),
        ));
    }

    let result = (|| {
        for (index, param) in params.iter().enumerate() {
            let bind_index = (index + 1) as c_int;
            let rc_bind = match param {
                SqliteParam::Null => unsafe { sqlite3_bind_null(stmt, bind_index) },
                SqliteParam::Int(value) => unsafe { sqlite3_bind_int64(stmt, bind_index, *value) },
                SqliteParam::Real(value) => unsafe {
                    sqlite3_bind_double(stmt, bind_index, *value)
                },
                SqliteParam::Text(value) => {
                    let bytes = value.as_bytes();
                    let text_ptr = bytes.as_ptr();
                    let ptr = {
                        #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
                        {
                            text_ptr
                        }
                        #[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
                        {
                            text_ptr as *const c_char
                        }
                    };
                    unsafe {
                        sqlite3_bind_text(
                            stmt,
                            bind_index,
                            ptr,
                            bytes.len() as c_int,
                            SQLITE_STATIC(),
                        )
                    }
                }
                SqliteParam::Blob(value) => unsafe {
                    sqlite3_bind_blob(
                        stmt,
                        bind_index,
                        value.as_ptr() as *const std::ffi::c_void,
                        value.len() as c_int,
                        SQLITE_STATIC(),
                    )
                },
            };
            if rc_bind != SQLITE_OK {
                return Err((rc_bind, errmsg_from_db(db)));
            }
        }

        match unsafe { sqlite3_step(stmt) } {
            SQLITE_DONE => Ok(None),
            SQLITE_ROW => {
                let columns = unsafe { sqlite3_column_count(stmt) };
                if columns != 1 {
                    return Err((
                        SQLITE_ERROR,
                        format!("raw scalar queries must return exactly one column; got {columns}"),
                    ));
                }
                let column_type = unsafe { sqlite3_column_type(stmt, 0) };
                let value = match column_type {
                    SQLITE_NULL => RawScalar::Null,
                    SQLITE_INTEGER => RawScalar::Integer(unsafe { sqlite3_column_int64(stmt, 0) }),
                    SQLITE_FLOAT => RawScalar::Real(unsafe { sqlite3_column_double(stmt, 0) }),
                    SQLITE_TEXT => {
                        let ptr = unsafe { sqlite3_column_text(stmt, 0) };
                        let len = unsafe { sqlite3_column_bytes(stmt, 0) } as usize;
                        if ptr.is_null() {
                            RawScalar::Null
                        } else {
                            let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
                            let text = String::from_utf8(bytes.to_vec()).map_err(|error| {
                                (
                                    SQLITE_ERROR,
                                    format!("invalid UTF-8 in SQLite text value: {error}"),
                                )
                            })?;
                            RawScalar::Text(text)
                        }
                    }
                    SQLITE_BLOB => {
                        let ptr = unsafe { sqlite3_column_blob(stmt, 0) };
                        let len = unsafe { sqlite3_column_bytes(stmt, 0) } as usize;
                        let bytes = if ptr.is_null() {
                            Vec::new()
                        } else {
                            unsafe { std::slice::from_raw_parts(ptr as *const u8, len).to_vec() }
                        };
                        RawScalar::Blob(bytes)
                    }
                    _ => RawScalar::Null,
                };
                if blob_only && !matches!(value, RawScalar::Null | RawScalar::Blob(_)) {
                    return Err((
                        SQLITE_MISMATCH,
                        "raw blob queries must return a BLOB or NULL".to_string(),
                    ));
                }
                Ok(Some(value))
            }
            rc => Err((rc, errmsg_from_db(db))),
        }
    })();

    let finalize_rc = unsafe { sqlite3_finalize(stmt) };
    if finalize_rc != SQLITE_OK && result.is_ok() {
        return Err((finalize_rc, errmsg_from_db(db)));
    }
    result
}
