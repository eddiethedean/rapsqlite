#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::{Column, Row, SqlitePool};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnection, SqlitePoolOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;

// libsqlite3-sys for raw SQLite C API access
use libsqlite3_sys::{
    sqlite3, sqlite3_context, sqlite3_create_function_v2, sqlite3_enable_load_extension,
    sqlite3_progress_handler, sqlite3_result_null, sqlite3_set_authorizer, sqlite3_trace_v2,
    sqlite3_user_data, sqlite3_value, SQLITE_DENY, SQLITE_IGNORE, SQLITE_OK, SQLITE_TRACE_STMT,
    SQLITE_UTF8,
};

// Exception classes matching aiosqlite API (ABI3 compatible)
create_exception!(_rapsqlite, Error, PyException);
create_exception!(_rapsqlite, Warning, PyException);
create_exception!(_rapsqlite, DatabaseError, PyException);
create_exception!(_rapsqlite, OperationalError, PyException);
create_exception!(_rapsqlite, ProgrammingError, PyException);
create_exception!(_rapsqlite, IntegrityError, PyException);

/// Validate a file path for security and correctness.
fn validate_path(path: &str) -> PyResult<()> {
    if path.is_empty() {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Database path cannot be empty",
        ));
    }
    if path.contains('\0') {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Database path cannot contain null bytes",
        ));
    }
    Ok(())
}

/// Parse SQLite connection string (URI format: file:path?param=value&param2=value2).
/// Returns (database_path, vec of (param_name, param_value)).
fn parse_connection_string(uri: &str) -> PyResult<(String, Vec<(String, String)>)> {
    // Handle :memory: special case
    if uri == ":memory:" {
        return Ok((":memory:".to_string(), Vec::new()));
    }
    
    // Check if it's a URI (starts with file:)
    if uri.starts_with("file:") {
        // Parse URI: file:path?param=value&param2=value2
        let uri_part = &uri[5..]; // Skip "file:"
        let (path_part, query_part) = if let Some(pos) = uri_part.find('?') {
            (uri_part[..pos].to_string(), Some(&uri_part[pos + 1..]))
        } else {
            (uri_part.to_string(), None)
        };
        
        let mut params = Vec::new();
        if let Some(query) = query_part {
            for param_pair in query.split('&') {
                if let Some(equal_pos) = param_pair.find('=') {
                    let key = param_pair[..equal_pos].to_string();
                    let value = param_pair[equal_pos + 1..].to_string();
                    params.push((key, value));
                }
            }
        }
        
        // Decode URI-encoded path (basic support)
        let decoded_path = if path_part.starts_with("///") {
            // Absolute path: file:///path/to/db
            path_part[2..].to_string()
        } else if path_part.starts_with("//") {
            // Network path: file://host/path (not commonly used for SQLite)
            path_part.to_string()
        } else {
            // Relative path: file:db.sqlite
            path_part
        };
        
        Ok((decoded_path, params))
    } else {
        // Regular file path
        Ok((uri.to_string(), Vec::new()))
    }
}

/// Convert a SQLite C API value (sqlite3_value*) to Python object.
/// This is used in callback trampolines for user-defined functions.
unsafe fn sqlite_c_value_to_py<'py>(
    py: Python<'py>,
    value: *mut sqlite3_value,
) -> PyResult<Py<PyAny>> {
    use libsqlite3_sys::{sqlite3_value_blob, sqlite3_value_bytes, sqlite3_value_double, sqlite3_value_int64, sqlite3_value_text, sqlite3_value_type, SQLITE_BLOB, SQLITE_FLOAT, SQLITE_INTEGER, SQLITE_NULL, SQLITE_TEXT};
    
    let value_type = sqlite3_value_type(value);
    match value_type {
        SQLITE_NULL => Ok(py.None()),
        SQLITE_INTEGER => {
            let int_val = sqlite3_value_int64(value);
            Ok(PyInt::new(py, int_val).into())
        }
        SQLITE_FLOAT => {
            let float_val = sqlite3_value_double(value);
            Ok(PyFloat::new(py, float_val).into())
        }
        SQLITE_TEXT => {
            let text_ptr = sqlite3_value_text(value);
            let text_len = sqlite3_value_bytes(value) as usize;
            if text_ptr.is_null() {
                Ok(py.None())
            } else {
                let text_slice = std::slice::from_raw_parts(text_ptr as *const u8, text_len);
                let text_str = std::str::from_utf8(text_slice)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        format!("Invalid UTF-8 in SQLite text value: {}", e)
                    ))?;
                Ok(PyString::new(py, text_str).into())
            }
        }
        SQLITE_BLOB => {
            let blob_ptr = sqlite3_value_blob(value);
            let blob_len = sqlite3_value_bytes(value) as usize;
            if blob_ptr.is_null() {
                Ok(py.None())
            } else {
                let blob_slice = std::slice::from_raw_parts(blob_ptr as *const u8, blob_len);
                Ok(PyBytes::new(py, blob_slice).into())
            }
        }
        _ => Ok(py.None()), // Unknown type, treat as NULL
    }
}

/// Convert a Python object to SQLite C API value and set it in the context.
/// This is used to return values from user-defined functions.
unsafe fn py_to_sqlite_c_result(
    py: Python<'_>,
    ctx: *mut sqlite3_context,
    result: &Bound<'_, PyAny>,
) -> PyResult<()> {
    use libsqlite3_sys::{sqlite3_result_blob, sqlite3_result_double, sqlite3_result_int64, sqlite3_result_null, sqlite3_result_text};
    
    if result.is_none() {
        sqlite3_result_null(ctx);
        return Ok(());
    }
    
    // Try to extract as integer
    if let Ok(int_val) = result.extract::<i64>() {
        sqlite3_result_int64(ctx, int_val);
        return Ok(());
    }
    
    // Try to extract as float
    if let Ok(float_val) = result.extract::<f64>() {
        sqlite3_result_double(ctx, float_val);
        return Ok(());
    }
    
    // Try to extract as string
    if let Ok(str_val) = result.extract::<String>() {
        let c_str = std::ffi::CString::new(str_val)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("String contains null byte: {}", e)
            ))?;
        let ptr = c_str.as_ptr();
        let len = c_str.as_bytes().len() as i32;
        // SQLite will copy the string, so we need to ensure it's valid
        // Use SQLITE_TRANSIENT to let SQLite manage the memory
        sqlite3_result_text(ctx, ptr, len, libsqlite3_sys::SQLITE_TRANSIENT());
        // Keep c_str alive until after the call
        std::mem::forget(c_str);
        return Ok(());
    }
    
    // Try to extract as bytes
    if let Ok(bytes_val) = result.extract::<Vec<u8>>() {
        let len = bytes_val.len() as i32;
        let ptr = bytes_val.as_ptr();
        sqlite3_result_blob(ctx, ptr as *const std::ffi::c_void, len, libsqlite3_sys::SQLITE_TRANSIENT());
        // Keep bytes_val alive
        std::mem::forget(bytes_val);
        return Ok(());
    }
    
    // Try PyBytes
    if let Ok(py_bytes) = result.downcast::<PyBytes>() {
        let bytes = py_bytes.as_bytes();
        let len = bytes.len() as i32;
        let ptr = bytes.as_ptr();
        sqlite3_result_blob(ctx, ptr as *const std::ffi::c_void, len, libsqlite3_sys::SQLITE_TRANSIENT());
        return Ok(());
    }
    
    // Try PyString
    if let Ok(py_str) = result.downcast::<PyString>() {
        let str_val = py_str.to_str()?;
        let c_str = std::ffi::CString::new(str_val)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("String contains null byte: {}", e)
            ))?;
        let ptr = c_str.as_ptr();
        let len = c_str.as_bytes().len() as i32;
        sqlite3_result_text(ctx, ptr, len, libsqlite3_sys::SQLITE_TRANSIENT());
        std::mem::forget(c_str);
        return Ok(());
    }
    
    // Default: return NULL
    sqlite3_result_null(ctx);
    Ok(())
}

