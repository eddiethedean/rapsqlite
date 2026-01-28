//! Shared internal types used across modules.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyFloat, PyInt, PyString};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

// Type aliases for complex types to reduce clippy warnings
pub(crate) type UserFunctions = Arc<StdMutex<HashMap<String, (i32, Py<PyAny>)>>>;
pub(crate) type ProgressHandler = Arc<StdMutex<Option<(i32, Py<PyAny>)>>>;

/// Transaction state tracking.
#[derive(Clone, PartialEq)]
pub(crate) enum TransactionState {
    None,
    Active,
}

/// Convert a Python value to a SQLite-compatible value for binding.
/// Returns a boxed value that can be used with sqlx query binding.
#[derive(Clone)]
pub(crate) enum SqliteParam {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqliteParam {
    pub(crate) fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Self> {
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

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Unsupported parameter type: {}. Use int, float, str, bytes, or None.",
            value.get_type().name()?
        )))
    }
}

