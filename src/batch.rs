//! Synchronous batch execution via raw SQLite C API.
//! Used by execute_many (pool path) to run the insert loop without per-row await.

use libsqlite3_sys::{
    sqlite3, sqlite3_bind_blob, sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null,
    sqlite3_bind_text, sqlite3_busy_timeout, sqlite3_close, sqlite3_errmsg,
    sqlite3_finalize, sqlite3_last_insert_rowid, sqlite3_open, sqlite3_prepare_v2, sqlite3_reset,
    sqlite3_step, SQLITE_DONE, SQLITE_OK, SQLITE_ROW, SQLITE_STATIC,
};
use std::ffi::{CStr, CString};
use std::os::raw::c_int;

use crate::errors::map_sqlite_error_from_msg;
use crate::types::SqliteParam;

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
    let c = CString::new(sql).map_err(|e| {
        (
            libsqlite3_sys::SQLITE_ERROR,
            format!("Invalid SQL: {e}"),
        )
    })?;
    let mut stmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            c.as_ptr(),
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
            rc,
            "sqlite3_prepare_v2 returned null statement".to_string(),
        ));
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
        return Err((libsqlite3_sys::SQLITE_MISUSE, "db pointer is null".to_string()));
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
        return Err((
            rc,
            "sqlite3_prepare_v2 returned null statement".to_string(),
        ));
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
                    unsafe {
                        sqlite3_bind_text(
                            stmt,
                            idx,
                            bytes.as_ptr() as *const i8,
                            bytes.len() as c_int,
                            SQLITE_STATIC(),
                        )
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

/// Execute batch on an existing raw sqlite3* (caller holds lock). Returns PyErr for Python path.
pub(crate) fn execute_many_raw(
    db: *mut sqlite3,
    query: &str,
    params: &[Vec<SqliteParam>],
    path: &str,
) -> Result<(u64, i64), pyo3::PyErr> {
    execute_many_raw_core(db, query, params)
        .map_err(|(rc, msg)| map_sqlite_error_from_msg(path, query, rc, &msg))
}

/// Open a fresh connection, run the batch, close. Runs on a blocking thread (Send + no PyErr).
/// Returns (total_changes, last_insert_rowid) or error message string.
pub(crate) fn execute_many_raw_standalone(
    path: &str,
    query: &str,
    params: &[Vec<SqliteParam>],
    timeout_ms: i64,
) -> Result<(u64, i64), String> {
    if params.is_empty() {
        return Ok((0, 0));
    }
    let path_c = CString::new(path).map_err(|e| format!("Invalid path: {e}"))?;
    let mut db = std::ptr::null_mut();
    let rc = unsafe { sqlite3_open(path_c.as_ptr(), &mut db) };
    if rc != SQLITE_OK {
        let msg = if db.is_null() {
            format!("sqlite3_open failed: {rc}")
        } else {
            let m = errmsg_from_db(db);
            let _ = unsafe { sqlite3_close(db) };
            m
        };
        return Err(msg);
    }
    if db.is_null() {
        return Err("sqlite3_open returned null".to_string());
    }
    unsafe { sqlite3_busy_timeout(db, timeout_ms as i32) };
    let result = execute_many_raw_core(db, query, params);
    let close_rc = unsafe { sqlite3_close(db) };
    match result {
        Ok(x) => {
            if close_rc != SQLITE_OK {
                // Best-effort: we succeeded but close failed
            }
            Ok(x)
        }
        Err((_rc, msg)) => Err(msg),
    }
}
