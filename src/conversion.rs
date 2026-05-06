//! SQLite <-> Python value conversions and row factory handling.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use sqlx::{Column, Row, TypeInfo};

use crate::types::Converters;

// libsqlite3-sys for raw SQLite C API access
use libsqlite3_sys::{sqlite3_context, sqlite3_value};

/// Convert a SQLite C API value (sqlite3_value*) to Python object.
/// This is used in callback trampolines for user-defined functions.
pub(crate) unsafe fn sqlite_c_value_to_py<'py>(
    py: Python<'py>,
    value: *mut sqlite3_value,
) -> PyResult<Py<PyAny>> {
    use libsqlite3_sys::{
        sqlite3_value_blob, sqlite3_value_bytes, sqlite3_value_double, sqlite3_value_int64,
        sqlite3_value_text, sqlite3_value_type, SQLITE_BLOB, SQLITE_FLOAT, SQLITE_INTEGER,
        SQLITE_NULL, SQLITE_TEXT,
    };

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
                let text_str = std::str::from_utf8(text_slice).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid UTF-8 in SQLite text value: {e}"
                    ))
                })?;
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
pub(crate) unsafe fn py_to_sqlite_c_result(
    _py: Python<'_>,
    ctx: *mut sqlite3_context,
    result: &Bound<'_, PyAny>,
) -> PyResult<()> {
    use libsqlite3_sys::{
        sqlite3_result_blob, sqlite3_result_double, sqlite3_result_int64, sqlite3_result_null,
        sqlite3_result_text,
    };

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
        let c_str = std::ffi::CString::new(str_val).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "String contains null byte: {e}"
            ))
        })?;
        let ptr = c_str.as_ptr();
        let len = c_str.as_bytes().len() as i32;
        // Use SQLITE_TRANSIENT so SQLite copies the string before this function returns.
        // After sqlite3_result_text returns, c_str can be safely dropped.
        sqlite3_result_text(ctx, ptr, len, libsqlite3_sys::SQLITE_TRANSIENT());
        return Ok(());
    }

    // Try to extract as bytes
    if let Ok(bytes_val) = result.extract::<Vec<u8>>() {
        let len = bytes_val.len() as i32;
        let ptr = bytes_val.as_ptr();
        sqlite3_result_blob(
            ctx,
            ptr as *const std::ffi::c_void,
            len,
            libsqlite3_sys::SQLITE_TRANSIENT(),
        );
        return Ok(());
    }

    // Try PyBytes
    if let Ok(py_bytes) = result.cast::<PyBytes>() {
        let bytes = py_bytes.as_bytes();
        let len = bytes.len() as i32;
        let ptr = bytes.as_ptr();
        sqlite3_result_blob(
            ctx,
            ptr as *const std::ffi::c_void,
            len,
            libsqlite3_sys::SQLITE_TRANSIENT(),
        );
        return Ok(());
    }

    // Try PyString
    if let Ok(py_str) = result.cast::<PyString>() {
        let str_val = py_str.to_str()?;
        let c_str = std::ffi::CString::new(str_val).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "String contains null byte: {e}"
            ))
        })?;
        let ptr = c_str.as_ptr();
        let len = c_str.as_bytes().len() as i32;
        sqlite3_result_text(ctx, ptr, len, libsqlite3_sys::SQLITE_TRANSIENT());
        return Ok(());
    }

    // Default: return NULL
    sqlite3_result_null(ctx);
    Ok(())
}

/// Get column value as Python bytes for register_converter (callable receives bytes).
fn get_column_bytes<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    col: usize,
) -> PyResult<Option<Bound<'py, PyBytes>>> {
    use sqlx::Row;
    if let Ok(Some(v)) = row.try_get::<Option<Vec<u8>>, _>(col) {
        return Ok(Some(PyBytes::new(py, &v)));
    }
    if let Ok(Some(v)) = row.try_get::<Option<String>, _>(col) {
        return Ok(Some(PyBytes::new(py, v.as_bytes())));
    }
    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(col) {
        let b = v.to_string().into_bytes();
        return Ok(Some(PyBytes::new(py, &b)));
    }
    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(col) {
        let b = v.to_string().into_bytes();
        return Ok(Some(PyBytes::new(py, &b)));
    }
    Ok(None)
}

