#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use pyo3_async_runtimes::tokio::{future_into_py, into_future};
use sqlx::{Column, Row, SqlitePool};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnection, SqlitePoolOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;

// Type aliases for complex types to reduce clippy warnings
type UserFunctions = Arc<StdMutex<HashMap<String, (i32, Py<PyAny>)>>>;
type ProgressHandler = Arc<StdMutex<Option<(i32, Py<PyAny>)>>>;

/// Detect if a query is a SELECT query (for determining execution strategy).
fn is_select_query(query: &str) -> bool {
    let trimmed = query.trim().to_uppercase();
    trimmed.starts_with("SELECT") || trimmed.starts_with("WITH")
}

/// Normalize a SQL query by removing extra whitespace and standardizing formatting.
/// This helps improve prepared statement cache hit rates by ensuring queries with
/// different whitespace are treated as identical.
/// 
/// **Prepared Statement Caching (Phase 2.13):**
/// sqlx (the underlying database library) automatically caches prepared statements
/// per connection. When the same query is executed multiple times on the same
/// connection, sqlx reuses the prepared statement, providing significant performance
/// benefits. This normalization function ensures that queries with only whitespace
/// differences are treated as identical, maximizing cache hit rates.
/// 
/// The prepared statement cache is managed entirely by sqlx and does not require
/// explicit configuration. Each connection in the pool maintains its own cache,
/// and statements are automatically prepared on first use and reused for subsequent
/// executions of the same query.
fn normalize_query(query: &str) -> String {
    // Remove leading/trailing whitespace
    let trimmed = query.trim();
    // Replace multiple whitespace characters with single space
    let normalized: String = trimmed
        .chars()
        .fold((String::new(), false), |(acc, was_space), ch| {
            let is_space = ch.is_whitespace();
            if is_space && was_space {
                // Skip multiple consecutive spaces
                (acc, true)
            } else if is_space {
                // Replace any whitespace with single space
                (acc + " ", true)
            } else {
                (acc + &ch.to_string(), false)
            }
        })
        .0;
    normalized
}

/// Track query usage in the cache for analytics and optimization.
/// This helps identify frequently used queries that benefit from prepared statement caching.
fn track_query_usage(query_cache: &Arc<StdMutex<HashMap<String, u64>>>, query: &str) {
    let normalized = normalize_query(query);
    let mut cache = query_cache.lock().unwrap();
    *cache.entry(normalized).or_insert(0) += 1;
}

// libsqlite3-sys for raw SQLite C API access
use libsqlite3_sys::{
    sqlite3, sqlite3_backup_finish, sqlite3_backup_init, sqlite3_backup_pagecount,
    sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_context, sqlite3_create_function_v2,
    sqlite3_enable_load_extension, sqlite3_errcode, sqlite3_errmsg, sqlite3_free, sqlite3_get_autocommit,
    sqlite3_libversion, sqlite3_load_extension, sqlite3_progress_handler, sqlite3_result_null,
    sqlite3_set_authorizer, sqlite3_total_changes, sqlite3_trace_v2, sqlite3_user_data, sqlite3_value, SQLITE_BUSY,
    SQLITE_DONE, SQLITE_LOCKED, SQLITE_OK, SQLITE_TRACE_STMT,
    SQLITE_UTF8,
};
use std::ffi::{CStr, CString};

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
    if let Some(uri_part) = uri.strip_prefix("file:") {
        // Parse URI: file:path?param=value&param2=value2
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
                let text_slice = std::slice::from_raw_parts(text_ptr, text_len);
                let text_str = std::str::from_utf8(text_slice)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        format!("Invalid UTF-8 in SQLite text value: {e}")
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
    _py: Python<'_>,
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
                format!("String contains null byte: {e}")
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
    if let Ok(py_bytes) = result.cast::<PyBytes>() {
        let bytes = py_bytes.as_bytes();
        let len = bytes.len() as i32;
        let ptr = bytes.as_ptr();
        sqlite3_result_blob(ctx, ptr as *const std::ffi::c_void, len, libsqlite3_sys::SQLITE_TRANSIENT());
        return Ok(());
    }
    
    // Try PyString
    if let Ok(py_str) = result.cast::<PyString>() {
        let str_val = py_str.to_str()?;
        let c_str = std::ffi::CString::new(str_val)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("String contains null byte: {e}")
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
/// "dict" => dict (column names as keys); "tuple" => tuple; Row class => RapRow instance; else callable(row) => result.
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
    if let Ok(s) = f.cast::<PyString>() {
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
    
    // Check if factory is the RapRow class (Row class from Python)
    // Try to get RapRow class from the module and compare types
    if let Ok(rapsqlite_mod) = py.import("rapsqlite._rapsqlite") {
        if let Ok(raprow_class) = rapsqlite_mod.getattr("RapRow") {
            // Check if f is the same type as RapRow class by comparing type objects
            let f_type = f.get_type();
            let raprow_type = raprow_class.get_type();
            if f_type.is(raprow_type) {
                // Create RapRow with columns and values
                let mut columns = Vec::new();
                let mut values = Vec::new();
                for i in 0..row.len() {
                    columns.push(row.columns()[i].name().to_string());
                    let val = sqlite_value_to_py(py, row, i)?;
                    values.push(val);
                }
                let raprow = raprow_class.call1((columns, values))?;
                return Ok(raprow.into_any());
            }
        }
    }
    
    // Fallback: treat as callable
    let list = row_to_py_list(py, row)?;
    let result = f.call1((list,))?;
    Ok(result)
}

/// Convert a Python value to a SQLite-compatible value for binding.
/// Returns a boxed value that can be used with sqlx query binding.
#[derive(Clone)]
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
        if let Ok(py_bytes) = value.cast::<PyBytes>() {
            return Ok(SqliteParam::Blob(py_bytes.as_bytes().to_vec()));
        }

        // Try to extract as int (Python int)
        if let Ok(py_int) = value.cast::<PyInt>() {
            if let Ok(int_val) = py_int.extract::<i64>() {
                return Ok(SqliteParam::Int(int_val));
            }
            // For very large Python ints, convert to string
            // SQLite can handle large integers as text, but we'll keep as int if possible
            return Ok(SqliteParam::Text(py_int.to_string()));
        }

        // Try to extract as float
        if let Ok(py_float) = value.cast::<PyFloat>() {
            if let Ok(float_val) = py_float.extract::<f64>() {
                return Ok(SqliteParam::Real(float_val));
            }
        }

        // Try to extract as string (PyString)
        if let Ok(py_str) = value.cast::<PyString>() {
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
                format!("Missing parameter: {name}"),
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
            format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
            format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
                format!("Too many parameters ({}). Currently supporting up to 16 parameters.", params.len())
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
        // Set default timeout of 30 seconds if not specified
        let timeout = timeout_secs.unwrap_or(30);
        opts = opts.acquire_timeout(Duration::from_secs(timeout));
        let new_pool = opts
            .connect(&format!("sqlite:{path}"))
            .await
            .map_err(|e| {
                OperationalError::new_err(format!(
                    "Failed to connect to database at {path}: {e}"
                ))
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
                .map_err(|e| map_sqlx_error(e, path, &pragma_query))?;
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
            .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection for callbacks: {e}")))?;
        
        *callback_guard = Some(pool_conn);
    }
    Ok(())
}

/// Execute init_hook if it hasn't been called yet.
/// This should be called from the first operation method that uses the pool.
async fn execute_init_hook_if_needed(
    init_hook: &Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_called: &Arc<StdMutex<bool>>,
    connection: Py<Connection>,
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
            let coro = hook_bound.call1((conn_bound,))
                .map_err(|e| OperationalError::new_err(format!("Failed to call init_hook: {e}")))?;
            
            // Convert Python coroutine to Rust future (into_future expects Bound)
            into_future(coro)
                .map_err(|e| OperationalError::new_err(format!("Failed to convert init_hook coroutine to future: {e}")))
        })?;
        
        // Await the future
        coro_future.await
            .map_err(|e| OperationalError::new_err(format!("init_hook raised an exception: {e}")))?;
    }
    
    Ok(())
}

