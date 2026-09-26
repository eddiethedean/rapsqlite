//! Synchronous batch execution via raw SQLite C API.
//! Used by execute_many (pool path) to run the insert loop without per-row await.

use libsqlite3_sys::{
    sqlite3, sqlite3_bind_blob, sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null,
    sqlite3_bind_parameter_count, sqlite3_bind_text, sqlite3_changes, sqlite3_clear_bindings,
    sqlite3_column_blob, sqlite3_column_bytes, sqlite3_column_count, sqlite3_column_double,
    sqlite3_column_int64, sqlite3_column_text, sqlite3_column_type, sqlite3_errmsg,
    sqlite3_finalize, sqlite3_last_insert_rowid, sqlite3_prepare_v2, sqlite3_reset, sqlite3_step,
    sqlite3_total_changes, SQLITE_BLOB, SQLITE_DONE, SQLITE_ERROR, SQLITE_FLOAT, SQLITE_INTEGER,
    SQLITE_MISMATCH, SQLITE_MISUSE, SQLITE_NULL, SQLITE_OK, SQLITE_RANGE, SQLITE_ROW,
    SQLITE_STATIC, SQLITE_TEXT, SQLITE_TOOBIG,
};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::pool::RawStatementCache;
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

/// Prepare exactly one SQL statement, rejecting any executable SQL left in
/// SQLite's tail pointer. sqlite3_prepare_v2() otherwise silently ignores
/// everything after the first statement when its tail argument is omitted.
fn prepare_single_statement(
    db: *mut sqlite3,
    query: &CString,
) -> Result<*mut libsqlite3_sys::sqlite3_stmt, (i32, String)> {
    let mut stmt = std::ptr::null_mut();
    let mut tail: *const c_char = std::ptr::null();
    let rc = unsafe { sqlite3_prepare_v2(db, query.as_ptr(), -1_i32, &mut stmt, &mut tail) };
    if rc != SQLITE_OK {
        return Err((rc, errmsg_from_db(db)));
    }
    if stmt.is_null() {
        return Err((
            SQLITE_ERROR,
            "sqlite3_prepare_v2 returned null statement".to_string(),
        ));
    }

    let mut remaining = tail;
    while !remaining.is_null() && unsafe { *remaining != 0 } {
        let mut extra_stmt = std::ptr::null_mut();
        let mut next_tail: *const c_char = std::ptr::null();
        let rc =
            unsafe { sqlite3_prepare_v2(db, remaining, -1_i32, &mut extra_stmt, &mut next_tail) };
        if rc != SQLITE_OK {
            let error = (rc, errmsg_from_db(db));
            let _ = unsafe { sqlite3_finalize(stmt) };
            return Err(error);
        }
        if !extra_stmt.is_null() {
            let _ = unsafe { sqlite3_finalize(extra_stmt) };
            let _ = unsafe { sqlite3_finalize(stmt) };
            return Err((
                SQLITE_ERROR,
                "only one SQL statement is allowed".to_string(),
            ));
        }
        if next_tail.is_null() || next_tail == remaining {
            let _ = unsafe { sqlite3_finalize(stmt) };
            return Err((SQLITE_ERROR, "unable to parse trailing SQL".to_string()));
        }
        remaining = next_tail;
    }

    Ok(stmt)
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

/// Bind a parameter without narrowing an arbitrary Rust buffer length at the
/// SQLite C boundary. SQLite's text/blob APIs take a signed `c_int` length;
/// wrapping a larger `usize` can turn it negative and make SQLite read beyond
/// the Rust-owned buffer.
fn bind_sqlite_param(
    db: *mut sqlite3,
    stmt: *mut libsqlite3_sys::sqlite3_stmt,
    index: c_int,
    param: &SqliteParam,
) -> Result<(), (i32, String)> {
    let rc = match param {
        SqliteParam::Null => unsafe { sqlite3_bind_null(stmt, index) },
        SqliteParam::Int(value) => unsafe { sqlite3_bind_int64(stmt, index, *value) },
        SqliteParam::Real(value) => unsafe { sqlite3_bind_double(stmt, index, *value) },
        SqliteParam::Text(value) => {
            let bytes = value.as_bytes();
            let length = checked_bind_length(bytes.len())?;
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
            unsafe { sqlite3_bind_text(stmt, index, ptr, length, SQLITE_STATIC()) }
        }
        SqliteParam::Blob(value) => {
            let length = checked_bind_length(value.len())?;
            unsafe {
                sqlite3_bind_blob(
                    stmt,
                    index,
                    value.as_ptr() as *const std::ffi::c_void,
                    length,
                    SQLITE_STATIC(),
                )
            }
        }
    };
    if rc == SQLITE_OK {
        Ok(())
    } else {
        Err((rc, errmsg_from_db(db)))
    }
}

fn checked_bind_length(length: usize) -> Result<c_int, (i32, String)> {
    c_int::try_from(length).map_err(|_| {
        (
            SQLITE_TOOBIG,
            "text or BLOB parameter exceeds SQLite's supported bind length".to_string(),
        )
    })
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

    let query_c = CString::new(query).map_err(|e| {
        (
            libsqlite3_sys::SQLITE_ERROR,
            format!("Invalid query string: {e}"),
        )
    })?;

    exec_simple(db, "BEGIN")?;

    let stmt = match prepare_single_statement(db, &query_c) {
        Ok(stmt) => stmt,
        Err(error) => {
            let _ = exec_simple(db, "ROLLBACK");
            return Err(error);
        }
    };

    let expected_params = unsafe { sqlite3_bind_parameter_count(stmt) } as usize;
    if let Some(param_set) = params
        .iter()
        .find(|param_set| param_set.len() != expected_params)
    {
        let provided_params = param_set.len();
        let _ = unsafe { sqlite3_finalize(stmt) };
        let _ = exec_simple(db, "ROLLBACK");
        return Err((
            SQLITE_RANGE,
            format!(
                "Incorrect number of bindings supplied: statement uses {expected_params}, but {provided_params} were supplied"
            ),
        ));
    }

    let mut total_changes = 0u64;
    for param_set in params.iter() {
        let rc_reset = unsafe { sqlite3_reset(stmt) };
        if rc_reset != SQLITE_OK {
            let _ = unsafe { sqlite3_finalize(stmt) };
            let _ = exec_simple(db, "ROLLBACK");
            return Err((rc_reset, errmsg_from_db(db)));
        }

        for (i, p) in param_set.iter().enumerate() {
            let idx = (i + 1) as c_int;
            if let Err(error) = bind_sqlite_param(db, stmt, idx, p) {
                let _ = unsafe { sqlite3_finalize(stmt) };
                let _ = exec_simple(db, "ROLLBACK");
                return Err(error);
            }
        }

        let changes_before = unsafe { sqlite3_total_changes(db) };
        loop {
            let rc_step = unsafe { sqlite3_step(stmt) };
            match rc_step {
                SQLITE_ROW => continue,
                SQLITE_DONE => {
                    // sqlite3_changes() retains its previous value after non-DML
                    // statements. The total-change counter tells us whether this
                    // execution changed the database before reading its row count.
                    if unsafe { sqlite3_total_changes(db) } != changes_before {
                        total_changes += unsafe { sqlite3_changes(db) } as u64;
                    }
                    break;
                }
                _ => {
                    let _ = unsafe { sqlite3_finalize(stmt) };
                    let _ = exec_simple(db, "ROLLBACK");
                    return Err((rc_step, errmsg_from_db(db)));
                }
            }
        }
    }

    let last_rowid = unsafe { sqlite3_last_insert_rowid(db) };

    let rc_fin = unsafe { sqlite3_finalize(stmt) };
    if rc_fin != SQLITE_OK {
        let _ = exec_simple(db, "ROLLBACK");
        return Err((rc_fin, errmsg_from_db(db)));
    }

    if let Err(error) = exec_simple(db, "COMMIT") {
        // SQLite leaves the transaction active for some COMMIT failures (for
        // example, deferred constraint violations). Do not return that handle
        // to the pool in a partially committed state.
        let _ = exec_simple(db, "ROLLBACK");
        return Err(error);
    }
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
    statement_cache: Option<&RawStatementCache>,
) -> Result<Option<RawScalar>, (i32, String)> {
    if db.is_null() {
        return Err((SQLITE_MISUSE, "db pointer is null".to_string()));
    }
    let cached_stmt = statement_cache.and_then(|cache| cache.get(query));
    let cached = cached_stmt.is_some();
    let stmt = if let Some(statement) = cached_stmt {
        let stmt = statement as *mut libsqlite3_sys::sqlite3_stmt;
        unsafe {
            sqlite3_reset(stmt);
            sqlite3_clear_bindings(stmt);
        }
        stmt
    } else {
        let query_c = CString::new(query).map_err(|e| (SQLITE_ERROR, e.to_string()))?;
        prepare_single_statement(db, &query_c)?
    };

    let reset_or_finalize = |stmt: *mut libsqlite3_sys::sqlite3_stmt| {
        if cached {
            unsafe {
                sqlite3_reset(stmt);
                sqlite3_clear_bindings(stmt);
            }
        } else {
            unsafe {
                sqlite3_finalize(stmt);
            }
        }
    };

    let expected_params = unsafe { sqlite3_bind_parameter_count(stmt) } as usize;
    if params.len() != expected_params {
        reset_or_finalize(stmt);
        return Err((
            SQLITE_RANGE,
            format!(
                "Incorrect number of bindings supplied: statement uses {expected_params}, but {} were supplied",
                params.len()
            ),
        ));
    }

    let columns = unsafe { sqlite3_column_count(stmt) };
    if columns != 1 {
        reset_or_finalize(stmt);
        return Err((
            SQLITE_ERROR,
            format!("raw scalar queries must return exactly one column; got {columns}"),
        ));
    }

    let retained = cached
        || statement_cache.is_some_and(|cache| cache.insert(query.to_string(), stmt as usize));

    let result = (|| {
        for (index, param) in params.iter().enumerate() {
            let bind_index = (index + 1) as c_int;
            bind_sqlite_param(db, stmt, bind_index, param)?;
        }

        match unsafe { sqlite3_step(stmt) } {
            SQLITE_DONE => Ok(None),
            SQLITE_ROW => {
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

    if retained {
        unsafe {
            sqlite3_reset(stmt);
            sqlite3_clear_bindings(stmt);
        }
    } else {
        let finalize_rc = unsafe { sqlite3_finalize(stmt) };
        if finalize_rc != SQLITE_OK && result.is_ok() {
            return Err((finalize_rc, errmsg_from_db(db)));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{checked_bind_length, SQLITE_TOOBIG};
    use std::os::raw::c_int;

    #[test]
    fn raw_bind_lengths_must_fit_sqlite_c_int() {
        assert_eq!(checked_bind_length(c_int::MAX as usize), Ok(c_int::MAX));
        let error = checked_bind_length(usize::MAX).unwrap_err();
        assert_eq!(error.0, SQLITE_TOOBIG);
    }
}
