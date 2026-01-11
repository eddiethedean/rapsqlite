#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use tokio::sync::Mutex;

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

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Connection>()?;
    Ok(())
}

/// Async SQLite connection.
#[pyclass]
struct Connection {
    path: String,
    pool: Arc<Mutex<Option<SqlitePool>>>,
}

#[pymethods]
impl Connection {
    /// Create a new async SQLite connection.
    #[new]
    fn new(path: String) -> PyResult<Self> {
        validate_path(&path)?;
        Ok(Connection {
            path,
            pool: Arc::new(Mutex::new(None)),
        })
    }

    /// Execute a SQL query (does not return results).
    fn execute(self_: PyRef<Self>, query: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        Python::attach(|py| {
            let future = async move {
                // Get or create the pool
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{path}"))
                                .await
                                .map_err(|e| {
                                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };
                // Use the cloned pool (releases lock immediately)
                let query_clone = query.clone();
                sqlx::query(&query)
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to execute query on database {}: {e}\nQuery: {}",
                            path, query_clone
                        ))
                    })?;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch all rows from a SELECT query.
    fn fetch_all(self_: PyRef<Self>, query: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        Python::attach(|py| {
            let future = async move {
                // Get or create the pool
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{path}"))
                                .await
                                .map_err(|e| {
                                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };
                // Use the cloned pool (releases lock immediately)
                let query_clone = query.clone();
                let rows = sqlx::query(&query)
                    .fetch_all(&pool_clone)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to fetch rows from database {}: {e}\nQuery: {}",
                            path, query_clone
                        ))
                    })?;

                let mut results = Vec::new();
                for row in rows {
                    let mut row_data = Vec::new();
                    for i in 0..row.len() {
                        // Try to get value as string for MVP
                        if let Ok(val) = row.try_get::<String, _>(i) {
                            row_data.push(val);
                        } else if let Ok(val) = row.try_get::<i64, _>(i) {
                            row_data.push(val.to_string());
                        } else if let Ok(val) = row.try_get::<f64, _>(i) {
                            row_data.push(val.to_string());
                        } else {
                            row_data.push(String::new());
                        }
                    }
                    results.push(row_data);
                }
                Ok(results)
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}