/// Convert a SQLite value from sqlx Row to Python object.
/// If converters is set and has a converter for the column's declared type, that converter(bytes) is used.
pub(crate) fn sqlite_value_to_py<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    col: usize,
    text_factory: Option<&Py<PyAny>>,
    converters: Option<&Converters>,
) -> PyResult<Py<PyAny>> {
    use sqlx::{Column, Row, TypeInfo};

    let type_name = row.columns()[col].type_info().name().to_ascii_uppercase();

    // register_converter: if we have a converter for this declared type, call it with bytes
    if let Some(conv) = converters {
        let callable = {
            let guard = conv.lock().unwrap();
            guard.get(&type_name).map(|c| c.clone_ref(py))
        };
        if let Some(callable) = callable {
            let callable_bound = callable.bind(py);
            let bytes_py = get_column_bytes(py, row, col)?;
            return Ok(match bytes_py {
                Some(b) => callable_bound.call1((b,))?.unbind(),
                None => py.None(),
            });
        }
    }

    // Apply `text_factory` only for declared TEXT columns (aiosqlite/sqlite3 semantics).
    if let Some(tf) = text_factory {
        let tf_bound = tf.bind(py);
        if !tf_bound.is_none() && type_name == "TEXT" {
            // Prefer String decoding (sqlx already decodes TEXT as UTF-8).
            // We pass bytes to the text_factory, matching sqlite3's callable(bytes)->Any behavior.
            if let Ok(opt_val) = row.try_get::<Option<String>, _>(col) {
                return Ok(match opt_val {
                    Some(val) => {
                        let arg = PyBytes::new(py, val.as_bytes());
                        tf_bound.call1((arg,))?.unbind()
                    }
                    None => py.None(),
                });
            }
        }
    }

    // Fallback path: use column type information to reduce redundant probes.
    // Check declared type first, then fall back to type probing for robustness.

    // Try type-specific extraction based on declared type (more efficient)
    match type_name.as_str() {
        "INTEGER" | "INT" => {
            if let Ok(opt_val) = row.try_get::<Option<i64>, _>(col) {
                return Ok(match opt_val {
                    Some(val) => PyInt::new(py, val).into(),
                    None => py.None(),
                });
            }
        }
        "REAL" | "FLOAT" | "DOUBLE" => {
            if let Ok(opt_val) = row.try_get::<Option<f64>, _>(col) {
                return Ok(match opt_val {
                    Some(val) => PyFloat::new(py, val).into(),
                    None => py.None(),
                });
            }
        }
        "TEXT" | "VARCHAR" | "CHAR" => {
            if let Ok(opt_val) = row.try_get::<Option<String>, _>(col) {
                return Ok(match opt_val {
                    Some(val) => PyString::new(py, &val).into(),
                    None => py.None(),
                });
            }
        }
        "BLOB" => {
            if let Ok(opt_val) = row.try_get::<Option<Vec<u8>>, _>(col) {
                return Ok(match opt_val {
                    Some(val) => PyBytes::new(py, &val).into(),
                    None => py.None(),
                });
            }
        }
        _ => {
            // Unknown or NULL type - fall through to type probing below
        }
    }

    // Type probing fallback (for NULL, unknown types, or when declared type doesn't match)
    // This handles SQLite's dynamic typing where any column can store any type
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

    Ok(py.None())
}

/// Number of columns to iterate (avoids index-out-of-bounds when row metadata
/// and actual row data disagree, e.g. with shared pool connection reuse).
fn row_column_count(row: &sqlx::sqlite::SqliteRow) -> usize {
    std::cmp::min(row.len(), row.columns().len())
}

/// Convert a SQLite row to Python list.
pub(crate) fn row_to_py_list<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    text_factory: Option<&Py<PyAny>>,
    converters: Option<&Converters>,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    let n = row_column_count(row);
    for i in 0..n {
        let val = sqlite_value_to_py(py, row, i, text_factory, converters)?;
        list.append(val)?;
    }
    Ok(list)
}