/// Convert a SQLite value from sqlx Row to Python object.
fn sqlite_value_to_py<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    col: usize,
) -> PyResult<Py<PyAny>> {
    // Try Option types first to detect NULL
    if let Ok(opt_val) = row.try_get::<Option<i64>, _>(col) {
        return Ok(match opt_val {
            Some(val) => PyInt::new(py, val).into(),
            None => py.None(),
        });
    }

    if let Ok(opt_val) = row.try_get::<Option<f64>, _>(col) {
        return Ok(match opt_val {
            Some(val) => PyFloat::new(py, val).into(),
            None => py.None(),
        });
    }

    if let Ok(opt_val) = row.try_get::<Option<String>, _>(col) {
        return Ok(match opt_val {
            Some(val) => PyString::new(py, &val).into(),
            None => py.None(),
        });
    }

    if let Ok(opt_val) = row.try_get::<Option<Vec<u8>>, _>(col) {
        return Ok(match opt_val {
            Some(val) => PyBytes::new(py, &val).into(),
            None => py.None(),
        });
    }

    // Try non-Option types
    if let Ok(val) = row.try_get::<i64, _>(col) {
        return Ok(PyInt::new(py, val).into());
    }

    if let Ok(val) = row.try_get::<f64, _>(col) {
        return Ok(PyFloat::new(py, val).into());
    }

    if let Ok(val) = row.try_get::<String, _>(col) {
        return Ok(PyString::new(py, &val).into());
    }

    if let Ok(val) = row.try_get::<Vec<u8>, _>(col) {
        return Ok(PyBytes::new(py, &val).into());
    }

    // Last resort: return None (treat as NULL)
    Ok(py.None())
}

/// Convert a SQLite row to Python list.
fn row_to_py_list<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for i in 0..row.len() {
        let val = sqlite_value_to_py(py, row, i)?;
        list.append(val)?;
    }
    Ok(list)
}

/// Convert a SQLite row to Python using row_factory. factory None => list;
/// "dict" => dict (column names as keys); "tuple" => tuple; else callable(row) => result.
fn row_to_py_with_factory<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    factory: Option<&Py<PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let default = || row_to_py_list(py, row).map(|l| l.into_any());
    let Some(f) = factory else {
        return default();
    };
    let f = f.bind(py);
    if f.is_none() {
        return default();
    }
    if let Ok(s) = f.downcast::<PyString>() {
        let name = s.to_str()?;
        return match name {
            "dict" => {
                let dict = PyDict::new(py);
                for i in 0..row.len() {
                    let col_name = row.columns()[i].name();
                    let val = sqlite_value_to_py(py, row, i)?;
                    dict.set_item(col_name, val)?;
                }
                Ok(dict.into_any())
            }
            "tuple" => {
                let mut vals = Vec::new();
                for i in 0..row.len() {
                    vals.push(sqlite_value_to_py(py, row, i)?);
                }
                let tuple = PyTuple::new(py, vals)?;
                Ok(tuple.into_any())
            }
            _ => default(),
        };
    }
    let list = row_to_py_list(py, row)?;
    let result = f.call1((list,))?;
    Ok(result)
}

/// Convert a Python value to a SQLite-compatible value for binding.
/// Returns a boxed value that can be used with sqlx query binding.
enum SqliteParam {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqliteParam {
    fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        // Check for None first
        if value.is_none() {
            return Ok(SqliteParam::Null);
        }

        // Try to extract as i64 (integer)
        if let Ok(int_val) = value.extract::<i64>() {
            return Ok(SqliteParam::Int(int_val));
        }

        // Try to extract as f64 (float)
        if let Ok(float_val) = value.extract::<f64>() {
            return Ok(SqliteParam::Real(float_val));
        }

        // Try to extract as String
        if let Ok(str_val) = value.extract::<String>() {
            return Ok(SqliteParam::Text(str_val));
        }

        // Try to extract as &str
        if let Ok(str_val) = value.extract::<&str>() {
            return Ok(SqliteParam::Text(str_val.to_string()));
        }

        // Try to extract as bytes (Vec<u8>)
        if let Ok(bytes_val) = value.extract::<Vec<u8>>() {
            return Ok(SqliteParam::Blob(bytes_val));
        }

        // Try to extract as PyBytes
        if let Ok(py_bytes) = value.downcast::<PyBytes>() {
            return Ok(SqliteParam::Blob(py_bytes.as_bytes().to_vec()));
        }

        // Try to extract as int (Python int)
        if let Ok(py_int) = value.downcast::<PyInt>() {
            if let Ok(int_val) = py_int.extract::<i64>() {
                return Ok(SqliteParam::Int(int_val));
            }
            // For very large Python ints, convert to string
            // SQLite can handle large integers as text, but we'll keep as int if possible
            return Ok(SqliteParam::Text(py_int.to_string()));
        }

        // Try to extract as float
        if let Ok(py_float) = value.downcast::<PyFloat>() {
            if let Ok(float_val) = py_float.extract::<f64>() {
                return Ok(SqliteParam::Real(float_val));
            }
        }

        // Try to extract as string (PyString)
        if let Ok(py_str) = value.downcast::<PyString>() {
            return Ok(SqliteParam::Text(py_str.to_str()?.to_string()));
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            format!("Unsupported parameter type: {}. Use int, float, str, bytes, or None.", value.get_type().name()?),
        ))
    }
}

