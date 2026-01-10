#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::prelude::*;
use pyo3_asyncio::tokio::future_into_py;
use sqlx::{Row, SqlitePool};

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Connection>()?;
    Ok(())
}

/// Async SQLite connection.
#[pyclass]
struct Connection {
    path: String,
}

#[pymethods]
impl Connection {
    /// Create a new async SQLite connection.
    #[new]
    fn new(path: String) -> Self {
        Connection { path }
    }

    /// Execute a SQL query (does not return results).
    fn execute(self_: PyRef<Self>, query: String) -> PyResult<PyObject> {
        let path = self_.path.clone();
        Python::with_gil(|py| {
            let future = async move {
                let pool = SqlitePool::connect(&format!("sqlite:{path}"))
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to connect: {e}"
                        ))
                    })?;
                sqlx::query(&query).execute(&pool).await.map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                        "Failed to execute query: {e}"
                    ))
                })?;
                Ok(())
            };
            future_into_py(py, future).map(|awaitable| awaitable.to_object(py))
        })
    }

    /// Fetch all rows from a SELECT query.
    fn fetch_all(self_: PyRef<Self>, query: String) -> PyResult<PyObject> {
        let path = self_.path.clone();
        Python::with_gil(|py| {
            let future = async move {
                let pool = SqlitePool::connect(&format!("sqlite:{path}"))
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to connect: {e}"
                        ))
                    })?;

                let rows = sqlx::query(&query).fetch_all(&pool).await.map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                        "Failed to fetch rows: {e}"
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
            future_into_py(py, future).map(|awaitable| awaitable.to_object(py))
        })
    }
}
