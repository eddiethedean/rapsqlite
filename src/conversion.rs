//! SQLite <-> Python value conversions and row factory handling.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use sqlx::{Column, Row};

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
        sqlite3_result_blob(
            ctx,
            ptr as *const std::ffi::c_void,
            len,
            libsqlite3_sys::SQLITE_TRANSIENT(),
        );
        // Keep bytes_val alive
        std::mem::forget(bytes_val);
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
        std::mem::forget(c_str);
        return Ok(());
    }

    // Default: return NULL
    sqlite3_result_null(ctx);
    Ok(())
}

/// Convert a SQLite value from sqlx Row to Python object.
pub(crate) fn sqlite_value_to_py<'py>(
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
pub(crate) fn row_to_py_list<'py>(
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
pub(crate) fn row_to_py_with_factory<'py>(
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