/// Parse named parameters from SQL query and convert to positional.
/// Returns the processed query with ? placeholders and ordered parameter values.
fn process_named_parameters(
    query: &str,
    dict: &Bound<'_, pyo3::types::PyDict>,
) -> PyResult<(String, Vec<SqliteParam>)> {
    let mut processed_query = query.to_string();
    let mut param_values = Vec::new();
    
    // Find all named parameter placeholders in order of appearance
    let mut param_placeholders: Vec<(usize, usize, String)> = Vec::new();
    let query_chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    
    while i < query_chars.len() {
        let ch = query_chars[i];
        
        // Check for :name, @name, or $name patterns
        if (ch == ':' || ch == '@') && i + 1 < query_chars.len() && 
           (query_chars[i + 1].is_alphabetic() || query_chars[i + 1] == '_') {
            let start = i;
            i += 1; // Skip the prefix
            let mut name = String::new();
            
            while i < query_chars.len() {
                let c = query_chars[i];
                if c.is_alphanumeric() || c == '_' {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            
            if !name.is_empty() {
                param_placeholders.push((start, i, name));
            }
        } else if ch == '$' && i + 1 < query_chars.len() && 
                  (query_chars[i + 1].is_alphabetic() || query_chars[i + 1] == '_') {
            let start = i;
            i += 1; // Skip the $
            let mut name = String::new();
            
            while i < query_chars.len() {
                let c = query_chars[i];
                if c.is_alphanumeric() || c == '_' {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            
            if !name.is_empty() {
                param_placeholders.push((start, i, name));
            }
        } else {
            i += 1;
        }
    }
    
    // Replace named parameters with ? and collect values in order
    // Process from end to start to avoid index shifting issues
    for (start, end, name) in param_placeholders.into_iter().rev() {
        if let Ok(Some(value)) = dict.get_item(name.as_str()) {
            let sqlx_param = SqliteParam::from_py(&value)?;
            param_values.push(sqlx_param);
            
            // Replace the named parameter with ?
            processed_query.replace_range(start..end, "?");
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(
                format!("Missing parameter: {}", name),
            ));
        }
    }
    
    // Reverse to get correct order (we processed backwards)
    param_values.reverse();
    
    Ok((processed_query, param_values))
}

/// Process positional parameters from a list/tuple.
fn process_positional_parameters(list: &Bound<'_, PyList>) -> PyResult<Vec<SqliteParam>> {
    let mut param_values = Vec::new();
    for item in list.iter() {
        let param = SqliteParam::from_py(&item)?;
        param_values.push(param);
    }
    Ok(param_values)
}

/// Bind parameters to a query and execute it.
/// This helper binds parameters dynamically to a sqlx query builder.
async fn bind_and_execute(
    query: &str,
    params: &[SqliteParam],
    pool: &SqlitePool,
    path: &str,
) -> Result<sqlx::sqlite::SqliteQueryResult, PyErr> {
    // Build query with bound parameters
    // sqlx uses method chaining, so we need to handle this carefully
    // For now, we'll use a match statement for common parameter counts
    // and fall back to building the query string with embedded values for larger counts
    
    let result = match params.len() {
        0 => sqlx::query(query).execute(pool).await,
        1 => {
            match &params[0] {
                SqliteParam::Null => sqlx::query(query).bind(Option::<i64>::None).execute(pool).await,
                SqliteParam::Int(v) => sqlx::query(query).bind(*v).execute(pool).await,
                SqliteParam::Real(v) => sqlx::query(query).bind(*v).execute(pool).await,
                SqliteParam::Text(v) => sqlx::query(query).bind(v.as_str()).execute(pool).await,
                SqliteParam::Blob(v) => sqlx::query(query).bind(v.as_slice()).execute(pool).await,
            }
        }
        _ => {
            // For multiple parameters, we need to chain binds
            // This is complex with sqlx's API, so we'll use a workaround:
            // Build the query with parameters bound sequentially
            // Since sqlx's bind chains are compile-time, we'll handle common cases
            // and use a helper that builds the query properly
            
            // Actually, let's use sqlx's query builder more directly
            // We can build a query by chaining binds - but we need to do this at compile time
            // For dynamic binding, we'll need a different approach
            
            // Workaround: Use sqlx::query and bind parameters one by one in a helper macro
            // or use a prepared statement approach
            
            // For now, let's handle up to 16 parameters (which should cover most cases)
            // using a helper that chains binds
            bind_query_multiple(query, params, pool).await
        }
    };
    
    result.map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and execute on a specific connection.
/// Similar to bind_and_execute but takes a PoolConnection instead of Pool.
async fn bind_and_execute_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<sqlx::sqlite::SqliteQueryResult, PyErr> {
    // Use &mut **conn to access the underlying connection that implements Executor
    let result = match params.len() {
        0 => sqlx::query(query).execute(&mut **conn).await,
        1 => {
            match &params[0] {
                SqliteParam::Null => sqlx::query(query).bind(Option::<i64>::None).execute(&mut **conn).await,
                SqliteParam::Int(v) => sqlx::query(query).bind(*v).execute(&mut **conn).await,
                SqliteParam::Real(v) => sqlx::query(query).bind(*v).execute(&mut **conn).await,
                SqliteParam::Text(v) => sqlx::query(query).bind(v.as_str()).execute(&mut **conn).await,
                SqliteParam::Blob(v) => sqlx::query(query).bind(v.as_slice()).execute(&mut **conn).await,
            }
        }
        _ => {
            // For multiple parameters, use bind_query_multiple_on_connection
            bind_query_multiple_on_connection(query, params, conn).await
        }
    };
    
    result.map_err(|e| map_sqlx_error(e, path, query))
}

/// Macro to bind a chain of parameters to a query builder
macro_rules! bind_chain {
    ($query:expr, $params:expr, $($idx:expr),*) => {
        {
            let q = sqlx::query($query);
            $(
                let q = match &$params[$idx] {
                    SqliteParam::Null => q.bind(Option::<i64>::None),
                    SqliteParam::Int(v) => q.bind(*v),
                    SqliteParam::Real(v) => q.bind(*v),
                    SqliteParam::Text(v) => q.bind(v.as_str()),
                    SqliteParam::Blob(v) => q.bind(v.as_slice()),
                };
            )*
            q
        }
    };
}

/// Helper to bind multiple parameters to a query and execute on a connection.
async fn bind_query_multiple_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    if params.is_empty() {
        return sqlx::query(query).execute(&mut **conn).await;
    }
    
    if params.len() > 16 {
        return Err(sqlx::Error::Protocol(
            format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
        ));
    }
    
    // Match on parameter count and use the macro to generate the bind chain
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(), // Already checked above
    };
    
    query_builder.execute(&mut **conn).await
}

/// Helper to bind multiple parameters to a query and execute it.
/// Handles up to 16 parameters using explicit bind chains.
async fn bind_query_multiple(
    query: &str,
    params: &[SqliteParam],
    pool: &SqlitePool,
) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    if params.is_empty() {
        return sqlx::query(query).execute(pool).await;
    }
    
    if params.len() > 16 {
        return Err(sqlx::Error::Protocol(
            format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
        ));
    }
    
    // Match on parameter count and use the macro to generate the bind chain
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(), // Already checked above
    };
    
    query_builder.execute(pool).await
}

