//! Shared internal types used across modules.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyFloat, PyInt, PyString, PyTuple};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

// Type aliases for complex types to reduce clippy warnings
pub(crate) type UserFunctions = Arc<StdMutex<HashMap<String, (i32, bool, Py<PyAny>)>>>;
/// (num_params, user_data pointer as usize for cleanup on remove; usize is Send)
pub(crate) type UserAggregates = Arc<StdMutex<HashMap<String, (i32, usize)>>>;
/// Collation name -> user_data pointer as usize for cleanup on remove
pub(crate) type UserCollations = Arc<StdMutex<HashMap<String, usize>>>;
/// (type, adapter callable) for register_adapter; applied before SqliteParam::from_py.
/// The atomic flag lets the normal scalar-parameter path skip the registry lock entirely.
type AdapterEntries = Vec<(Py<PyAny>, Py<PyAny>)>;
pub(crate) struct AdapterRegistry {
    values: StdMutex<AdapterEntries>,
    enabled: AtomicBool,
}

impl AdapterRegistry {
    pub(crate) fn new() -> Self {
        Self {
            values: StdMutex::new(Vec::new()),
            enabled: AtomicBool::new(false),
        }
    }

    pub(crate) fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, AdapterEntries>> {
        self.values.lock()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }
}

/// Declared type name (uppercase) -> converter callable(bytes) -> Python value for register_converter.
/// The atomic flag avoids locking an empty converter registry on ordinary row reads.
type ConverterEntries = HashMap<String, Py<PyAny>>;
pub(crate) struct ConverterRegistry {
    values: StdMutex<ConverterEntries>,
    enabled: AtomicBool,
}

impl ConverterRegistry {
    pub(crate) fn new() -> Self {
        Self {
            values: StdMutex::new(HashMap::new()),
            enabled: AtomicBool::new(false),
        }
    }

    pub(crate) fn lock(
        &self,
    ) -> std::sync::LockResult<std::sync::MutexGuard<'_, ConverterEntries>> {
        self.values.lock()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }
}

pub(crate) type Adapters = Arc<AdapterRegistry>;
pub(crate) type Converters = Arc<ConverterRegistry>;
pub(crate) type ProgressHandler = Arc<StdMutex<Option<(i32, Py<PyAny>)>>>;

/// Max adapter chain depth to avoid infinite loops when adapters return adapted types.
const MAX_ADAPTER_DEPTH: usize = 10;

/// Transaction state tracking.
#[derive(Clone, PartialEq)]
pub(crate) enum TransactionState {
    None,
    /// A transaction is in the process of starting (connection is being acquired / BEGIN pending).
    Starting,
    Active,
}

impl TransactionState {
    /// True if the connection should treat itself as "in transaction" for routing purposes.
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, TransactionState::Starting | TransactionState::Active)
    }
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
        // Exact built-in type checks are cheaper than repeatedly attempting
        // generic extraction. Subclasses retain the compatibility fallbacks below.
        if value.is_none() {
            return Ok(SqliteParam::Null);
        }

        if let Ok(py_int) = value.cast::<PyInt>() {
            if let Ok(int_val) = py_int.extract::<i64>() {
                return Ok(SqliteParam::Int(int_val));
            }
            // SQLite cannot store arbitrary-size Python integers as INTEGER.
            return Ok(SqliteParam::Text(py_int.to_string()));
        }

        if let Ok(py_float) = value.cast::<PyFloat>() {
            return Ok(SqliteParam::Real(py_float.extract::<f64>()?));
        }

        if let Ok(py_str) = value.cast::<PyString>() {
            return Ok(SqliteParam::Text(py_str.to_str()?.to_string()));
        }

        if let Ok(py_bytes) = value.cast::<PyBytes>() {
            return Ok(SqliteParam::Blob(py_bytes.as_bytes().to_vec()));
        }

        // Generic extraction remains for bytearray/buffer-compatible values and
        // user-defined scalar types accepted by the previous implementation.
        if let Ok(bytes_val) = value.extract::<Vec<u8>>() {
            return Ok(SqliteParam::Blob(bytes_val));
        }

        if let Ok(int_val) = value.extract::<i64>() {
            return Ok(SqliteParam::Int(int_val));
        }
        if let Ok(float_val) = value.extract::<f64>() {
            return Ok(SqliteParam::Real(float_val));
        }
        if let Ok(str_val) = value.extract::<String>() {
            return Ok(SqliteParam::Text(str_val));
        }

        // Tuple: convert to text representation for aiosqlite compatibility (single placeholder binding).
        if value.cast::<PyTuple>().is_ok() {
            let s = value.repr()?.to_string();
            return Ok(SqliteParam::Text(s));
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Unsupported parameter type: {}. Use int, float, str, bytes, or None.",
            value.get_type().name()?
        )))
    }

    /// Apply registered adapters then convert to SqliteParam. If adapters is None or empty,
    /// equivalent to from_py(value). Otherwise for each (typ, adapter) in order, if value
    /// is an instance of typ, call adapter(value) and use the result; repeat until no
    /// adapter matches or max depth; then from_py.
    pub(crate) fn apply_adapters_then_from_py(
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        adapters: Option<&Adapters>,
    ) -> PyResult<Self> {
        let Some(adapters) = adapters else {
            return Self::from_py(value);
        };
        if !adapters.is_enabled() {
            return Self::from_py(value);
        }
        let list = adapters.lock().unwrap();
        let mut current = value.clone().unbind();
        for _ in 0..MAX_ADAPTER_DEPTH {
            let mut matched = false;
            for (typ, adapter) in list.iter() {
                let typ_bound = typ.bind(py);
                if current.bind(py).is_instance(typ_bound)? {
                    let adapter_bound = adapter.bind(py);
                    current = adapter_bound.call1((current.bind(py),))?.unbind();
                    matched = true;
                    break;
                }
            }
            if !matched {
                break;
            }
        }
        Self::from_py(current.bind(py))
    }
}
