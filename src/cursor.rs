//! `Cursor` implementation (aiosqlite-compatible cursor API).

#![allow(non_local_definitions)]

use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::pool::PoolConnection;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use crate::conversion::row_to_py_with_factory;
use crate::parameters::{process_named_parameters, process_positional_parameters};
use crate::pool::{ensure_callback_connection, get_or_create_pool, has_callbacks};
use crate::query::{
    bind_and_execute, bind_and_execute_on_connection, bind_and_fetch_all, bind_and_fetch_all_on_connection,
    bind_and_fetch_one, bind_and_fetch_one_on_connection, bind_and_fetch_optional,
    bind_and_fetch_optional_on_connection,
};
use crate::types::{ProgressHandler, SqliteParam, TransactionState, UserFunctions};
use crate::utils::is_select_query;
use crate::{Connection, OperationalError, ProgrammingError};

/// Cursor for executing queries.
#[pyclass]
pub(crate) struct Cursor {
    pub(crate) connection: Py<Connection>,
    pub(crate) query: String,
    pub(crate) results: Arc<StdMutex<Option<Vec<Py<PyAny>>>>>,
    pub(crate) current_index: Arc<StdMutex<usize>>,
    pub(crate) parameters: Arc<StdMutex<Option<Py<PyAny>>>>,
    // Store processed query and parameters to avoid re-processing (fixes parameterized query issue)
    pub(crate) processed_query: Option<String>,
    pub(crate) processed_params: Option<Vec<SqliteParam>>,
    pub(crate) connection_path: String, // Store path for direct pool access
    pub(crate) connection_pool: Arc<Mutex<Option<SqlitePool>>>, // Reference to connection's pool
    pub(crate) connection_pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Reference to connection's pragmas
    pub(crate) pool_size: Arc<StdMutex<Option<usize>>>,
    pub(crate) connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // Connection's row_factory at cursor creation
    // Transaction and callback state for proper connection priority
    pub(crate) transaction_state: Arc<Mutex<TransactionState>>,
    pub(crate) transaction_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) callback_connection: Arc<Mutex<Option<PoolConnection<sqlx::Sqlite>>>>,
    pub(crate) load_extension_enabled: Arc<StdMutex<bool>>,
    pub(crate) user_functions: UserFunctions,
    pub(crate) trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) progress_handler: ProgressHandler,
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
                    let (processed_query, processed_params) =
                        if let (Some(proc_query), Some(proc_params)) =
                            (stored_proc_query_fetchone, stored_proc_params_fetchone)
                        {
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
                                        let (proc_query, param_values) =
                                            process_named_parameters(&query, dict)?;
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
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(
                            &processed_query,
                            &processed_params,
                            conn,
                            &path,
                        )
                        .await?
                    } else if has_callbacks_flag {
                        // Ensure callback connection exists
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;

                        // Use callback connection
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(
                            &processed_query,
                            &processed_params,
                            conn,
                            &path,
                        )
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
                        bind_and_fetch_all(&processed_query, &processed_params, &pool_clone, &path)
                            .await?
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
                        Python::attach(|py| -> PyResult<Py<PyAny>> { Ok(PyList::empty(py).into()) })
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
                        let (processed_query, processed_params) = if let (
                            Some(proc_query),
                            Some(proc_params),
                        ) =
                            (stored_proc_query, stored_proc_params)
                        {
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
                                        let (proc_query, param_values) =
                                            process_named_parameters(&query, dict)?;
                                        // Verify we got parameters if query contains named placeholders
                                        if param_values.is_empty()
                                            && (query.contains(':')
                                                || query.contains('@')
                                                || query.contains('$'))
                                        {
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
                            bind_and_fetch_all_on_connection(
                                &processed_query,
                                &processed_params,
                                conn,
                                &path,
                            )
                            .await?
                        } else if has_callbacks_flag {
                            // Ensure callback connection exists
                            ensure_callback_connection(
                                &path,
                                &pool,
                                &callback_connection,
                                &pragmas,
                                &pool_size,
                                &connection_timeout_secs,
                            )
                            .await?;

                            // Use callback connection
                            let mut conn_guard = callback_connection.lock().await;
                            let conn = conn_guard.as_mut().ok_or_else(|| {
                                OperationalError::new_err("Callback connection not available")
                            })?;
                            bind_and_fetch_all_on_connection(
                                &processed_query,
                                &processed_params,
                                conn,
                                &path,
                            )
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
                            bind_and_fetch_all(
                                &processed_query,
                                &processed_params,
                                &pool_clone,
                                &path,
                            )
                            .await?
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
                    let (processed_query, processed_params) =
                        if let (Some(proc_query), Some(proc_params)) =
                            (stored_proc_query_fetchmany, stored_proc_params_fetchmany)
                        {
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
                                        let (proc_query, param_values) =
                                            process_named_parameters(&query, dict)?;
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
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(
                            &processed_query,
                            &processed_params,
                            conn,
                            &path,
                        )
                        .await?
                    } else if has_callbacks_flag {
                        // Ensure callback connection exists
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;

                        // Use callback connection
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        bind_and_fetch_all_on_connection(
                            &processed_query,
                            &processed_params,
                            conn,
                            &path,
                        )
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
                        bind_and_fetch_all(&processed_query, &processed_params, &pool_clone, &path)
                            .await?
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
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        bind_and_execute_on_connection(&statement, &[], conn, &path).await?;
                    } else if has_callbacks_flag {
                        ensure_callback_connection(
                            &path,
                            &pool,
                            &callback_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;

                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
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
                    return Err(ProgrammingError::new_err(
                        "Cursor not executed. Call execute() first.",
                    ));
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