/// Helper to bind parameters and fetch all rows.
async fn bind_and_fetch_all(
    query: &str,
    params: &[SqliteParam],
    pool: &SqlitePool,
    path: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_all(pool)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    
    query_builder
        .fetch_all(pool)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch one row.
async fn bind_and_fetch_one(
    query: &str,
    params: &[SqliteParam],
    pool: &SqlitePool,
    path: &str,
) -> Result<sqlx::sqlite::SqliteRow, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_one(pool)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    
    query_builder
        .fetch_one(pool)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch optional row.
async fn bind_and_fetch_optional(
    query: &str,
    params: &[SqliteParam],
    pool: &SqlitePool,
    path: &str,
) -> Result<Option<sqlx::sqlite::SqliteRow>, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_optional(pool)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    
    query_builder
        .fetch_optional(pool)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch all rows on a specific connection.
async fn bind_and_fetch_all_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_all(&mut **conn)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    query_builder
        .fetch_all(&mut **conn)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch one row on a specific connection.
async fn bind_and_fetch_one_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<sqlx::sqlite::SqliteRow, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_one(&mut **conn)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    query_builder
        .fetch_one(&mut **conn)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch optional row on a specific connection.
async fn bind_and_fetch_optional_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<Option<sqlx::sqlite::SqliteRow>, PyErr> {
    if params.is_empty() {
        return sqlx::query(query)
            .fetch_optional(&mut **conn)
            .await
            .map_err(|e| map_sqlx_error(e, path, query));
    }
    if params.len() > 16 {
        return Err(map_sqlx_error(
            sqlx::Error::Protocol(
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len()).into()
            ),
            path,
            query,
        ));
    }
    let query_builder = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!(),
    };
    query_builder
        .fetch_optional(&mut **conn)
        .await
        .map_err(|e| map_sqlx_error(e, path, query))
}

/// Helper to get or create pool and apply PRAGMAs.
async fn get_or_create_pool(
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
        if let Some(secs) = timeout_secs {
            opts = opts.acquire_timeout(Duration::from_secs(secs));
        }
        let new_pool = opts
            .connect(&format!("sqlite:{}", path))
            .await
            .map_err(|e| {
                OperationalError::new_err(format!(
                    "Failed to connect to database at {}: {e}",
                    path
                ))
            })?;
        
        // Apply PRAGMAs
        let pragmas_list = {
            let pragmas_guard = pragmas.lock().unwrap();
            pragmas_guard.clone()
        };
        
        for (name, value) in pragmas_list {
            let pragma_query = format!("PRAGMA {} = {}", name, value);
            sqlx::query(&pragma_query)
                .execute(&new_pool)
                .await
                .map_err(|e| map_sqlx_error(e, path, &pragma_query))?;
        }
        
        // TODO: Call init_hook if provided (requires proper async coroutine handling)
        // This will be implemented in a follow-up optimization
        
        *pool_guard = Some(new_pool);
    }
    Ok(pool_guard.as_ref().unwrap().clone())
}

/// Helper to ensure callback connection exists.
/// This acquires a connection from the pool and stores it for callback installation.
/// The connection is stored in the callback_connection mutex and should be accessed via that mutex.
/// Note: Accessing the raw sqlite3* handle from PoolConnection requires further research
/// into sqlx 0.8's API. This is a known limitation that needs to be resolved.
async fn ensure_callback_connection(
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
        let pool_clone = get_or_create_pool(path, pool, pragmas, pool_size, connection_timeout_secs).await?;
        
        // Acquire a connection from the pool
        let pool_conn = pool_clone.acquire().await
            .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection for callbacks: {}", e)))?;
        
        *callback_guard = Some(pool_conn);
    }
    Ok(())
}

/// Check if any callbacks are currently set.
fn has_callbacks(
    load_extension_enabled: &Arc<StdMutex<bool>>,
    user_functions: &Arc<StdMutex<HashMap<String, (i32, Py<PyAny>)>>>,
    trace_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    authorizer_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: &Arc<StdMutex<Option<(i32, Py<PyAny>)>>>,
) -> bool {
    let load_ext = *load_extension_enabled.lock().unwrap();
    let has_functions = !user_functions.lock().unwrap().is_empty();
    let has_trace = trace_callback.lock().unwrap().is_some();
    let has_authorizer = authorizer_callback.lock().unwrap().is_some();
    let has_progress = progress_handler.lock().unwrap().is_some();
    
    load_ext || has_functions || has_trace || has_authorizer || has_progress
}

/// Helper to determine which connection to use for operations.
/// Returns:
/// - Transaction connection if transaction is active (priority)
/// - Callback connection if callbacks are set
/// - None if using pool
async fn get_active_connection_for_operation(
    transaction_state: &Arc<Mutex<TransactionState>>,
    transaction_connection: &Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    callback_connection: &Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    has_callbacks_flag: bool,
) -> Option<PoolConnection<sqlx::Sqlite>> {
    // Check transaction first (highest priority)
    let trans_guard = transaction_state.lock().await;
    if *trans_guard == TransactionState::Active {
        drop(trans_guard);
        let conn_guard = transaction_connection.lock().await;
        // Don't take ownership, just check if it exists
        // Actually, we can't return a reference, so we need a different approach
        // For now, return None and let the caller handle it
        return None; // Signal to use transaction connection (handled separately)
    }
    drop(trans_guard);
    
    // If callbacks are set, use callback connection
    if has_callbacks_flag {
        let conn_guard = callback_connection.lock().await;
        // Similar issue - can't return reference
        return None; // Signal to use callback connection (handled separately)
    }
    
    None // Use pool
}

/// Map sqlx error to appropriate Python exception.
fn map_sqlx_error(e: sqlx::Error, path: &str, query: &str) -> PyErr {
    use sqlx::Error as SqlxError;

    let error_msg = format!(
        "Failed to execute query on database {}: {e}\nQuery: {}",
        path, query
    );

    match e {
        SqlxError::Database(db_err) => {
            let msg = db_err.message();
            // Check for specific SQLite error codes
            if msg.contains("SQLITE_CONSTRAINT")
                || msg.contains("UNIQUE constraint")
                || msg.contains("NOT NULL constraint")
                || msg.contains("FOREIGN KEY constraint")
            {
                IntegrityError::new_err(error_msg)
            } else if msg.contains("SQLITE_BUSY") || msg.contains("database is locked") {
                OperationalError::new_err(error_msg)
            } else {
                DatabaseError::new_err(error_msg)
            }
        }
        SqlxError::Protocol(_) | SqlxError::Io(_) => OperationalError::new_err(error_msg),
        SqlxError::ColumnNotFound(_) | SqlxError::ColumnIndexOutOfBounds { .. } => {
            ProgrammingError::new_err(error_msg)
        }
        SqlxError::Decode(_) => ProgrammingError::new_err(error_msg),
        _ => DatabaseError::new_err(error_msg),
    }
}

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Connection>()?;
    m.add_class::<Cursor>()?;
    m.add_class::<TransactionContextManager>()?;

    // Register exception classes (required for create_exception! to be accessible from Python)
    m.add("Error", py.get_type::<Error>())?;
    m.add("Warning", py.get_type::<Warning>())?;
    m.add("DatabaseError", py.get_type::<DatabaseError>())?;
    m.add("OperationalError", py.get_type::<OperationalError>())?;
    m.add("ProgrammingError", py.get_type::<ProgrammingError>())?;
    m.add("IntegrityError", py.get_type::<IntegrityError>())?;

    Ok(())
}