/// Check if any callbacks are currently set.
fn has_callbacks(
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

/// Helper to determine which connection to use for operations.
/// Returns:
/// Map sqlx error to appropriate Python exception.
fn map_sqlx_error(e: sqlx::Error, path: &str, query: &str) -> PyErr {
    use sqlx::Error as SqlxError;

    let error_msg = format!(
        "Failed to execute query on database {path}: {e}\nQuery: {query}"
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

/// Row class for dict-like access to query results (similar to aiosqlite.Row).
#[pyclass]
struct RapRow {
    columns: Vec<String>,
    values: Vec<Py<PyAny>>,
}

#[pymethods]
impl RapRow {
    /// Create a new Row from column names and values.
    #[new]
    fn new(columns: Vec<String>, values: Vec<Py<PyAny>>) -> PyResult<Self> {
        if columns.len() != values.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Columns and values must have the same length"
            ));
        }
        Ok(RapRow { columns, values })
    }

    /// Get item by index or column name.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Try integer index first
        if let Ok(idx) = key.extract::<usize>() {
            if idx >= self.values.len() {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                    format!("Index {idx} out of range")
                ));
            }
            return Ok(self.values[idx].clone_ref(py));
        }
        
        // Try string column name
        if let Ok(col_name) = key.extract::<String>() {
            if let Some(idx) = self.columns.iter().position(|c| c == &col_name) {
                return Ok(self.values[idx].clone_ref(py));
            }
            return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(
                format!("Column '{col_name}' not found")
            ));
        }
        
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "Key must be int or str"
        ))
    }

    /// Get number of columns.
    fn __len__(&self) -> usize {
        self.values.len()
    }

    /// Check if row contains a column.
    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        // Try string column name
        if let Ok(col_name) = key.extract::<String>() {
            return Ok(self.columns.contains(&col_name));
        }
        
        // Try integer index
        if let Ok(idx) = key.extract::<usize>() {
            return Ok(idx < self.values.len());
        }
        
        Ok(false)
    }

    /// Get column names.
    fn keys(&self) -> PyResult<Vec<String>> {
        Ok(self.columns.clone())
    }

    /// Get values.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        Ok(self.values.iter().map(|v| v.clone_ref(py)).collect())
    }

    /// Get items as (column_name, value) pairs.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(String, Py<PyAny>)>> {
        Ok(self.columns.iter().zip(self.values.iter())
            .map(|(col, val)| (col.clone(), val.clone_ref(py)))
            .collect())
    }

    /// Iterate over column names.
    fn __iter__(&self) -> PyResult<Vec<String>> {
        Ok(self.columns.clone())
    }

    /// String representation.
    fn __str__(&self, py: Python<'_>) -> PyResult<String> {
        let items: Vec<String> = self.columns.iter().zip(self.values.iter())
            .map(|(col, val)| {
                let val_str = val.bind(py).repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "?".to_string());
                format!("{col}={val_str}")
            })
            .collect();
        Ok(format!("Row({})", items.join(", ")))
    }

    /// Repr representation.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.__str__(py)
    }
}

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Connection>()?;
    m.add_class::<Cursor>()?;
    m.add_class::<ExecuteContextManager>()?;
    m.add_class::<TransactionContextManager>()?;
    m.add_class::<RapRow>()?;

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
    init_hook_called: Arc<StdMutex<bool>>, // Track if init_hook has been executed
    pool_size: Arc<StdMutex<Option<usize>>>, // Configurable pool size
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
    user_functions: UserFunctions, // name -> (nargs, callback)
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Trace callback
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Authorizer callback
    progress_handler: ProgressHandler, // (n, callback)
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
                    let conn = conn_guard.as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
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
                        ).await?;
                        
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
                        handle.as_raw_handle().as_ptr()
                    } else {
                        // No callbacks - use pool directly with temporary connection
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        let mut temp_conn = pool_clone.acquire().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {e}")))?;
                        let sqlite_conn: &mut SqliteConnection = &mut temp_conn;
                        let mut handle = sqlite_conn.lock_handle().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
                        let handle_ptr = handle.as_raw_handle().as_ptr();
                        
                        // Call sqlite3_total_changes while connection is alive
                        let total = unsafe {
                            sqlite3_total_changes(handle_ptr)
                        };
                        
                        // Connection will be released when temp_conn is dropped
                        drop(handle);
                        drop(temp_conn);
                        
                        return Ok(total as u64);
                    }
                };
                
                // Call sqlite3_total_changes (for transaction or callback connection paths)
                let total = unsafe {
                    sqlite3_total_changes(raw_db)
                };
                
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
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    conn_guard.take()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?
                } else {
                    // Acquire a connection from the pool for the transaction
                    pool_clone.acquire().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {e}")))?
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
                let mut conn = conn_guard.take()
                    .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;

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
                let mut conn = conn_guard.take()
                    .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;

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
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        let result =
                            bind_and_execute_on_connection(&query, param_values, conn, &path).await?;
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
                    ).await?;
                    
                    // Use callback connection for each iteration
                    for param_values in processed_params.iter() {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        let result =
                            bind_and_execute_on_connection(&query, param_values, conn, &path).await?;
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&processed_query, &param_values, conn, &path).await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    
                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_one_on_connection(&processed_query, &param_values, conn, &path).await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    
                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_optional_on_connection(&processed_query, &param_values, conn, &path).await?
                } else if has_callbacks_flag {
                    // Ensure callback connection exists
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    
                    // Use callback connection
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
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
            processed_query: None, // No processed query for cursor() method
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
    fn set_pragma(self_: PyRef<Self>, name: String, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
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
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                // Call the C API
                let enabled_int = if enabled { 1 } else { 0 };
                let result = unsafe {
                    sqlite3_enable_load_extension(raw_db, enabled_int)
                };
                
                if result != 0 {
                    return Err(OperationalError::new_err(
                        format!("Failed to enable/disable load extension: SQLite error code {result}")
                    ));
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
                        "Extension loading is not enabled. Call enable_load_extension(true) first."
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                // Convert extension name to CString
                let name_cstr = CString::new(name.clone())
                    .map_err(|e| OperationalError::new_err(format!("Invalid extension name: {e}")))?;
                
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
                        let cstr = unsafe { CStr::from_ptr(errmsg) };
                        let msg = cstr.to_string_lossy().to_string();
                        unsafe {
                            sqlite3_free(errmsg as *mut std::ffi::c_void);
                        }
                        msg
                    } else {
                        format!("SQLite error code {result}")
                    };
                    return Err(OperationalError::new_err(
                        format!("Failed to load extension '{name}': {error_msg}")
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
                let raw_db = handle.as_raw_handle().as_ptr();
                
                if func_clone.is_none() {
                    // Remove the function from user_functions
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.remove(&name);
                    }
                    
                    // Remove from SQLite by calling sqlite3_create_function_v2 with NULL callback
                    let name_cstr = std::ffi::CString::new(name.clone())
                        .map_err(|e| OperationalError::new_err(format!("Function name contains null byte: {e}")))?;
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
                            format!("Failed to remove function '{name}': SQLite error code {result}")
                        ));
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
                    let callback_for_storage = Python::with_gil(|py| {
                        func_clone.as_ref().unwrap().clone_ref(py)
                    });
                    {
                        let mut funcs_guard = user_functions.lock().unwrap();
                        funcs_guard.insert(name.clone(), (nargs, callback_for_storage));
                    }
                    
                    // Create a boxed callback pointer to pass as user data
                    let name_cstr = std::ffi::CString::new(name.clone())
                        .map_err(|e| OperationalError::new_err(format!("Function name contains null byte: {e}")))?;
                    
                    // Store the Python callback in a Box and pass it as user_data
                    // Clone it with GIL
                    // Note: Python::with_gil is used here for sync callback access in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
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
                            // Note: Python::with_gil is used here for sync callback execution in async context.
                            // The deprecation warning is acceptable as this is a sync operation within async.
                            #[allow(deprecated)]
                            // Note: Python::with_gil is used here for sync operation in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::with_gil(|py| {
                                // Clone the callback to use it (the original stays in the Box)
                                let callback = (*callback_ptr).clone_ref(py);
                                
                                let mut py_args = Vec::new();
                                for i in 0..argc {
                                    let value_ptr = *argv.add(i as usize);
                                    match sqlite_c_value_to_py(py, value_ptr) {
                                        Ok(py_val) => {
                                            py_args.push(py_val);
                                        }
                                        Err(e) => {
                                            // On error, set SQLite error and return
                                            let error_msg = format!("Error converting argument {i}: {e}");
                                            libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
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
                                        callback.bind(py).call1((py_args[0].clone_ref(py), py_args[1].clone_ref(py)))
                                    }
                                    3 => {
                                        // Three arguments
                                        callback.bind(py).call1((py_args[0].clone_ref(py), py_args[1].clone_ref(py), py_args[2].clone_ref(py)))
                                    }
                                    4 => {
                                        // Four arguments
                                        callback.bind(py).call1((py_args[0].clone_ref(py), py_args[1].clone_ref(py), py_args[2].clone_ref(py), py_args[3].clone_ref(py)))
                                    }
                                    5 => {
                                        // Five arguments
                                        callback.bind(py).call1((py_args[0].clone_ref(py), py_args[1].clone_ref(py), py_args[2].clone_ref(py), py_args[3].clone_ref(py), py_args[4].clone_ref(py)))
                                    }
                                    _ => {
                                        // For more than 5 arguments, use Python's unpacking
                                        // Create a helper function that unpacks the tuple
                                        let args_tuple = match PyTuple::new(py, py_args.iter().map(|arg| arg.clone_ref(py))) {
                                            Ok(t) => t,
                                            Err(e) => {
                                                let error_msg = format!("Error creating argument tuple: {e}");
                                                libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
                                                return;
                                            }
                                        };
                                        // Use Python code to unpack: lambda f, args: f(*args)
                                        let code_str = match std::ffi::CString::new("lambda f, args: f(*args)") {
                                            Ok(s) => s,
                                            Err(_) => {
                                                libsqlite3_sys::sqlite3_result_error(ctx, c"Error creating CString".as_ptr(), 22);
                                                return;
                                            }
                                        };
                                        let unpack_code = match py.eval(code_str.as_c_str(), None, None) {
                                            Ok(code) => code,
                                            Err(e) => {
                                                let error_msg = format!("Error creating unpack helper: {e}");
                                                libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
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
                                                let error_msg = format!("Error converting result: {e}");
                                                libsqlite3_sys::sqlite3_result_error(ctx, error_msg.as_ptr() as *const i8, error_msg.len() as i32);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Python exception - convert to SQLite error
                                        let error_msg = format!("Python function error: {e}");
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
                            format!("Failed to create function '{name}': SQLite error code {result}")
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
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
                        format!("Failed to set trace callback: SQLite error code {result}")
                    ));
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
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
                        
                        // Note: Python::with_gil is used here for sync operation in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
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
                                    result.extract::<i32>().unwrap_or(SQLITE_OK) // Default to OK if conversion fails
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
                ).await?;
                
                // Get the callback connection and access raw handle
                let mut conn_guard = callback_connection.lock().await;
                let conn = conn_guard.as_mut()
                    .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await
                    .map_err(|e| OperationalError::new_err(format!("Failed to lock handle: {e}")))?;
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
                    bytes.iter()
                        .map(|b| format!("{b:02x}"))
                        .collect()
                }

                // Get connection for queries
                // We need to handle different connection types
                let mut statements = Vec::new();
                statements.push("BEGIN TRANSACTION;".to_string());

                // Query sqlite_master - use appropriate connection
                let schema_rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
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
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
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
                let escape_sql_string = |s: &str| -> String {
                    s.replace("'", "''")
                };

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
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        sqlx::query(&query)
                            .fetch_all(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query))?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
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
                            rows[0].columns().get(i).map(|c| c.name().to_string()).unwrap_or_else(|| format!("column_{i}"))
                        })
                        .collect();

                    // Generate INSERT statements
                    for row in rows {
                        let mut values = Vec::new();
                        for i in 0..column_count {
                            values.push(format_value(&row, i));
                        }
                        let values_str = values.join(", ");
                        statements.push(format!("INSERT INTO {} ({}) VALUES ({});", 
                            table_name, 
                            column_names.join(", "), 
                            values_str));
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
    fn get_tables(
        self_: PyRef<Self>,
        name: Option<String>,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_table_info(
        self_: PyRef<Self>,
        table_name: String,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
                        let dflt_val: Py<PyAny> = if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
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
    fn get_indexes(
        self_: PyRef<Self>,
        table_name: Option<String>,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_foreign_keys(
        self_: PyRef<Self>,
        table_name: String,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_schema(
        self_: PyRef<Self>,
        table_name: Option<String>,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&tables_query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&tables_query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
                    let info_query = format!("PRAGMA table_info('{}')", tbl_name.replace("'", "''"));
                    let info_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_fetch_all_on_connection(&info_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_fetch_all_on_connection(&info_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        bind_and_fetch_all(&info_query, &[], &pool_clone, &path).await?
                    };

                    // Get indexes
                    let indexes_query = format!("SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND tbl_name = '{}' AND name NOT LIKE 'sqlite_%' ORDER BY name", tbl_name.replace("'", "''"));
                    let indexes_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_fetch_all_on_connection(&indexes_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_fetch_all_on_connection(&indexes_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        bind_and_fetch_all(&indexes_query, &[], &pool_clone, &path).await?
                    };

                    // Get foreign keys
                    let fk_query = format!("PRAGMA foreign_key_list('{}')", tbl_name.replace("'", "''"));
                    let fk_rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_fetch_all_on_connection(&fk_query, &[], conn, &path).await?
                    } else if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_fetch_all_on_connection(&fk_query, &[], conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
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
                                let dflt_val: Py<PyAny> = if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
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
                                let unique = if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                                    if sql.to_uppercase().contains("UNIQUE") { 1 } else { 0 }
                                } else { 0 };
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
    fn get_views(
        self_: PyRef<Self>,
        name: Option<String>,
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
                    format!("SELECT name FROM sqlite_master WHERE type='view' AND name = '{}'", view_name.replace("'", "''"))
                } else {
                    "SELECT name FROM sqlite_master WHERE type='view' ORDER BY name".to_string()
                };

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_index_list(
        self_: PyRef<Self>,
        table_name: String,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_index_info(
        self_: PyRef<Self>,
        index_name: String,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
    fn get_table_xinfo(
        self_: PyRef<Self>,
        table_name: String,
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else if has_callbacks_flag {
                    ensure_callback_connection(
                        &path,
                        &pool,
                        &callback_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    bind_and_fetch_all_on_connection(&query, &[], conn, &path).await?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
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
                        let dflt_val: Py<PyAny> = if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
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
            let (target_path_opt, target_pool_opt, target_pragmas_opt, target_pool_size_opt,
                 target_connection_timeout_secs_opt, target_transaction_state_opt,
                 target_transaction_connection_opt, target_callback_connection_opt,
                 target_load_extension_enabled_opt, target_user_functions_opt,
                 target_trace_callback_opt, target_authorizer_callback_opt,
                 target_progress_handler_opt) = if target_is_rapsqlite {
                let target_conn = target_clone.bind(py).cast::<Connection>()
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
                (None, None, None, None, None, None, None, None, None, None, None, None, None)
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
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to lock source handle: {e}")))?;
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
                    ).await?;
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard
                        .as_mut()
                        .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to lock source handle: {e}")))?;
                    source_handle = SendPtr(handle.as_raw_handle().as_ptr());
                    _source_pool_conn = None;
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    ).await?;
                    let mut pool_conn = pool_clone.acquire().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to acquire source connection: {e}")))?;
                    let sqlite_conn: &mut SqliteConnection = &mut pool_conn;
                    let mut handle = sqlite_conn.lock_handle().await
                        .map_err(|e| OperationalError::new_err(format!("Failed to lock source handle: {e}")))?;
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
                    let target_pragmas: Arc<StdMutex<Vec<(String, String)>>> = target_pragmas_opt.unwrap();
                    let target_pool_size: Arc<StdMutex<Option<usize>>> = target_pool_size_opt.unwrap();
                    let target_connection_timeout_secs: Arc<StdMutex<Option<u64>>> = target_connection_timeout_secs_opt.unwrap();
                    let target_transaction_state: Arc<Mutex<TransactionState>> = target_transaction_state_opt.unwrap();
                    let target_transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>> = target_transaction_connection_opt.unwrap();
                    let target_callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>> = target_callback_connection_opt.unwrap();
                    let target_load_extension_enabled: Arc<StdMutex<bool>> = target_load_extension_enabled_opt.unwrap();
                    let target_user_functions: UserFunctions = target_user_functions_opt.unwrap();
                    let target_trace_callback: Arc<StdMutex<Option<Py<PyAny>>>> = target_trace_callback_opt.unwrap();
                    let target_authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>> = target_authorizer_callback_opt.unwrap();
                    let target_progress_handler: ProgressHandler = target_progress_handler_opt.unwrap();

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
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Target transaction connection not available"))?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to lock target handle: {e}")))?;
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
                        ).await?;
                        let mut conn_guard = target_callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Target callback connection not available"))?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to lock target handle: {e}")))?;
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
                        ).await?;
                        let mut target_pool_conn = target_pool_clone.acquire().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to acquire target connection: {e}")))?;
                        let sqlite_conn: &mut SqliteConnection = &mut target_pool_conn;
                        let mut handle = sqlite_conn.lock_handle().await
                            .map_err(|e| OperationalError::new_err(format!("Failed to lock target handle: {e}")))?;
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
                        let get_handle = backup_helper.getattr("get_sqlite3_handle")
                            .map_err(|e| OperationalError::new_err(format!(
                                "Failed to get get_sqlite3_handle function: {e}"
                            )))?;
                        
                        // Call the function with the target connection
                        let conn_obj = target_clone.bind(py);
                        let result = get_handle.call1((conn_obj,))
                            .map_err(|e| OperationalError::new_err(format!(
                                "Failed to extract sqlite3* handle: {e}"
                            )))?;
                        
                        // Check if extraction succeeded (returns None on failure)
                        if result.is_none() {
                            return Err(OperationalError::new_err(
                                "Could not extract sqlite3* handle from target connection. \
                                Target must be a rapsqlite.Connection or sqlite3.Connection. \
                                The connection may be closed or invalid."
                            ));
                        }
                        
                        // Extract the pointer value as usize
                        let ptr_val: usize = result.extract()
                            .map_err(|e| OperationalError::new_err(format!(
                                "Failed to extract pointer value: {e}"
                            )))?;
                        
                        // Verify pointer is not null
                        if ptr_val == 0 {
                            return Err(OperationalError::new_err(
                                "Extracted sqlite3* handle is null. Connection may be closed."
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
                        "Source sqlite3* handle is null. Connection may be closed or invalid."
                    ));
                }
                
                // Check SQLite library version compatibility
                let source_libversion = unsafe {
                    std::ffi::CStr::from_ptr(sqlite3_libversion())
                        .to_string_lossy()
                        .to_string()
                };
                // Note: Both should use same library, but verify for debugging
                
                // Convert database name to C string
                let name_cstr = std::ffi::CString::new(name.clone())
                    .map_err(|e| OperationalError::new_err(format!("Invalid database name: {e}")))?;

                // Check connection states before backup
                // SQLite backup requires destination to not have active transactions
                // Verify both source and target are in valid states
                let target_has_transaction = unsafe {
                    sqlite3_get_autocommit(target_handle.0) == 0
                };
                if target_has_transaction {
                    return Err(OperationalError::new_err(
                        "Cannot backup: target connection has an active transaction. \
                        Commit or rollback the transaction before backup."
                    ));
                }
                
                // Verify source connection is also in valid state
                // Source can have transactions, but should be open and valid
                let _source_autocommit = unsafe {
                    sqlite3_get_autocommit(source_handle.0)
                };
                // Both 0 (in transaction) and non-zero (autocommit) are valid for source
                // This check just verifies the handle is valid and connection is open

                // Initialize backup
                let backup_handle = SendPtr(unsafe {
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
                            std::ffi::CStr::from_ptr(msg_ptr)
                                .to_string_lossy()
                                .to_string()
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
                    let result = unsafe {
                        sqlite3_backup_step(backup_handle.0, pages_to_copy)
                    };

                    match result {
                        SQLITE_OK | SQLITE_BUSY | SQLITE_LOCKED => {
                            // Progress - call progress callback if provided
                            if let Some(ref progress_cb) = progress_callback {
                                let remaining = unsafe { sqlite3_backup_remaining(backup_handle.0) };
                                let page_count = unsafe { sqlite3_backup_pagecount(backup_handle.0) };
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
                                    let remaining_py: Py<PyAny> = PyInt::new(py, remaining as i64).into_any().unbind();
                                    let page_count_py: Py<PyAny> = PyInt::new(py, page_count as i64).into_any().unbind();
                                    let pages_copied_py: Py<PyAny> = PyInt::new(py, pages_copied as i64).into_any().unbind();
                                    if let Ok(args) = PyTuple::new(py, &[remaining_py, page_count_py, pages_copied_py]) {
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

/// Execute context manager returned by `Connection::execute()`.
/// Allows `async with db.execute(...)` pattern by being both awaitable and an async context manager.
#[pyclass]
struct ExecuteContextManager {
    cursor: Py<Cursor>,
    query: String,
    param_values: Vec<SqliteParam>,
    is_select: bool,
    // Connection state needed for execution
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    load_extension_enabled: Arc<StdMutex<bool>>,
    user_functions: UserFunctions,
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: ProgressHandler,
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_called: Arc<StdMutex<bool>>,
    last_rowid: Arc<Mutex<i64>>,
    last_changes: Arc<Mutex<u64>>,
    connection: Py<Connection>,
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
            let cursor = slf.borrow(py).cursor.clone_ref(py);
            // Get cursor's results Arc to mark it as executed for non-SELECT queries
            // Note: Python::with_gil is used here for sync result caching in async context.
            // The deprecation warning is acceptable as this is a sync operation within async.
            #[allow(deprecated)]
            let _cursor_results = Python::with_gil(|_py| -> PyResult<Arc<StdMutex<Option<Vec<sqlx::sqlite::SqliteRow>>>>> {
                // We can't easily get the results Arc from Py<Cursor>
                // Instead, we'll handle this in fetchall() by checking if it's non-SELECT
                // For now, we'll pass None and handle it in fetchall()
                Ok(Arc::new(StdMutex::new(None))) // Placeholder - won't be used
            }).unwrap_or_else(|_| Arc::new(StdMutex::new(None)));
            
            let future = async move {
                // For non-SELECT queries, execute immediately when entering context
                if !is_select {
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
                        )
                        .await?;
                    }
                    
                    // Only call init_hook if not already called (avoid re-entry during init_hook execution)
                    // This prevents deadlocks when init_hook calls conn.execute() which triggers __aenter__
                    execute_init_hook_if_needed(&init_hook, &init_hook_called, connection).await?;
                    
                    let has_callbacks_flag = has_callbacks(
                        &load_extension_enabled,
                        &user_functions,
                        &trace_callback,
                        &authorizer_callback,
                        &progress_handler,
                    );

                    let result = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_execute_on_connection(&query, &param_values, conn, &path)
                            .await?
                    } else if has_callbacks_flag {
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_execute_on_connection(&query, &param_values, conn, &path)
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
                        bind_and_execute(&query, &param_values, &pool_clone, &path)
                            .await?
                    };

                    let rowid = result.last_insert_rowid();
                    let changes = result.rows_affected();

                    *last_rowid.lock().await = rowid;
                    *last_changes.lock().await = changes;
                    
                    // Mark cursor results as cached (empty for non-SELECT) to prevent re-execution
                    // The fetchall() method will check if it's non-SELECT and results are None,
                    // and return empty results without executing. This is handled in fetchall().
                } else {
                    // For SELECT queries, ensure pool exists for lazy execution
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
                        )
                        .await?;
                    }
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
struct TransactionContextManager {
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    connection: Py<Connection>,
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>, // Optional initialization hook
    init_hook_called: Arc<StdMutex<bool>>, // Track if init_hook has been executed
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
            let init_hook = Arc::clone(&slf.borrow(py).init_hook);
            let init_hook_called = Arc::clone(&slf.borrow(py).init_hook_called);
            let future = async move {
                // Check transaction state and release lock before calling init_hook
                // This prevents deadlock when init_hook calls conn.execute() which needs to check transaction state
                {
                    let trans_guard = transaction_state.lock().await;
                    if *trans_guard == TransactionState::Active {
                        return Err(OperationalError::new_err("Transaction already in progress"));
                    }
                } // Lock released here

                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                )
                .await?;

                // Execute init_hook if needed (before starting transaction)
                // Clone connection before passing to async function
                // Lock is released, so init_hook's execute() can check transaction state without deadlock
                // Note: Python::with_gil is used here for sync clone_ref in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
                let connection_for_hook = Python::with_gil(|py| connection.clone_ref(py));
                execute_init_hook_if_needed(&init_hook, &init_hook_called, connection_for_hook).await?;
                let mut conn = pool_clone
                    .acquire()
                    .await
                    .map_err(|e| OperationalError::new_err(format!("Failed to acquire connection: {e}")))?;
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
                // Re-acquire lock to set transaction state
                {
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::Active;
                }
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
    // Store processed query and parameters to avoid re-processing (fixes parameterized query issue)
    processed_query: Option<String>,
    processed_params: Option<Vec<SqliteParam>>,
    connection_path: String, // Store path for direct pool access
    connection_pool: Arc<Mutex<Option<SqlitePool>>>, // Reference to connection's pool
    connection_pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Reference to connection's pragmas
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // Connection's row_factory at cursor creation
    // Transaction and callback state for proper connection priority
    transaction_state: Arc<Mutex<TransactionState>>,
    transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    load_extension_enabled: Arc<StdMutex<bool>>,
    user_functions: UserFunctions,
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: ProgressHandler,
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
        let params_for_storage = parameters.map(|params| params.clone().unbind());
        
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
        let stored_proc_query_fetchone = self.processed_query.clone();
        let stored_proc_params_fetchone = self.processed_params.clone();
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let row_factory = Arc::clone(&self.row_factory);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        
        Python::attach(|py| {
            let future = async move {
                // Ensure results are cached (same logic as fetchmany)
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    // Use stored processed parameters if available, otherwise re-process
                    let (processed_query, processed_params) = if let (Some(proc_query), Some(proc_params)) = (stored_proc_query_fetchone, stored_proc_params_fetchone) {
                        (proc_query, proc_params)
                    } else {
                        // Fallback: re-process parameters
                        // Note: Python::with_gil is used here for sync parameter processing in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| -> PyResult<(String, Vec<SqliteParam>)> {
                            let params_guard = parameters.lock().unwrap();
                            if let Some(ref params_py) = *params_guard {
                                let params_bound = params_py.bind(py);
                                if let Ok(dict) = params_bound.cast::<pyo3::types::PyDict>() {
                                    let (proc_query, param_values) = process_named_parameters(&query, dict)?;
                                    return Ok((proc_query, param_values));
                                }
                                if let Ok(list) = params_bound.cast::<PyList>() {
                                    let param_values = process_positional_parameters(list)?;
                                    return Ok((query.clone(), param_values));
                                }
                                let param = SqliteParam::from_py(params_bound)?;
                                return Ok((query.clone(), vec![param]));
                            }
                            Ok((query.clone(), Vec::new()))
                        })?
                    };
                    
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
                    
                    let rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                    } else if has_callbacks_flag {
                        // Ensure callback connection exists
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        
                        // Use callback connection
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&processed_query, &processed_params, &pool_clone, &path).await?
                    };
                    
                    // Note: Python::with_gil is used here for sync result caching in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
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
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
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
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        
        // Check if this is a non-SELECT query - if so and results are None,
        // it means the query was already executed in __aenter__ and we should
        // just return empty results without executing again
        let is_select = is_select_query(&query);
        if !is_select {
            let results_guard = results.lock().unwrap();
            if results_guard.is_none() {
                // Non-SELECT query already executed in __aenter__, return empty results
                // Mark as executed to prevent re-execution
                drop(results_guard);
                *results.lock().unwrap() = Some(Vec::new());
                // Return an awaitable future (empty list for non-SELECT queries)
                return Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let future = async move {
                        Python::attach(|py| -> PyResult<Py<PyAny>> {
                            Ok(PyList::empty(py).into())
                        })
                    };
                    future_into_py(py, future).map(|bound| bound.unbind())
                });
            }
        }
        
        // Clone processed parameters for use in async future
        let stored_proc_query = self.processed_query.clone();
        let stored_proc_params = self.processed_params.clone();
        
        Python::attach(|py| {
            let future = async move {
                // Ensure results are cached
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    // Check if this is a non-SELECT query - if so, it was already executed in __aenter__
                    // and we should just mark results as empty
                    let is_select = is_select_query(&query);
                    if !is_select {
                        // Non-SELECT query already executed in __aenter__, mark as empty
                        let mut results_guard = results.lock().unwrap();
                        *results_guard = Some(Vec::new());
                    } else {
                        // SELECT query - fetch results
                        // Use stored processed parameters if available (from Connection.execute()), otherwise re-process
                        let (processed_query, processed_params) = if let (Some(proc_query), Some(proc_params)) = (stored_proc_query, stored_proc_params) {
                            // Use stored processed parameters - these are already in the correct order
                            // and match the ? placeholders in processed_query
                            // The parameters were processed by process_named_parameters() which ensures
                            // correct order matching the ? placeholders
                            (proc_query, proc_params)
                        } else {
                            // Fallback: re-process parameters (for cursors created via cursor() method)
                            // Note: Python::with_gil is used here for sync parameter processing in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| -> PyResult<(String, Vec<SqliteParam>)> {
                                let params_guard = parameters.lock().unwrap();
                                if let Some(ref params_py) = *params_guard {
                                    let params_bound = params_py.bind(py);
                                    
                                    // Try dict first (named parameters)
                                    if let Ok(dict) = params_bound.cast::<pyo3::types::PyDict>() {
                                        let (proc_query, param_values) = process_named_parameters(&query, dict)?;
                                        // Verify we got parameters if query contains named placeholders
                                        if param_values.is_empty() && (query.contains(':') || query.contains('@') || query.contains('$')) {
                                            return Err(ProgrammingError::new_err(
                                                format!("Named parameters found in query but none extracted. Query: '{query}', Processed: '{proc_query}'")
                                            ));
                                        }
                                        // Additional verification: check if processed query has ? placeholders
                                        if !proc_query.contains('?') && query.contains(':') {
                                            return Err(ProgrammingError::new_err(
                                                format!("Query had named parameters but processed query has no ? placeholders. Original: '{query}', Processed: '{proc_query}'")
                                            ));
                                        }
                                        return Ok((proc_query, param_values));
                                    }
                                    
                                    // Try list (positional parameters)
                                    if let Ok(list) = params_bound.cast::<PyList>() {
                                        let param_values = process_positional_parameters(list)?;
                                        return Ok((query.clone(), param_values));
                                    }
                                    
                                    // Single value
                                    let param = SqliteParam::from_py(params_bound)?;
                                    return Ok((query.clone(), vec![param]));
                                }
                                Ok((query.clone(), Vec::new()))
                            })?
                        };
                        
                        // Priority: transaction > callbacks > pool
                        // Check transaction state - must check inside async future to get current state
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
                        
                        let rows = if in_transaction {
                            // Use transaction connection - it's already acquired and holds the transaction
                            let mut conn_guard = transaction_connection.lock().await;
                            let conn = conn_guard
                                .as_mut()
                                .ok_or_else(|| OperationalError::new_err(
                                    "Transaction is active but transaction_connection is None. This indicates a bug in transaction management.".to_string()
                                ))?;
                            bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                        } else if has_callbacks_flag {
                            // Ensure callback connection exists
                            ensure_callback_connection(
                                &path,
                                &pool,
                                &callback_connection,
                                &pragmas,
                                &pool_size,
                                &connection_timeout_secs,
                            ).await?;
                            
                            // Use callback connection
                            let mut conn_guard = callback_connection.lock().await;
                            let conn = conn_guard
                                .as_mut()
                                .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                            bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                        } else {
                            let pool_clone = get_or_create_pool(
                                &path,
                                &pool,
                                &pragmas,
                                &pool_size,
                                &connection_timeout_secs,
                            )
                            .await?;
                            bind_and_fetch_all(&processed_query, &processed_params, &pool_clone, &path).await?
                        };
                        
                        // Note: Python::with_gil is used here for sync result caching in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
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
                }
                
                // Return all remaining results
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
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
        let stored_proc_query_fetchmany = self.processed_query.clone();
        let stored_proc_params_fetchmany = self.processed_params.clone();
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let row_factory = Arc::clone(&self.row_factory);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        
        Python::attach(|py| {
            let future = async move {
                // Check if results need to be fetched
                let needs_fetch = {
                    let results_guard = results.lock().unwrap();
                    results_guard.is_none()
                };
                
                if needs_fetch {
                    // Use stored processed parameters if available, otherwise re-process
                    let (processed_query, processed_params) = if let (Some(proc_query), Some(proc_params)) = (stored_proc_query_fetchmany, stored_proc_params_fetchmany) {
                        (proc_query, proc_params)
                    } else {
                        // Fallback: re-process parameters
                        // Note: Python::with_gil is used here for sync parameter processing in async context.
                        // The deprecation warning is acceptable as this is a sync operation within async.
                        #[allow(deprecated)]
                        Python::with_gil(|py| -> PyResult<(String, Vec<SqliteParam>)> {
                            let params_guard = parameters.lock().unwrap();
                            if let Some(ref params_py) = *params_guard {
                                let params_bound = params_py.bind(py);
                                
                                // Check if it's a dict (named parameters)
                                if let Ok(dict) = params_bound.cast::<pyo3::types::PyDict>() {
                                    let (proc_query, param_values) = process_named_parameters(&query, dict)?;
                                    return Ok((proc_query, param_values));
                                }
                                
                                // Check if it's a list (positional parameters)
                                if let Ok(list) = params_bound.cast::<PyList>() {
                                    let param_values = process_positional_parameters(list)?;
                                    return Ok((query.clone(), param_values));
                                }
                                
                                // Single value
                                let param = SqliteParam::from_py(params_bound)?;
                                return Ok((query.clone(), vec![param]));
                            }
                            Ok((query.clone(), Vec::new()))
                        })?
                    };
                    
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
                    
                    let rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                    } else if has_callbacks_flag {
                        // Ensure callback connection exists
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        
                        // Use callback connection
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard
                            .as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_fetch_all_on_connection(&processed_query, &processed_params, conn, &path).await?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_fetch_all(&processed_query, &processed_params, &pool_clone, &path).await?
                    };
                    
                    // Cache results as Python objects
                    // Note: Python::with_gil is used here for sync result caching in async context.
                    // The deprecation warning is acceptable as this is a sync operation within async.
                    #[allow(deprecated)]
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
                // Note: Python::with_gil is used here for sync context manager creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
                // Note: Python::with_gil is used here for sync result conversion in async context.
                // The deprecation warning is acceptable as this is a sync operation within async.
                #[allow(deprecated)]
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

    /// Execute a script containing multiple SQL statements separated by semicolons.
    fn executescript(&self, script: String) -> PyResult<Py<PyAny>> {
        let path = self.connection_path.clone();
        let pool = Arc::clone(&self.connection_pool);
        let pragmas = Arc::clone(&self.connection_pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let load_extension_enabled = Arc::clone(&self.load_extension_enabled);
        let user_functions = Arc::clone(&self.user_functions);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        
        Python::attach(|py| {
            let future = async move {
                // Parse script into individual statements
                // Simple approach: split by semicolon, but be careful about semicolons in strings
                // For now, use a simple split - more sophisticated parsing can be added later
                let statements: Vec<String> = script
                    .split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                
                if statements.is_empty() {
                    return Ok(());
                }
                
                // Check transaction state and callback flags
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
                
                // Execute each statement sequentially
                for statement in statements {
                    if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.as_mut()
                            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
                        bind_and_execute_on_connection(&statement, &[], conn, &path).await?;
                    } else if has_callbacks_flag {
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        ).await?;
                        
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut()
                            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
                        bind_and_execute_on_connection(&statement, &[], conn, &path).await?;
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        bind_and_execute(&statement, &[], &pool_clone, &path).await?;
                    }
                }
                
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Async iterator entry point.
    fn __aiter__(slf: PyRef<Self>) -> PyResult<Py<Self>> {
        Ok(slf.into())
    }

    /// Async iterator next item.
    fn __anext__(&self) -> PyResult<Py<PyAny>> {
        let results = Arc::clone(&self.results);
        let current_index = Arc::clone(&self.current_index);
        
        Python::attach(|py| {
            // Get the row value while holding GIL
            let row_opt = {
                let results_guard = results.lock().unwrap();
                let results_opt = results_guard.as_ref();
                
                if results_opt.is_none() {
                    return Err(ProgrammingError::new_err("Cursor not executed. Call execute() first."));
                }
                
                let results_vec = results_opt.unwrap();
                let mut index_guard = current_index.lock().unwrap();
                
                if *index_guard >= results_vec.len() {
                    // End of iteration - raise StopAsyncIteration
                    return Err(PyErr::new::<pyo3::exceptions::PyStopAsyncIteration, _>(""));
                }
                
                let row = results_vec[*index_guard].clone_ref(py);
                *index_guard += 1;
                Some(row)
            };
            
            if let Some(row) = row_opt {
                Ok(row)
            } else {
                Err(PyErr::new::<pyo3::exceptions::PyStopAsyncIteration, _>(""))
            }
        })
    }
}
