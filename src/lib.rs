#![allow(non_local_definitions)] // False positive from pyo3 macros

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyFloat, PyInt, PyList, PyString};
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use tokio::sync::Mutex;

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
    last_rowid: Arc<Mutex<i64>>,
    last_changes: Arc<Mutex<u64>>,
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
            transaction_state: Arc::new(Mutex::new(TransactionState::None)),
            last_rowid: Arc::new(Mutex::new(0)),
            last_changes: Arc::new(Mutex::new(0)),
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
        let pool = Arc::clone(&self.pool);
        let transaction_state = Arc::clone(&self.transaction_state);
        Python::attach(|py| {
            let future = async move {
                // Rollback any open transaction
                let trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    drop(trans_guard);
                    let pool_clone = {
                        let pool_guard = pool.lock().await;
                        pool_guard.as_ref().map(|p| p.clone())
                    };
                    if let Some(p) = pool_clone {
                        let _ = sqlx::query("ROLLBACK").execute(&p).await;
                    }
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
        Python::attach(|py| {
            let future = async move {
                // Rollback any open transaction
                let trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    drop(trans_guard);
                    let pool_clone = {
                        let pool_guard = pool.lock().await;
                        pool_guard.as_ref().map(|p| p.clone())
                    };
                    if let Some(p) = pool_clone {
                        let _ = sqlx::query("ROLLBACK").execute(&p).await;
                    }
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
        let transaction_state = Arc::clone(&self.transaction_state);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard == TransactionState::Active {
                    return Err(OperationalError::new_err("Transaction already in progress"));
                }

                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{}", path))
                                .await
                                .map_err(|e| {
                                    OperationalError::new_err(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };

                sqlx::query("BEGIN")
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "BEGIN"))?;

                *trans_guard = TransactionState::Active;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Commit the current transaction.
    fn commit(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let transaction_state = Arc::clone(&self.transaction_state);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                let pool_clone = {
                    let pool_guard = pool.lock().await;
                    pool_guard
                        .as_ref()
                        .ok_or_else(|| OperationalError::new_err("Connection pool not available"))?
                        .clone()
                };

                sqlx::query("COMMIT")
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "COMMIT"))?;

                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Rollback the current transaction.
    fn rollback(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let transaction_state = Arc::clone(&self.transaction_state);
        Python::attach(|py| {
            let future = async move {
                let mut trans_guard = transaction_state.lock().await;
                if *trans_guard != TransactionState::Active {
                    return Err(OperationalError::new_err("No transaction in progress"));
                }

                let pool_clone = {
                    let pool_guard = pool.lock().await;
                    pool_guard
                        .as_ref()
                        .ok_or_else(|| OperationalError::new_err("Connection pool not available"))?
                        .clone()
                };

                sqlx::query("ROLLBACK")
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, "ROLLBACK"))?;

                *trans_guard = TransactionState::None;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Execute a SQL query (does not return results).
    fn execute(self_: PyRef<Self>, query: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        Python::attach(|py| {
            let future = async move {
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{}", path))
                                .await
                                .map_err(|e| {
                                    OperationalError::new_err(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };

                let result = sqlx::query(&query)
                    .execute(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &query))?;

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
        _self_: PyRef<Self>,
        _query: String,
        _parameters: Vec<Vec<Py<PyAny>>>,
    ) -> PyResult<Py<PyAny>> {
        // For Phase 1, execute_many is a placeholder
        // Proper parameter binding will be added in Phase 2
        Python::attach(|py| {
            let future = async move {
                // Placeholder implementation
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
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{}", path))
                                .await
                                .map_err(|e| {
                                    OperationalError::new_err(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };

                let rows = sqlx::query(&query)
                    .fetch_all(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &query))?;

                // Convert rows to Python lists - need Python GIL for this
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let row_list = row_to_py_list(py, row)?;
                        result_list.append(row_list)?;
                    }
                    Ok(result_list.into())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch a single row from a SELECT query.
    fn fetch_one(self_: PyRef<Self>, query: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        Python::attach(|py| {
            let future = async move {
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{}", path))
                                .await
                                .map_err(|e| {
                                    OperationalError::new_err(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };

                let row = sqlx::query(&query)
                    .fetch_one(&pool_clone)
                    .await
                    .map_err(|e| map_sqlx_error(e, &path, &query))?;

                Python::attach(|py| -> PyResult<Py<PyAny>> { Ok(row_to_py_list(py, &row)?.into()) })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Fetch a single row from a SELECT query, returning None if no rows.
    fn fetch_optional(self_: PyRef<Self>, query: String) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        Python::attach(|py| {
            let future = async move {
                let pool_clone = {
                    let mut pool_guard = pool.lock().await;
                    if pool_guard.is_none() {
                        *pool_guard = Some(
                            SqlitePool::connect(&format!("sqlite:{}", path))
                                .await
                                .map_err(|e| {
                                    OperationalError::new_err(format!(
                                        "Failed to connect to database at {}: {e}",
                                        path
                                    ))
                                })?,
                        );
                    }
                    pool_guard.as_ref().unwrap().clone()
                };

                match sqlx::query(&query).fetch_optional(&pool_clone).await {
                    Ok(Some(row)) => Python::attach(|py| -> PyResult<Py<PyAny>> {
                        Ok(row_to_py_list(py, &row)?.into())
                    }),
                    Ok(None) => Python::attach(|py| -> PyResult<Py<PyAny>> { Ok(py.None()) }),
                    Err(e) => Err(map_sqlx_error(e, &path, &query)),
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
        Ok(Cursor {
            connection: slf.into(),
            query: String::new(),
        })
    }
}

/// Cursor for executing queries.
#[pyclass]
struct Cursor {
    connection: Py<Connection>,
    query: String,
}

#[pymethods]
impl Cursor {
    /// Execute a SQL query.
    fn execute(&mut self, query: String) -> PyResult<Py<PyAny>> {
        self.query = query.clone();
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            conn.call_method1("execute", (query,))
                .map(|bound| bound.unbind())
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
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            conn.call_method1("fetch_one", (self.query.clone(),))
                .map(|bound| bound.unbind())
        })
    }

    /// Fetch all rows.
    fn fetchall(&self) -> PyResult<Py<PyAny>> {
        if self.query.is_empty() {
            return Err(ProgrammingError::new_err("No query executed"));
        }
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            conn.call_method1("fetch_all", (self.query.clone(),))
                .map(|bound| bound.unbind())
        })
    }

    /// Fetch many rows.
    ///
    /// Note: For Phase 1, this returns all rows from fetch_all.
    /// Proper size-based slicing will be implemented in Phase 2.
    fn fetchmany(&self, _size: Option<usize>) -> PyResult<Py<PyAny>> {
        if self.query.is_empty() {
            return Err(ProgrammingError::new_err("No query executed"));
        }
        // For Phase 1, fetchmany just calls fetch_all
        // Proper implementation with slicing will be in Phase 2
        Python::attach(|py| {
            let conn = self.connection.bind(py);
            conn.call_method1("fetch_all", (self.query.clone(),))
                .map(|bound| bound.unbind())
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