/// Transaction state tracking.
#[derive(Clone, PartialEq)]
enum TransactionState {
    None,
    Active,
}

/// Async SQLite connection.
#[pyclass]
struct Connection {
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    // Store the connection used for active transaction
    // All operations within a transaction must use this same connection
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    last_rowid: Arc<Mutex<i64>>,
    last_changes: Arc<Mutex<u64>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Store PRAGMA settings
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>, // Optional initialization hook
    pool_size: Arc<StdMutex<Option<usize>>>, // Configurable pool size
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>, // Connection timeout in seconds
    row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // None | "dict" | "tuple" | callable
    // Callback infrastructure (Phase 2.7)
    callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>, // Dedicated connection for callbacks
    load_extension_enabled: Arc<StdMutex<bool>>, // Track load_extension state
    user_functions: Arc<StdMutex<HashMap<String, (i32, Py<PyAny>)>>>, // name -> (nargs, callback)
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Trace callback
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Authorizer callback
    progress_handler: Arc<StdMutex<Option<(i32, Py<PyAny>)>>>, // (n, callback)
}

#[pymethods]
impl Connection {
    /// Create a new async SQLite connection.
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
            pool_size: Arc::new(StdMutex::new(None)),
            connection_timeout_secs: Arc::new(StdMutex::new(None)),
            row_factory: Arc::new(StdMutex::new(None)),
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
    fn row_factory(&self) -> PyResult<Py<PyAny>> {
        Python::with_gil(|py| {
            let guard = self.row_factory.lock().unwrap();
            Ok(match guard.as_ref() {
                Some(f) => f.clone_ref(py),
                None => py.None(),
            })
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

    #[getter(pool_size)]
    fn pool_size(&self) -> PyResult<Py<PyAny>> {
        Python::with_gil(|py| {
            let guard = self.pool_size.lock().unwrap();
            Ok(match guard.as_ref() {
                Some(&n) => PyInt::new(py, n as i64).into_any().unbind(),
                None => py.None(),
            })
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
    fn begin(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    return Err(OperationalError::new_err("Transaction already in progress"));
                }

                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Acquire a connection from the pool and store it for the transaction
                // All operations within the transaction must use this same connection
                let mut conn = pool_clone.acquire().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {}", e)))?;

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

                *trans_guard = TransactionState::Active;
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
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                // Retrieve the stored transaction connection
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard.take()
                    .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;

                // Execute COMMIT on the same connection that started the transaction
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "COMMIT"))?;

                // Connection is automatically returned to pool when dropped
                drop(conn);

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
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                // Retrieve the stored transaction connection
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard.take()
                    .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;

                // Execute ROLLBACK on the same connection that started the transaction
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "ROLLBACK"))?;

                // Connection is automatically returned to pool when dropped
                drop(conn);

                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Execute a SQL query (does not return results).
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
        
        // Process parameters
        let (processed_query, param_values) = Python::with_gil(|py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };
            
            let params = params.as_borrowed();
            
            // Check if it's a dict (named parameters)
            if let Ok(dict) = params.downcast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, dict);
            }
            
            // Check if it's a list or tuple (positional parameters)
            if let Ok(list) = params.downcast::<PyList>() {
                let params_vec = process_positional_parameters(list)?;
                return Ok((query, params_vec));
            }
            
            // Single value (treat as single positional parameter)
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;
        
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

                // Check if we're in a transaction - use stored connection if active
                let mut conn_guard = transaction_connection.lock().await;
                let result = if let Some(conn) = conn_guard.as_mut() {
                    // Use transaction connection - need &mut **conn to access underlying connection that implements Executor
                    bind_and_execute_on_connection(&processed_query, &param_values, conn, &path)
                        .await?
                } else {
                    // Use pool
                    bind_and_execute(&processed_query, &param_values, &pool_clone, &path)
                        .await?
                };
                drop(conn_guard);

                let rowid = result.last_insert_rowid();
                let changes = result.rows_affected();

                *last_rowid.lock().await = rowid;
                *last_changes.lock().await = changes;

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
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
        
        // Process all parameter sets
        // Each element in parameters is a list/tuple of parameters for one execution
        let processed_params = Python::with_gil(|py| -> PyResult<Vec<Vec<SqliteParam>>> {
            let mut result = Vec::new();
            for param_set in parameters.iter() {
                // Convert Vec<Py<PyAny>> to Vec<SqliteParam>
                let mut params_vec = Vec::new();
                for param in param_set {
                    let bound_param = param.bind(py);
                    let sqlx_param = SqliteParam::from_py(&bound_param)?;
                    params_vec.push(sqlx_param);
                }
                result.push(params_vec);
            }
            Ok(result)
        })?;
        
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

                let mut total_changes = 0u64;
                let mut last_row_id = 0i64;

                // Check if we're in a transaction - use stored connection if active
                let in_transaction = {
                    let trans_guard = transaction_state.lock().await;
                    *trans_guard == TransactionState::Active
                };

                if in_transaction {
                    // Use stored transaction connection. Release lock each iteration
                    // to match the execute-in-loop pattern (lock -> use -> release).
                    for param_values in processed_params.iter() {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        let result =
                            bind_and_execute_on_connection(&query, param_values, conn, &path).await?;
                        total_changes += result.rows_affected();
                        last_row_id = result.last_insert_rowid();
                        drop(conn_guard);
                    }
                } else {
                    for param_values in processed_params {
                        let result = bind_and_execute(&query, &param_values, &pool_clone, &path)
                            .await?;
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

        // Process parameters
        let (processed_query, param_values) = Python::with_gil(|py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            // Check if it's a dict (named parameters)
            if let Ok(dict) = params.downcast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, dict);
            }

            // Check if it's a list or tuple (positional parameters)
            if let Ok(list) = params.downcast::<PyList>() {
                let params_vec = process_positional_parameters(list)?;
                return Ok((query, params_vec));
            }

            // Single value (treat as single positional parameter)
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&processed_query, &param_values, conn, &path).await?
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