/// Convert a SQLite row to Python using row_factory. factory None => list;
/// "dict" => dict (column names as keys); "tuple" => tuple; Row class => RapRow instance; else callable(row) => result.
pub(crate) fn row_to_py_with_factory<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
    factory: Option<&Py<PyAny>>,
    text_factory: Option<&Py<PyAny>>,
    converters: Option<&Converters>,
) -> PyResult<Bound<'py, PyAny>> {
    let default = || row_to_py_list(py, row, text_factory, converters).map(|l| l.into_any());
    let Some(f) = factory else {
        return default();
    };
    let f = f.bind(py);
    if f.is_none() {
        return default();
    }
    if let Ok(s) = f.cast::<PyString>() {
        let name = s.to_str()?;
        let n = row_column_count(row);
        return match name {
            "dict" => {
                let dict = PyDict::new(py);
                for i in 0..n {
                    let col_name = row.columns()[i].name();
                    let val = sqlite_value_to_py(py, row, i, text_factory, converters)?;
                    dict.set_item(col_name, val)?;
                }
                Ok(dict.into_any())
            }
            "tuple" => {
                let mut vals = Vec::new();
                for i in 0..n {
                    vals.push(sqlite_value_to_py(py, row, i, text_factory, converters)?);
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
                let n = row_column_count(row);
                for i in 0..n {
                    columns.push(row.columns()[i].name().to_string());
                    let val = sqlite_value_to_py(py, row, i, text_factory, converters)?;
                    values.push(val);
                }
                let raprow = raprow_class.call1((columns, values))?;
                return Ok(raprow.into_any());
            }
        }
    }

    // Fallback: treat as callable
    let list = row_to_py_list(py, row, text_factory, converters)?;
    let result = f.call1((list,))?;
    Ok(result)
}

/// Parse column names from a SELECT query (between SELECT and FROM) for 0-row description.
/// Returns None for SELECT * or unparseable queries. Used so SQLAlchemy ORM can build keymap
/// from cursor description when the first (or only) result has 0 rows.
fn parse_select_column_names(query: &str) -> Option<Vec<String>> {
    let q = query.trim();
    // Normalize whitespace: replace all whitespace sequences with single space
    // This handles newlines and tabs that SQLAlchemy may include in formatted SQL
    let normalized: String = q
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let upper = normalized.to_uppercase();
    if !upper.starts_with("SELECT") {
        return None;
    }
    // Find " FROM " (with spaces) to avoid matching inside string literals or subqueries
    let from_pos = upper.find(" FROM ")?;
    let select_list = normalized[6..from_pos].trim(); // after "SELECT"
    if select_list.is_empty() || select_list == "*" {
        return None;
    }
    let mut names = Vec::new();
    for part in select_list.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        // "table.col" or "t_xxx.id" -> use full identifier so SQLAlchemy keymap matches Column.
        names.push(part.to_string());
    }
    if names.is_empty() {
        return None;
    }
    Some(names)
}

/// Build a minimal cursor description when there are no rows (so SQLAlchemy still sees returns_rows=True).
/// If query is a SELECT and column names can be parsed, use them so ORM keymap matches (e.g. session.get missing).
/// Otherwise uses a single placeholder column.
pub(crate) fn build_description_empty_result<'py>(
    py: Python<'py>,
    query: Option<&str>,
) -> PyResult<Bound<'py, PyAny>> {
    let col_names: Vec<String> = if let Some(q) = query {
        parse_select_column_names(q).unwrap_or_else(|| vec!["column_0".to_string()])
    } else {
        vec!["column_0".to_string()]
    };
    let mut col_tuples = Vec::with_capacity(col_names.len());
    for name in &col_names {
        let seven = PyTuple::new(
            py,
            [
                PyString::new(py, name.as_str()).into(),
                py.None(),
                py.None(),
                py.None(),
                py.None(),
                py.None(),
                py.None(),
            ],
        )?;
        col_tuples.push(seven.into_any());
    }
    Ok(PyTuple::new(py, col_tuples)?.into_any())
}

/// Build a cursor description tuple from a SQLite row (aiosqlite/sqlite3 compatible).
/// Returns a Python tuple of 7-tuples: (name, type_code, display_size, internal_size, precision, scale, null_ok).
pub(crate) fn build_description_tuple<'py>(
    py: Python<'py>,
    row: &sqlx::sqlite::SqliteRow,
) -> PyResult<Bound<'py, PyTuple>> {
    let n = row_column_count(row);
    let mut col_tuples = Vec::with_capacity(n);
    for i in 0..n {
        let name = row.columns()[i].name().to_string();
        let type_name = row.columns()[i].type_info().name().to_ascii_uppercase();
        let seven = PyTuple::new(
            py,
            [
                PyString::new(py, &name).into(),
                PyString::new(py, &type_name).into(),
                py.None(),
                py.None(),
                py.None(),
                py.None(),
                py.None(),
            ],
        )?;
        col_tuples.push(seven.into_any());
    }
    PyTuple::new(py, col_tuples)
}