        // Process parameters
        let (processed_query, param_values) = Python::with_gil(|py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            if let Ok(dict) = params.downcast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, dict);
            }
            if let Ok(list) = params.downcast::<PyList>() {
                let params_vec = process_positional_parameters(list)?;
                return Ok((query, params_vec));
            }
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let row = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_one_on_connection(&processed_query, &param_values, conn, &path).await?
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

        // Process parameters
        let (processed_query, param_values) = Python::with_gil(|py| -> PyResult<_> {
            let Some(params) = parameters else {
                return Ok((query, Vec::new()));
            };

            let params = params.as_borrowed();

            if let Ok(dict) = params.downcast::<pyo3::types::PyDict>() {
                return process_named_parameters(&query, dict);
            }
            if let Ok(list) = params.downcast::<PyList>() {
                let params_vec = process_positional_parameters(list)?;
                return Ok((query, params_vec));
            }
            let param = SqliteParam::from_py(&params)?;
            Ok((query, vec![param]))
        })?;

        Python::attach(|py| {
            let future = async move {
                let in_transaction = {
                    let g = transaction_state.lock().await;
                    *g == TransactionState::Active
                };

                let opt = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_optional_on_connection(&processed_query, &param_values, conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    bind_and_fetch_optional(&processed_query, &param_values, &pool_clone, &path).await?
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
        Ok(Cursor {
            connection: slf.into(),
            query: String::new(),
            results: Arc::new(StdMutex::new(None)),
            current_index: Arc::new(StdMutex::new(0)),
            parameters: Arc::new(StdMutex::new(None)),
            connection_path: path,
            connection_pool: pool,
            connection_pragmas: pragmas,
            pool_size,
            connection_timeout_secs,
            row_factory,
        })
    }

    /// Return an async context manager for a transaction.
    /// On __aenter__ calls begin(); on __aexit__ calls commit() or rollback().
    fn transaction(slf: PyRef<Self>) -> PyResult<TransactionContextManager> {
        Ok(TransactionContextManager {
            path: slf.path.clone(),
            pool: Arc::clone(&slf.pool),
            pragmas: Arc::clone(&slf.pragmas),
            pool_size: Arc::clone(&slf.pool_size),
            connection_timeout_secs: Arc::clone(&slf.connection_timeout_secs),
            transaction_state: Arc::clone(&slf.transaction_state),
            transaction_connection: Arc::clone(&slf.transaction_connection),
            connection: slf.into(),
        })
    }

    /// Set a PRAGMA value on the database connection.
    fn set_pragma(&self, name: String, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        
        // Convert value to string for PRAGMA
        let pragma_value = Python::with_gil(|py| -> PyResult<String> {
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
        
        let pragma_query = format!("PRAGMA {} = {}", name, pragma_value);
        
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                // Store the state
                {
                    let mut enabled_guard = load_extension_enabled.lock().unwrap();
                    *enabled_guard = enabled;
                }
                
                // Access raw sqlite3* handle via PoolConnection's Deref to SqliteConnection
                // PoolConnection<Sqlite> derefs to SqliteConnection, so we can use &mut *conn
                // Then call lock_handle() to get LockedSqliteHandle, then as_raw_handle() for NonNull<sqlite3>
                let sqlite_conn: &mut SqliteConnection = &mut **conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                // Call the C API
                let enabled_int = if enabled { 1 } else { 0 };
                let result = unsafe {
                    sqlite3_enable_load_extension(raw_db, enabled_int)
                };
                
                if result != 0 {
                    return Err(OperationalError::new_err(
                        format!("Failed to enable/disable load extension: SQLite error code {}", result)
                    ));
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
        
        Python::attach(|py| {
            // Clone the callback with GIL to avoid Send issues
            let func_clone = func.as_ref().map(|f| f.clone_ref(py));
            
            let future = async move {
                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = &mut **conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                if func_clone.is_none() {
                    // Remove the function
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.remove(&name);
                    }
                    
                    // Call sqlite3_create_function_v2 with NULL callback to remove
                    let name_cstr = std::ffi::CString::new(name.clone())
                        .map_err(|e| OperationalError::new_err(format!("Function name contains null byte: {}", e)))?;
                    let result = unsafe {
                        sqlite3_create_function_v2(
                            raw_db,
                            name_cstr.as_ptr(),
                            nargs,
                            SQLITE_UTF8,
                            std::ptr::null_mut(), // pApp (user data)
                            None, // xFunc (scalar function callback)
                            None, // xStep (aggregate step callback)
                            None, // xFinal (aggregate final callback)
                            None, // xDestroy (destructor)
                        )
                    };
                    
                    if result != SQLITE_OK {
                        return Err(OperationalError::new_err(
                            format!("Failed to remove function '{}': SQLite error code {}", name, result)
                        ));
                    }
                } else {
                    // Store the function - need to clone the callback with GIL
                    let callback_for_storage = Python::with_gil(|py| {
                        func_clone.as_ref().unwrap().clone_ref(py)
                    });
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.insert(name.clone(), (nargs, callback_for_storage));
                    }
                    
                    // Create a boxed callback pointer to pass as user data
                    let name_cstr = std::ffi::CString::new(name.clone())
                        .map_err(|e| OperationalError::new_err(format!("Function name contains null byte: {}", e)))?;
                    
                    // Store the Python callback in a Box and pass it as user_data
                    // Clone it with GIL
                    let callback = Python::with_gil(|py| {
                        func_clone.as_ref().unwrap().clone_ref(py)
                    });
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
                            Python::with_gil(|py| {
                                // Clone the callback to use it (the original stays in the Box)
                                let callback = unsafe { (*callback_ptr).clone_ref(py) };
                                
                                let mut py_args = Vec::new();
                                for i in 0..argc {
                                    let value_ptr = *argv.add(i as usize);
                                    match sqlite_c_value_to_py(py, value_ptr) {
                                        Ok(py_val) => {
                                            py_args.push(py_val);
                                        }
                                        Err(e) => {
                                            // On error, set SQLite error and return
                                            let error_msg = format!("Error converting argument {}: {}", i, e);
                                            libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
                                            return;
                                        }
                                    }
                                }
                                
                                // Call the Python callback
                                match PyTuple::new(py, py_args) {
                                    Ok(py_args_tuple) => {
                                        match callback.bind(py).call1((py_args_tuple,)) {
                                            Ok(result) => {
                                                // Convert result back to SQLite
                                                match py_to_sqlite_c_result(py, ctx, &result) {
                                                    Ok(_) => {}
                                                    Err(e) => {
                                                        let error_msg = format!("Error converting result: {}", e);
                                                        libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                // Python exception - convert to SQLite error
                                                let error_msg = format!("Python function error: {}", e);
                                                libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let error_msg = format!("Error creating argument tuple: {}", e);
                                        libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
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
                            None, // xStep (aggregate step callback)
                            None, // xFinal (aggregate final callback)
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
                        return Err(OperationalError::new_err(
                            format!("Failed to create function '{}': SQLite error code {}", name, result)
                        ));
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
        
        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            
            // Store the callback state
            {
                let mut trace_guard = trace_callback.lock().unwrap();
                *trace_guard = callback_clone;
            }
            
            let future = async move {
                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = &mut **conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
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
                        let sql_str = match std::ffi::CStr::from_ptr(sql_cstr).to_str() {
                            Ok(s) => s.to_string(),
                            Err(_) => return 0, // Invalid UTF-8, skip
                        };
                        
                        // Get the Python callback from the context (pCtx)
                        let callback_ptr = ctx as *mut Py<PyAny>;
                        
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
                        if callback_ptr.is_null() { 0 } else { SQLITE_TRACE_STMT as u32 }, // Trace mask
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
                    return Err(OperationalError::new_err(
                        format!("Failed to set trace callback: SQLite error code {}", result)
                    ));
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
        
        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            
            // Store the callback state
            {
                let mut auth_guard = authorizer_callback.lock().unwrap();
                *auth_guard = callback_clone;
            }
            
            let future = async move {
                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = &mut **conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
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
                        let arg1_str = if arg1.is_null() {
                            None
                        } else {
                            std::ffi::CStr::from_ptr(arg1).to_str().ok().map(|s| s.to_string())
                        };
                        let arg2_str = if arg2.is_null() {
                            None
                        } else {
                            std::ffi::CStr::from_ptr(arg2).to_str().ok().map(|s| s.to_string())
                        };
                        let arg3_str = if arg3.is_null() {
                            None
                        } else {
                            std::ffi::CStr::from_ptr(arg3).to_str().ok().map(|s| s.to_string())
                        };
                        let arg4_str = if arg4.is_null() {
                            None
                        } else {
                            std::ffi::CStr::from_ptr(arg4).to_str().ok().map(|s| s.to_string())
                        };
                        
                        // Get the Python callback from the context
                        let callback_ptr = ctx as *mut Py<PyAny>;
                        
                        Python::with_gil(|py| {
                            let callback = (*callback_ptr).clone_ref(py);
                            
                            // Convert None strings to None in Python, otherwise pass the string
                            let py_arg1 = match arg1_str {
                                Some(s) => PyString::new(py, &s).into(),
                                None => py.None(),
                            };
                            let py_arg2 = match arg2_str {
                                Some(s) => PyString::new(py, &s).into(),
                                None => py.None(),
                            };
                            let py_arg3 = match arg3_str {
                                Some(s) => PyString::new(py, &s).into(),
                                None => py.None(),
                            };
                            let py_arg4 = match arg4_str {
                                Some(s) => PyString::new(py, &s).into(),
                                None => py.None(),
                            };
                            
                            match callback.bind(py).call1((action, py_arg1, py_arg2, py_arg3, py_arg4)) {
                                Ok(result) => {
                                    // Convert Python result to SQLite auth code
                                    if let Ok(code) = result.extract::<i32>() {
                                        code
                                    } else {
                                        SQLITE_OK // Default to OK if conversion fails
                                    }
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
        
        Python::attach(|py| {
            // Clone the callback with GIL
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            
            // Store the progress handler state
            {
                let mut progress_guard = progress_handler.lock().unwrap();
                *progress_guard = callback_clone.map(|c| (n, c));
            }
            
            let future = async move {
                // Ensure callback connection exists
                ensure_callback_connection(
                    &path,
                    &pool,
                    &callback_connection,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = &mut **conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {}", e)))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                // Define the progress handler callback trampoline
                extern "C" fn progress_trampoline(ctx: *mut std::ffi::c_void) -> std::ffi::c_int {
                    unsafe {
                        if ctx.is_null() {
                            return 0; // Continue
                        }
                        
                        // Get the Python callback from the context
                        let callback_ptr = ctx as *mut Py<PyAny>;
                        
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
                                    } else if let Ok(code) = result.extract::<i32>() {
                                        code // Use integer directly
                                    } else {
                                        0 // Default to continue if conversion fails
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
                
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}

/// Transaction context manager returned by `Connection::transaction()`.
/// Runs begin on __aenter__ and commit/rollback on __aexit__ using the same
/// connection state as the Connection.
#[pyclass]
struct TransactionContextManager {
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    connection: Py<Connection>,
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
            let transaction_state = Arc::clone(&slf.borrow(py).transaction_state);
            let transaction_connection = Arc::clone(&slf.borrow(py).transaction_connection);
            let connection = slf.borrow(py).connection.clone_ref(py);
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    return Err(OperationalError::new_err("Transaction already in progress"));
                }
                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;
                let mut conn = pool_clone
                    .acquire()
                    .await
                    .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {}", e)))?;
                sqlx::query("PRAGMA busy_timeout = 5000")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "PRAGMA busy_timeout = 5000"))?;
                sqlx::query("BEGIN IMMEDIATE")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "BEGIN IMMEDIATE"))?;
                {
                    let mut conn_guard = transaction_connection.lock().await;
                    *conn_guard = Some(conn);
                }
                *trans_guard = TransactionState::Active;
                Ok(connection)
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
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = conn_guard
                    .take()
                    .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                let query = if rollback { "ROLLBACK" } else { "COMMIT" };
                sqlx::query(query)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, query))?;
                drop(conn);
                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}

/// Cursor for executing queries.
#[pyclass]
struct Cursor {
    connection: Py<Connection>,
    query: String,
    results: Arc<StdMutex<Option<Vec<Py<PyAny>>>>>,
    current_index: Arc<StdMutex<usize>>,
    parameters: Arc<StdMutex<Option<Py<PyAny>>>>,
    connection_path: String, // Store path for direct pool access
    connection_pool: Arc<Mutex<Option<SqlitePool>>>, // Reference to connection's pool
    connection_pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Reference to connection's pragmas
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // Connection's row_factory at cursor creation
}

#[pymethods]
impl Cursor {
    /// Execute a SQL query.
    #[pyo3(signature = (query, parameters = None))]
    fn execute(
        &mut self,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        self.query = query.clone();
        
        // Store parameters
        let params_for_storage = if let Some(params) = parameters {
            Some(params.clone().unbind())
        } else {
            None
        };
        
        {
            let mut params_guard = self.parameters.lock().unwrap();
            *params_guard = params_for_storage;
        }
        
        // Reset cursor state for new query
        {
            *self.current_index.lock().unwrap() = 0;
            *self.results.lock().unwrap() = None;
        }
        
        // Execute via Connection (no results cached yet - will fetch on first fetch call)
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            if let Some(params) = parameters {
                conn.call_method1("execute", (query, params))
                    .map(|bound| bound.unbind())
            } else {
                conn.call_method1("execute", (query, py.None()))
                    .map(|bound| bound.unbind())
            }
        })
    }

    /// Execute a SQL query multiple times.
    fn executemany(
        &mut self,
        query: String,
        parameters: Vec<Vec<Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        self.query = query.clone();
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            conn.call_method1("execute_many", (query, parameters))
                .map(|bound| bound.unbind())
        })
    }

    /// Fetch one row.
    fn fetchone(&self) -> PyResult<Py<PyAny>> {
        if self.query.is_empty() {
            return Err(ProgrammingError::new_err("No query executed"));
        }
        
        // Use same logic as fetchmany but return single element or None
        let query = self.query.clone();
        let results = Arc::clone(&self.results);
        let current_index = Arc::clone(&self.current_index);
        let parameters = Arc::clone(&self.parameters);
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let row_factory = Arc::clone(&self.row_factory);
        
        Python::attach(|py| {
            let future = async move {
                // Ensure results are cached (same logic as fetchmany)
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    let processed_params = Python::with_gil(|py| -> PyResult<Vec<SqliteParam>> {
                        let params_guard = parameters.lock().unwrap();
                        if let Some(ref params_py) = *params_guard {
                            let params_bound = params_py.bind(py);
                            if let Ok(dict) = params_bound.downcast::<pyo3::types::PyDict>() {
                                let (_, param_values) = process_named_parameters(&query, dict)?;
                                return Ok(param_values);
                            }
                            if let Ok(list) = params_bound.downcast::<PyList>() {
                                return process_positional_parameters(list);
                            }
                            let param = SqliteParam::from_py(&params_bound)?;
                            return Ok(vec![param]);
                        }
                        Ok(Vec::new())
                    })?;
                    
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    
                    let rows = bind_and_fetch_all(&query, &processed_params, &pool_clone, &path).await?;
                    
                    let cached_results = Python::with_gil(|py| -> PyResult<Vec<Py<PyAny>>> {
                        let guard = row_factory.lock().unwrap();
                        let factory_opt = guard.as_ref();
                        let mut vec = Vec::new();
                        for row in rows.iter() {
                            let out = row_to_py_with_factory(py, row, factory_opt)?;
                            vec.push(out.unbind());
                        }
                        Ok(vec)
                    })?;
                    
                    {
                        let mut results_guard = results.lock().unwrap();
                        *results_guard = Some(cached_results);
                    }
                    *current_index.lock().unwrap() = 0;
                }
                
                // Get first element or None
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let mut index_guard = current_index.lock().unwrap();
                    let results_guard = results.lock().unwrap();
                    
                    let Some(ref results_vec) = *results_guard else {
                        return Ok(py.None());
                    };
                    
                    if *index_guard >= results_vec.len() {
                        return Ok(py.None());
                    }
                    
                    let row = results_vec[*index_guard].clone_ref(py);
                    *index_guard += 1;
                    
                    Ok(row)
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch all rows.
    fn fetchall(&self) -> PyResult<Py<PyAny>> {
        if self.query.is_empty() {
            return Err(ProgrammingError::new_err("No query executed"));
        }
        
        let query = self.query.clone();
        let results = Arc::clone(&self.results);
        let current_index = Arc::clone(&self.current_index);
        let parameters = Arc::clone(&self.parameters);
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let row_factory = Arc::clone(&self.row_factory);
        
        Python::attach(|py| {
            let future = async move {
                // Ensure results are cached
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    let processed_params = Python::with_gil(|py| -> PyResult<Vec<SqliteParam>> {
                        let params_guard = parameters.lock().unwrap();
                        if let Some(ref params_py) = *params_guard {
                            let params_bound = params_py.bind(py);
                            if let Ok(dict) = params_bound.downcast::<pyo3::types::PyDict>() {
                                let (_, param_values) = process_named_parameters(&query, dict)?;
                                return Ok(param_values);
                            }
                            if let Ok(list) = params_bound.downcast::<PyList>() {
                                return process_positional_parameters(list);
                            }
                            let param = SqliteParam::from_py(&params_bound)?;
                            return Ok(vec![param]);
                        }
                        Ok(Vec::new())
                    })?;
                    
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    
                    let rows = bind_and_fetch_all(&query, &processed_params, &pool_clone, &path).await?;
                    
                    let cached_results = Python::with_gil(|py| -> PyResult<Vec<Py<PyAny>>> {
                        let guard = row_factory.lock().unwrap();
                        let factory_opt = guard.as_ref();
                        let mut vec = Vec::new();
                        for row in rows.iter() {
                            let out = row_to_py_with_factory(py, row, factory_opt)?;
                            vec.push(out.unbind());
                        }
                        Ok(vec)
                    })?;
                    
                    {
                        let mut results_guard = results.lock().unwrap();
                        *results_guard = Some(cached_results);
                    }
                }
                
                // Return all remaining results
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let mut index_guard = current_index.lock().unwrap();
                    let results_guard = results.lock().unwrap();
                    
                    let Some(ref results_vec) = *results_guard else {
                        return Err(ProgrammingError::new_err("No results available"));
                    };
                    
                    let start = *index_guard;
                    let result_list = PyList::empty(py);
                    for row in &results_vec[start..] {
                        result_list.append(row.clone_ref(py))?;
                    }
                    
                    // Update index to end
                    *index_guard = results_vec.len();
                    
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch many rows with size-based slicing.
    /// Phase 2.2: Properly implements size parameter by fetching all results,
    /// caching them, and returning appropriate slices.
    fn fetchmany(&self, size: Option<usize>) -> PyResult<Py<PyAny>> {
        if self.query.is_empty() {
            return Err(ProgrammingError::new_err("No query executed"));
        }
        
        let query = self.query.clone();
        let results = Arc::clone(&self.results);
        let current_index = Arc::clone(&self.current_index);
        let parameters = Arc::clone(&self.parameters);
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let row_factory = Arc::clone(&self.row_factory);
        
        Python::attach(|py| {
            let future = async move {
                // Check if results need to be fetched
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    // Fetch all results using direct pool access
                    let processed_params = Python::with_gil(|py| -> PyResult<Vec<SqliteParam>> {
                        let params_guard = parameters.lock().unwrap();
                        if let Some(ref params_py) = *params_guard {
                            let params_bound = params_py.bind(py);
                            
                            // Check if it's a dict (named parameters)
                            if let Ok(dict) = params_bound.downcast::<pyo3::types::PyDict>() {
                                let (_, param_values) = process_named_parameters(&query, dict)?;
                                return Ok(param_values);
                            }
                            
                            // Check if it's a list (positional parameters)
                            if let Ok(list) = params_bound.downcast::<PyList>() {
                                return process_positional_parameters(list);
                            }
                            
                            // Single value
                            let param = SqliteParam::from_py(&params_bound)?;
                            return Ok(vec![param]);
                        }
                        Ok(Vec::new())
                    })?;
                    
                    // Fetch all results
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    
                    let rows = bind_and_fetch_all(&query, &processed_params, &pool_clone, &path).await?;
                    
                    // Cache results as Python objects
                    let cached_results = Python::with_gil(|py| -> PyResult<Vec<Py<PyAny>>> {
                        let guard = row_factory.lock().unwrap();
                        let factory_opt = guard.as_ref();
                        let mut vec = Vec::new();
                        for row in rows.iter() {
                            let out = row_to_py_with_factory(py, row, factory_opt)?;
                            vec.push(out.unbind());
                        }
                        Ok(vec)
                    })?;
                    
                    // Store cached results
                    {
                        let mut results_guard = results.lock().unwrap();
                        *results_guard = Some(cached_results);
                    }
                    
                    // Reset index
                    *current_index.lock().unwrap() = 0;
                }
                
                // Get slice based on size
                Python::with_gil(|py| -> PyResult<Py<PyAny>> {
                    let mut index_guard = current_index.lock().unwrap();
                    let results_guard = results.lock().unwrap();
                    
                    let Some(ref results_vec) = *results_guard else {
                        return Err(ProgrammingError::new_err("No results available"));
                    };
                    
                    let start = *index_guard;
                    let fetch_size = size.unwrap_or(1);
                    let end = std::cmp::min(start + fetch_size, results_vec.len());
                    
                    // Create result slice
                    let result_list = PyList::empty(py);
                    for row in &results_vec[start..end] {
                        result_list.append(row.clone_ref(py))?;
                    }
                    
                    // Update index for next call
                    *index_guard = end;
                    
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
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
        Python::attach(|py| {
            let future = async move {
                Ok(false) // Return False to not suppress exceptions
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}
