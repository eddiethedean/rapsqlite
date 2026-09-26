//! `Connection` implementation (main user-facing class).

#![allow(non_local_definitions)] // False positive from pyo3 macros

mod backup;
mod cache;
mod callbacks;
mod schema;
pub(crate) use callbacks::{
    cleanup_failed_transaction_connection, clear_active_handle, discard_callback_connection,
    finish_transaction_callback_connection, rebind_callbacks, sqlite_connection_in_transaction,
    CallbackContext,
};

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::sqlite::SqliteConnection;
use sqlx::{Column, Row, TypeInfo, ValueRef};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

// libsqlite3-sys for raw SQLite C API access
use libsqlite3_sys::{
    sqlite3_free, sqlite3_interrupt, sqlite3_libversion_number, sqlite3_load_extension,
    sqlite3_total_changes, SQLITE_OK,
};

use crate::batch::RawScalar;
use crate::context_managers::next_savepoint_name;
use crate::conversion::row_to_py_with_factory;
use crate::errors::{map_sqlite_error_from_msg, map_sqlx_error, map_sqlx_error_with_visibility};
use crate::parameters::process_parameters;
use crate::pool::{
    acquire_with_pragmas, apply_pragmas_to_connection, callbacks_enabled,
    capture_init_hook_reentrancy, close_registered_pool_if_last, execute_init_hook_if_needed,
    execute_init_hook_if_needed_fast, get_or_create_pool, has_callbacks, lock_session_connection,
    lock_session_connection_from_pool, release_session_connection, InitHookState,
    PoolConnectionSlot, PoolHandle, PoolRegistryLease, SessionConnectionSlot,
};
use crate::query::{
    bind_and_execute_on_connection, bind_and_fetch_all_on_connection,
    bind_and_fetch_one_on_connection, bind_and_fetch_optional_on_connection,
    bind_and_fetch_scalar_optional_on_connection,
};
use crate::types::{
    Adapters, Converters, ProgressHandler, SqliteParam, TraceCallback, TraceCallbackState,
    TransactionState, TransactionStateTracker, UserAggregates, UserCollations, UserFunctions,
};
use crate::utils::{
    cstr_from_c_char_ptr, is_select_query, parse_connection_string, returns_result_rows,
    track_query_usage_count_if_enabled, track_query_usage_if_enabled, validate_path,
    QueryUsageStats,
};
use crate::{
    Cursor, ExecuteContextManager, NotSupportedError, ProgrammingError, SavepointContextManager,
    TransactionContextManager, ValueError,
};
use crate::{InterfaceError, InternalError, OperationalError};

/// Async SQLite connection.
#[pyclass]
pub(crate) struct Connection {
    path: String,
    pool_registry_lease: Arc<StdMutex<Option<PoolRegistryLease>>>,
    pool: Arc<PoolHandle>,
    transaction_state: Arc<TransactionStateTracker>,
    /// Serializes raw SQL BEGIN with implicit DML transaction startup.
    transaction_start_lock: Arc<Mutex<()>>,
    // Store the connection used for active transaction
    // All operations within a transaction must use this same connection
    transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    last_rowid: Arc<Mutex<i64>>,
    last_changes: Arc<Mutex<u64>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>, // Store PRAGMA settings
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,   // Optional initialization hook
    init_hook_called: Arc<InitHookState>,
    init_hook_present: Arc<AtomicBool>,
    pool_size: Arc<StdMutex<Option<usize>>>, // Configurable pool size
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>, // Connection timeout in seconds
    idle_timeout_secs: Arc<StdMutex<Option<u64>>>, // Idle connection timeout (pool closes idle conns after this)
    row_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // None | "dict" | "tuple" | callable
    text_factory: Arc<StdMutex<Option<Py<PyAny>>>>, // Callable(bytes) -> str, or None for default UTF-8
    // Opt-in, bounded query analytics. This is separate from SQLx's own
    // per-connection prepared statement cache.
    query_usage_stats: Arc<StdMutex<QueryUsageStats>>,
    query_usage_enabled: Arc<AtomicBool>,
    // Callback infrastructure (Phase 2.7)
    callback_connection: Arc<Mutex<PoolConnectionSlot>>, // Dedicated connection for callbacks
    callback_operation_lock: Arc<Mutex<()>>,
    callback_connection_required: Arc<StdMutex<bool>>, // Callback handle needed for extensions or callbacks
    callback_features: Arc<AtomicU8>,
    extension_loading_allowed: Arc<StdMutex<bool>>,
    loaded_extensions: Arc<StdMutex<Vec<String>>>,
    user_functions: UserFunctions,   // name -> (nargs, callback)
    user_aggregates: UserAggregates, // name -> (num_params, user_data ptr for cleanup)
    user_collations: UserCollations, // name -> user_data ptr for cleanup on remove
    adapters: Adapters,              // (type, callable) for register_adapter
    converters: Converters,          // typename -> callable(bytes)->Any for register_converter
    trace_callback: TraceCallback,
    authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>, // Authorizer callback
    progress_handler: ProgressHandler,                     // (n, callback)
    // Raw SQLite callback contexts (Box<Py<PyAny>> as void*) so we can free on replace/clear.
    // Stored as usize for Send/Sync; 0 means none installed.
    authorizer_callback_ctx_ptr: Arc<StdMutex<usize>>,
    progress_handler_ctx_ptr: Arc<StdMutex<usize>>,
    // Error message security: control whether query strings are included in errors
    include_query_in_errors: Arc<StdMutex<bool>>, // If false, exclude query strings from error messages
    // SQLite busy_timeout (aiosqlite compatibility) - timeout in seconds for database locks
    timeout: Arc<StdMutex<f64>>, // Default: 5.0 seconds (matches sqlite3 default)
    // Phase 3.9: transaction isolation level (None | "DEFERRED" | "IMMEDIATE" | "EXCLUSIVE")
    isolation_level: Arc<StdMutex<Option<String>>>,
    // Phase 3.10: iter_chunk_size (aiosqlite compat); used for chunked iteration when applicable
    iter_chunk_size: Arc<StdMutex<usize>>,
    /// True only when in an explicit transaction (begin() or transaction()); false for implicit.
    /// in_transaction() reports True only when this is true (aiosqlite semantics).
    explicit_transaction: Arc<Mutex<bool>>,
    /// True after close() has completed; used to reject post-close operations without touching pool/Tokio.
    closed: Arc<StdMutex<bool>>,
    /// Reused connection for non-transaction, non-callback operations (session-scoped).
    /// Released on close() and when starting a transaction to match aiosqlite and improve concurrent reads.
    session_connection: SessionConnectionSlot,
}

// Note: We do not implement Drop for Connection because:
// 1. PyO3 pyclass cleanup happens in Python's GC, which may not have an async runtime
// 2. Async cleanup (transaction rollback, connection release) requires async context
// 3. The close() method handles all cleanup properly
//
// Resource cleanup behavior:
// - Arc references will be automatically dropped when Connection is dropped
// - Pool connections are in PoolConnectionSlot; dropped outside runtime context are forgotten
// - However, active transactions will NOT be rolled back automatically
// - Callback connections will be returned to pool when Arc is dropped
//
// For proper cleanup including transaction rollback, always:
// - Use async context managers: `async with rapsqlite.connect(...) as db:`
// - Or call close() explicitly: `await db.close()`

fn invoke_trace_callback(trace_callback: &TraceCallback, sql: &str) {
    let callback = trace_callback.clone_callback();
    if let Some(callback) = callback {
        #[allow(deprecated)]
        Python::attach(|py| {
            let _ = callback.bind(py).call1((sql,));
        });
    }
}

/// Shared state needed to run a single execute. Used by Connection and ExecuteContextManager
/// so we clone one struct instead of 25+ individual fields when creating ExecuteContextManager.
#[derive(Clone)]
pub(crate) struct ConnectionExecutionState {
    pub(crate) path: String,
    pub(crate) pool: Arc<PoolHandle>,
    pub(crate) session_connection: SessionConnectionSlot,
    pub(crate) pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub(crate) pool_size: Arc<StdMutex<Option<usize>>>,
    pub(crate) connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub(crate) transaction_state: Arc<TransactionStateTracker>,
    pub(crate) transaction_start_lock: Arc<Mutex<()>>,
    pub(crate) transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub(crate) callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub(crate) callback_context: CallbackContext,
    #[allow(dead_code)] // used when binding parameters; ExecuteContextManager may not read
    pub(crate) adapters: Adapters,
    pub(crate) converters: Converters,
    pub(crate) trace_callback: TraceCallback,
    pub(crate) init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub(crate) init_hook_called: Arc<InitHookState>,
    pub(crate) init_hook_present: Arc<AtomicBool>,
    pub(crate) last_rowid: Arc<Mutex<i64>>,
    pub(crate) last_changes: Arc<Mutex<u64>>,
    pub(crate) timeout: Arc<StdMutex<f64>>,
    pub(crate) isolation_level: Arc<StdMutex<Option<String>>>,
    pub(crate) include_query_in_errors: Arc<StdMutex<bool>>,
    pub(crate) closed: Arc<StdMutex<bool>>,
    pub(crate) explicit_transaction: Arc<Mutex<bool>>,
}

/// Helper to turn a poisoned StdMutex into a Python InternalError instead of panicking.
fn lock_or_internal_error<'a, T>(
    mutex: &'a StdMutex<T>,
    context: &str,
) -> Result<std::sync::MutexGuard<'a, T>, PyErr> {
    mutex
        .lock()
        .map_err(|_| InternalError::new_err(format!("internal error: mutex poisoned in {context}")))
}

/// Returns Ok(()) if the connection is not closed; otherwise returns InterfaceError.
pub(crate) fn ensure_not_closed(closed: &Arc<StdMutex<bool>>) -> Result<(), PyErr> {
    let guard = lock_or_internal_error(closed, "Connection::ensure_not_closed")?;
    if *guard {
        return Err(InterfaceError::new_err("connection is closed"));
    }
    Ok(())
}

impl Connection {
    pub(crate) fn callback_context(&self) -> CallbackContext {
        CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        }
    }

    /// Build schema introspection context for delegation to schema module.
    fn build_schema_context(self_: PyRef<Self>) -> PyResult<schema::SchemaContext> {
        let connection_self: Py<Connection> = self_.into();
        #[allow(deprecated)]
        Python::attach(|py| {
            let connection = connection_self.borrow(py);
            let init_hook_called = Arc::clone(&connection.init_hook_called);
            let init_hook_present = Arc::clone(&connection.init_hook_present);
            let init_hook_reentrant = capture_init_hook_reentrancy(
                &init_hook_present,
                &init_hook_called,
                &connection_self,
            )?;

            Ok(schema::SchemaContext {
                callback_context: connection.callback_context(),
                path: connection.path.clone(),
                pool: Arc::clone(&connection.pool),
                pragmas: Arc::clone(&connection.pragmas),
                pool_size: Arc::clone(&connection.pool_size),
                connection_timeout_secs: Arc::clone(&connection.connection_timeout_secs),
                idle_timeout_secs: Arc::clone(&connection.idle_timeout_secs),
                transaction_state: Arc::clone(&connection.transaction_state),
                transaction_connection: Arc::clone(&connection.transaction_connection),
                session_connection: connection.session_connection.clone(),
                callback_connection: Arc::clone(&connection.callback_connection),
                callback_connection_required: Arc::clone(&connection.callback_connection_required),
                user_functions: Arc::clone(&connection.user_functions),
                user_aggregates: Arc::clone(&connection.user_aggregates),
                user_collations: Arc::clone(&connection.user_collations),
                trace_callback: Arc::clone(&connection.trace_callback),
                authorizer_callback: Arc::clone(&connection.authorizer_callback),
                progress_handler: Arc::clone(&connection.progress_handler),
                init_hook: Arc::clone(&connection.init_hook),
                init_hook_called,
                init_hook_present,
                init_hook_reentrant,
                include_query_in_errors: Arc::clone(&connection.include_query_in_errors),
                closed: Arc::clone(&connection.closed),
                connection_self: connection_self.clone_ref(py),
            })
        })
    }
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
    /// .. code-block:: python
    ///
    ///     from rapsqlite import Connection
    ///
    ///     # Basic connection
    ///     async with Connection("example.db") as conn:
    ///         await conn.execute("CREATE TABLE test (id INTEGER)")
    ///
    ///     # With PRAGMA settings
    ///     async with Connection("example.db", pragmas={
    ///         "journal_mode": "WAL",
    ///         "foreign_keys": True
    ///     }) as conn:
    ///         await conn.execute("CREATE TABLE test (id INTEGER)")
    ///
    ///     # With initialization hook
    ///     async def init_db(conn):
    ///         await conn.execute("CREATE TABLE IF NOT EXISTS users (id INTEGER)")
    ///
    ///     async with Connection("example.db", init_hook=init_db) as conn:
    ///         # Database is already initialized
    ///         pass
    #[new]
    #[pyo3(signature = (path, *, pragmas = None, init_hook = None, timeout = 5.0, iter_chunk_size = 64, loop_param = None))]
    fn new(
        path: String,
        pragmas: Option<&Bound<'_, pyo3::types::PyDict>>,
        init_hook: Option<Py<PyAny>>,
        timeout: f64,
        iter_chunk_size: i64,
        loop_param: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let _ = loop_param; // Accepted for aiosqlite compat (connect(loop=...)); ignored (deprecated).
                            // Validate timeout (must be non-negative)
        if timeout < 0.0 {
            return Err(ValueError::new_err("timeout must be >= 0.0"));
        }
        if iter_chunk_size < 1 {
            return Err(ValueError::new_err("iter_chunk_size must be >= 1"));
        }
        let iter_chunk_size = iter_chunk_size as usize;
        // Split SQLite URI connection options from the database path. Options such
        // as mode/cache belong in the SQLite URL, not in PRAGMA statements.
        let (parsed_path, uri_params) = parse_connection_string(&path)?;
        validate_path(&parsed_path)?;

        let mut all_pragmas = Vec::new();
        let mut sqlite_uri_options = Vec::new();
        for (key, value) in uri_params {
            match key.to_ascii_lowercase().as_str() {
                "mode" | "cache" | "immutable" | "vfs" | "nolock" | "psow" => {
                    sqlite_uri_options.push((key, value));
                }
                _ => all_pragmas.push((key, value)),
            }
        }
        let db_path = if sqlite_uri_options.is_empty() {
            parsed_path
        } else {
            let options = sqlite_uri_options
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&");
            format!("file:{parsed_path}?{options}")
        };
        let (pool_registry_lease, session_connection) = PoolRegistryLease::new(db_path.clone());

        // Add pragmas from dict if provided
        if let Some(pragmas_dict) = pragmas {
            for item in pragmas_dict.iter() {
                let (key, value) = item; // iter() returns tuples directly in pyo3 0.27
                let key_str = key.extract::<String>()?;
                let value_str = value.to_string();
                all_pragmas.push((key_str, value_str));
            }
        }

        // Ensure busy_timeout is always set from the timeout parameter.
        // This makes concurrent writers reliably wait rather than erroring with SQLITE_BUSY.
        let timeout_ms = (timeout * 1000.0) as i64;
        all_pragmas.retain(|(k, _)| k.to_lowercase() != "busy_timeout");
        all_pragmas.push(("busy_timeout".to_string(), timeout_ms.to_string()));
        let init_hook_present = init_hook.is_some();

        Ok(Connection {
            path: db_path,
            pool_registry_lease: Arc::new(StdMutex::new(Some(pool_registry_lease))),
            pool: Arc::new(PoolHandle::default()),
            transaction_state: Arc::new(TransactionStateTracker::new()),
            transaction_start_lock: Arc::new(Mutex::new(())),
            transaction_connection: Arc::new(Mutex::new(PoolConnectionSlot::default())),
            last_rowid: Arc::new(Mutex::new(0)),
            last_changes: Arc::new(Mutex::new(0)),
            pragmas: Arc::new(StdMutex::new(all_pragmas)),
            init_hook: Arc::new(StdMutex::new(init_hook)),
            init_hook_called: Arc::new(InitHookState::new()),
            init_hook_present: Arc::new(AtomicBool::new(init_hook_present)),
            pool_size: Arc::new(StdMutex::new(None)),
            connection_timeout_secs: Arc::new(StdMutex::new(None)),
            idle_timeout_secs: Arc::new(StdMutex::new(None)),
            row_factory: Arc::new(StdMutex::new(None)),
            text_factory: Arc::new(StdMutex::new(None)),
            query_usage_stats: Arc::new(StdMutex::new(QueryUsageStats::default())),
            query_usage_enabled: Arc::new(AtomicBool::new(false)),
            // Callback infrastructure (Phase 2.7)
            callback_connection: Arc::new(Mutex::new(PoolConnectionSlot::default())),
            callback_operation_lock: Arc::new(Mutex::new(())),
            callback_connection_required: Arc::new(StdMutex::new(false)),
            callback_features: Arc::new(AtomicU8::new(0)),
            extension_loading_allowed: Arc::new(StdMutex::new(false)),
            loaded_extensions: Arc::new(StdMutex::new(Vec::new())),
            user_functions: Arc::new(StdMutex::new(HashMap::new())),
            user_aggregates: Arc::new(StdMutex::new(HashMap::new())),
            user_collations: Arc::new(StdMutex::new(HashMap::new())),
            adapters: Arc::new(crate::types::AdapterRegistry::new()),
            converters: Arc::new(crate::types::ConverterRegistry::new()),
            trace_callback: Arc::new(TraceCallbackState::new()),
            authorizer_callback: Arc::new(StdMutex::new(None)),
            progress_handler: Arc::new(StdMutex::new(None)),
            authorizer_callback_ctx_ptr: Arc::new(StdMutex::new(0)),
            progress_handler_ctx_ptr: Arc::new(StdMutex::new(0)),
            include_query_in_errors: Arc::new(StdMutex::new(true)), // Default: include queries for debugging
            timeout: Arc::new(StdMutex::new(timeout)), // SQLite busy_timeout in seconds (aiosqlite compatibility)
            isolation_level: Arc::new(StdMutex::new(None)), // Phase 3.9: None | DEFERRED | IMMEDIATE | EXCLUSIVE
            iter_chunk_size: Arc::new(StdMutex::new(iter_chunk_size)), // Phase 3.10: aiosqlite compat
            explicit_transaction: Arc::new(Mutex::new(false)),
            closed: Arc::new(StdMutex::new(false)),
            session_connection,
        })
    }

    #[getter(path)]
    fn path(&self) -> &str {
        &self.path
    }

    /// Enable or disable query-usage analytics. Disabled by default so SQL
    /// normalization and the analytics mutex are absent from normal queries.
    #[getter(query_usage_tracking)]
    fn query_usage_tracking(&self) -> bool {
        self.query_usage_enabled.load(Ordering::Acquire)
    }

    #[setter(query_usage_tracking)]
    fn set_query_usage_tracking(&self, enabled: bool) {
        self.query_usage_enabled.store(enabled, Ordering::Release);
    }

    /// Return a snapshot of normalized query usage counts collected while
    /// ``query_usage_tracking`` was enabled. Retention is bounded; see
    /// ``query_usage_dropped`` for executions omitted by the diagnostic caps.
    fn query_usage(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let usage = self
            .query_usage_stats
            .lock()
            .map_err(|_| InternalError::new_err("internal error: query usage mutex poisoned"))?;
        let result = PyDict::new(py);
        for (query, count) in usage.counts.iter() {
            result.set_item(query, *count)?;
        }
        Ok(result.into_any().unbind())
    }

    /// Return the number of query executions omitted because a query exceeded
    /// the diagnostic size cap or the distinct-query limit was reached.
    fn query_usage_dropped(&self) -> PyResult<u64> {
        self.query_usage_stats
            .lock()
            .map(|usage| usage.dropped)
            .map_err(|_| InternalError::new_err("internal error: query usage mutex poisoned"))
    }

    /// Clear collected query-usage analytics without changing the enabled state.
    fn clear_query_usage(&self) -> PyResult<()> {
        self.query_usage_stats
            .lock()
            .map_err(|_| InternalError::new_err("internal error: query usage mutex poisoned"))?
            .clear();
        Ok(())
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
    /// .. code-block:: python
    ///
    ///     await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    ///     await conn.execute("INSERT INTO users (name) VALUES (?)", ["Bob"])
    ///     changes = await conn.total_changes()  # Returns 2
    fn total_changes(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self.idle_timeout_secs);
        let session_connection = self.session_connection.clone();
        let callback_connection = Arc::clone(&self.callback_connection);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_features = Arc::clone(&self.callback_features);
        let closed = Arc::clone(&self.closed);
        let callback_context = self.callback_context();

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Check if we're in a transaction - if so, use transaction connection
                let in_transaction = transaction_state.is_routing_active().await;

                let raw_db = if in_transaction {
                    // Use transaction connection
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock handle: {e}"))
                    })?;
                    handle.as_raw_handle().as_ptr()
                } else {
                    // Check if callbacks are set - if not, use session connection
                    let has_callbacks_flag = callbacks_enabled(&callback_features);

                    if has_callbacks_flag {
                        let _callback_operation_guard =
                            callback_context.callback_operation_lock.lock().await;
                        callbacks::rebind_callbacks(callback_context.clone()).await?;
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock handle: {e}"))
                        })?;
                        let total =
                            unsafe { sqlite3_total_changes(handle.as_raw_handle().as_ptr()) };
                        drop(handle);
                        drop(conn_guard);
                        callbacks::discard_callback_connection(&callback_context).await;
                        return Ok(total as u64);
                    } else {
                        // No callbacks - use session connection (compute total while handle is valid)
                        let mut conn_guard = lock_session_connection(
                            &path,
                            &pool,
                            &session_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Session connection not available")
                        })?;
                        let sqlite_conn: &mut SqliteConnection = &mut *conn;
                        let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                            OperationalError::new_err(format!("Failed to lock handle: {e}"))
                        })?;
                        let handle_ptr = handle.as_raw_handle().as_ptr();
                        let total = unsafe { sqlite3_total_changes(handle_ptr) };
                        return Ok(total as u64);
                    }
                };

                // Call sqlite3_total_changes (for transaction or callback connection paths)
                // Safety: raw_db is a valid sqlite3* pointer obtained from
                // lock_handle().as_raw_handle().as_ptr() and is guaranteed to be valid
                // for the lifetime of the handle lock. sqlite3_total_changes is a
                // read-only operation that doesn't modify the database handle.
                let total = unsafe { sqlite3_total_changes(raw_db) };

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
    /// .. code-block:: python
    ///
    ///     in_tx = await conn.in_transaction()  # False
    ///     await conn.begin()
    ///     in_tx = await conn.in_transaction()  # True
    ///     await conn.commit()
    ///     in_tx = await conn.in_transaction()  # False
    fn in_transaction(&self) -> PyResult<Py<PyAny>> {
        let transaction_state = Arc::clone(&self.transaction_state);
        let explicit_transaction = Arc::clone(&self.explicit_transaction);

        Python::attach(|py| {
            let future = async move {
                let trans_guard = transaction_state.lock().await;
                let ex_guard = explicit_transaction.lock().await;
                Ok(*trans_guard == TransactionState::Active && *ex_guard)
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
        if let Some(lease) = self.pool_registry_lease.lock().unwrap().as_ref() {
            lease.configure_pool_size(*guard);
        }
        Ok(())
    }

    /// Retain the physical SQLite session between operations on this logical
    /// Connection. Disabled by default to avoid consuming pool capacity while
    /// a Connection is idle; enable it for repeated low-latency workloads.
    #[getter(session_affinity)]
    fn session_affinity(&self) -> bool {
        self.session_connection.retain()
    }

    #[setter(session_affinity)]
    fn set_session_affinity(&self, enabled: bool) {
        self.session_connection.set_retain(enabled);
        if !enabled {
            // Callback-bound handles occupy a separate slot from the retained
            // session connection. Release one immediately when it is idle;
            // an active callback operation releases it on completion.
            callbacks::release_callback_connection_if_idle(&self.callback_context());
        }
    }

    /// Return pool metrics (size, num_idle, in_use, max_connections).
    fn pool_metrics(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let pool = Arc::clone(&self.pool);
        let pragmas = Arc::clone(&self.pragmas);
        let pool_size = Arc::clone(&self.pool_size);
        let connection_timeout_secs = Arc::clone(&self.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self.idle_timeout_secs);
        let closed = Arc::clone(&self.closed);
        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                    &idle_timeout_secs,
                )
                .await?;
                let p = pool
                    .get()
                    .ok_or_else(|| OperationalError::new_err("Pool not available"))?;
                let size = p.size();
                let num_idle = p.num_idle();
                let in_use = size as usize - num_idle;
                let max_connections = p.options().get_max_connections();
                #[allow(deprecated)]
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let dict = PyDict::new(py);
                    dict.set_item("size", size)?;
                    dict.set_item("num_idle", num_idle)?;
                    dict.set_item("in_use", in_use)?;
                    dict.set_item("max_connections", max_connections)?;
                    Ok(dict.into_any().unbind())
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    #[getter(connection_timeout)]
    fn connection_timeout(&self) -> PyResult<Py<PyAny>> {
        // Note: Python::attach is used here for sync operation in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::attach(|py| {
            let guard = self.connection_timeout_secs.lock().unwrap();
            Ok(match guard.as_ref() {
                Some(&n) => PyInt::new(py, n as i64).into_any().unbind(),
                None => py.None(),
            })
        })
    }

    /// Get whether query strings are included in error messages.
    ///
    /// When True (default), error messages include sanitized query strings for debugging.
    /// When False, query strings are excluded entirely for enhanced security.
    ///
    /// Queries are always sanitized to remove sensitive patterns (passwords, tokens, etc.)
    /// even when included. For maximum security with highly sensitive data, set this to False.
    #[getter(include_query_in_errors)]
    fn include_query_in_errors(&self) -> PyResult<bool> {
        let guard = self.include_query_in_errors.lock().unwrap();
        Ok(*guard)
    }

    /// Set whether query strings are included in error messages.
    ///
    /// When True (default), error messages include sanitized query strings for debugging.
    /// When False, query strings are excluded entirely for enhanced security.
    ///
    /// Queries are always sanitized to remove sensitive patterns (passwords, tokens, etc.)
    /// even when included. For maximum security with highly sensitive data, set this to False.
    #[setter(include_query_in_errors)]
    fn set_include_query_in_errors(&self, value: bool) -> PyResult<()> {
        let mut guard = self.include_query_in_errors.lock().unwrap();
        *guard = value;
        Ok(())
    }

    /// Get the SQLite busy_timeout value (in seconds).
    ///
    /// This controls how long SQLite will wait when the database is locked by another
    /// process/thread before raising an error. Default: 5.0 seconds (matches sqlite3/aiosqlite).
    ///
    /// This is an aiosqlite-compatible feature that sets SQLite's busy_timeout PRAGMA.
    #[getter(timeout)]
    fn timeout(&self) -> PyResult<f64> {
        let guard = self.timeout.lock().unwrap();
        Ok(*guard)
    }

    /// Set the SQLite busy_timeout value (in seconds).
    ///
    /// This controls how long SQLite will wait when the database is locked by another
    /// process/thread before raising an error. Set to 0.0 to disable timeout.
    ///
    /// This is an aiosqlite-compatible feature that sets SQLite's busy_timeout PRAGMA.
    /// The timeout is applied to connections when they are used (e.g., in transactions).
    #[setter(timeout)]
    fn set_timeout(&self, value: f64) -> PyResult<()> {
        if value < 0.0 {
            return Err(ValueError::new_err("timeout must be >= 0.0"));
        }
        let mut guard = self.timeout.lock().unwrap();
        *guard = value;

        // Keep per-connection PRAGMAs in sync so newly acquired pooled connections
        // always have the correct busy_timeout.
        let timeout_ms = (value * 1000.0) as i64;
        let mut pragmas = self.pragmas.lock().unwrap();
        pragmas.retain(|(k, _)| k.to_lowercase() != "busy_timeout");
        pragmas.push(("busy_timeout".to_string(), timeout_ms.to_string()));
        Ok(())
    }

    /// Get iter_chunk_size (Phase 3.10). Used for chunked iteration; aiosqlite-compatible.
    #[getter(iter_chunk_size)]
    fn iter_chunk_size(&self) -> PyResult<usize> {
        let guard = self.iter_chunk_size.lock().unwrap();
        Ok(*guard)
    }

    /// Get the transaction isolation level (Phase 3.9). None | "DEFERRED" | "IMMEDIATE" | "EXCLUSIVE".
    #[getter(isolation_level)]
    fn isolation_level(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let guard = self.isolation_level.lock().unwrap();
        match guard.as_deref() {
            Some(s) => Ok(PyString::new(py, s).into()),
            None => Ok(py.None()),
        }
    }

    /// Set the transaction isolation level. Use None for default (DEFERRED when beginning transactions).
    #[setter(isolation_level)]
    fn set_isolation_level(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.isolation_level.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            let s = value.extract::<String>()?;
            let u = s.to_uppercase();
            match u.as_str() {
                "DEFERRED" | "IMMEDIATE" | "EXCLUSIVE" => Some(u),
                _ => {
                    return Err(ValueError::new_err(
                        "isolation_level must be None, 'DEFERRED', 'IMMEDIATE', or 'EXCLUSIVE'",
                    ));
                }
            }
        };
        Ok(())
    }

    #[setter(connection_timeout)]
    fn set_connection_timeout(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.connection_timeout_secs.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            // Accept both int and float, convert to u64 (seconds)
            // Try float first (handles both int and float), then int
            let n: f64 = if let Ok(f) = value.extract::<f64>() {
                f
            } else if let Ok(i) = value.extract::<i64>() {
                i as f64
            } else {
                return Err(pyo3::exceptions::PyTypeError::new_err(
                    "connection_timeout must be an int or float",
                ));
            };
            if n < 0.0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "connection_timeout must be >= 0",
                ));
            }
            Some(n as u64)
        };
        Ok(())
    }

    /// Idle connection timeout in seconds. When set, connections idle in the pool
    /// longer than this are closed. None (default) means no idle timeout.
    #[getter(idle_timeout)]
    fn idle_timeout(&self) -> PyResult<Py<PyAny>> {
        #[allow(deprecated)]
        Python::attach(|py| {
            let guard = self.idle_timeout_secs.lock().unwrap();
            Ok(match guard.as_ref() {
                Some(&n) => PyInt::new(py, n as i64).into_any().unbind(),
                None => py.None(),
            })
        })
    }

    #[setter(idle_timeout)]
    fn set_idle_timeout(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut guard = self.idle_timeout_secs.lock().unwrap();
        *guard = if value.is_none() {
            None
        } else {
            let n: i64 = value
                .extract::<i64>()
                .or_else(|_| value.extract::<u64>().map(|u| u as i64))?;
            if n < 0 {
                return Err(ValueError::new_err("idle_timeout must be >= 0"));
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
    /// Commit on clean exit, rollback on exception (aiosqlite/sqlite3-style).
    fn __aexit__(
        &self,
        exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let commit_on_exit = exc_type.is_none();
        let path = self.path.clone();
        let callback_operation_lock = Arc::clone(&self.callback_operation_lock);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let explicit_transaction = Arc::clone(&self.explicit_transaction);
        let callback_context = self.callback_context();
        Python::attach(|py| {
            let future = async move {
                let _callback_operation_guard = callback_operation_lock.lock().await;
                let mut cleanup_error = None;

                // Commit or rollback any open transaction.
                // If user explicitly called begin(), rollback on exit (user controls commit/rollback).
                // Otherwise: commit on clean exit, rollback on exception (aiosqlite/sqlite3-style).
                let transaction_active =
                    *transaction_state.lock().await == TransactionState::Active;
                if transaction_active {
                    let is_explicit = {
                        let ex_guard = explicit_transaction.lock().await;
                        *ex_guard
                    };
                    callbacks::clear_active_handle(&transaction_connection);
                    let mut conn_guard = transaction_connection.lock().await;
                    if let Some(mut conn) = conn_guard.0.take() {
                        let sql = if is_explicit {
                            "ROLLBACK"
                        } else if commit_on_exit {
                            "COMMIT"
                        } else {
                            "ROLLBACK"
                        };
                        if let Err(error) = sqlx::query(sql).execute(&mut *conn).await {
                            if sql == "COMMIT" {
                                cleanup_error = Some(map_sqlx_error(error, &path, sql));
                            }
                        }
                        let mut still_in_transaction =
                            callbacks::sqlite_connection_in_transaction(&mut conn)
                                .await
                                .unwrap_or(true);
                        // A failed COMMIT (for example, a deferred foreign-key
                        // violation) leaves SQLite's transaction open. Roll it
                        // back before releasing the handle, but still report the
                        // original commit error to the context-manager caller.
                        if still_in_transaction && sql == "COMMIT" {
                            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                            still_in_transaction =
                                callbacks::sqlite_connection_in_transaction(&mut conn)
                                    .await
                                    .unwrap_or(true);
                        }
                        if still_in_transaction {
                            conn.close_on_drop();
                            drop(conn);
                        } else if let Err(error) =
                            callbacks::release_callback_connection_after_cleanup(
                                &callback_context,
                                conn,
                            )
                            .await
                        {
                            cleanup_error.get_or_insert(error);
                        }
                    }
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::None;
                    drop(trans_guard);
                    let mut ex_guard = explicit_transaction.lock().await;
                    *ex_guard = false;
                }

                callbacks::clear_active_handle(&callback_context.callback_connection);
                let callback_connection = {
                    let mut callback_guard = callback_context.callback_connection.lock().await;
                    callback_guard.0.take()
                };
                if let Some(connection) = callback_connection {
                    if let Err(error) = callbacks::release_callback_connection_after_cleanup(
                        &callback_context,
                        connection,
                    )
                    .await
                    {
                        cleanup_error.get_or_insert(error);
                    }
                }
                callbacks::clear_callback_registries(&callback_context);

                // Release our reference to the pool (do not close: pool is shared via global registry).
                if let Some(error) = cleanup_error {
                    Err(error)
                } else {
                    Ok(())
                }
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Close the connection.
    fn close(&self) -> PyResult<Py<PyAny>> {
        let session_connection = self.session_connection.clone();
        let callback_operation_lock = Arc::clone(&self.callback_operation_lock);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let explicit_transaction = Arc::clone(&self.explicit_transaction);
        let closed = Arc::clone(&self.closed);
        let callback_context = self.callback_context();
        let pool_registry_lease = Arc::clone(&self.pool_registry_lease);
        let lease_guard = pool_registry_lease.lock().unwrap();
        let pool_registry_identity = lease_guard
            .as_ref()
            .and_then(PoolRegistryLease::identity)
            .map(str::to_owned);
        drop(lease_guard);
        Python::attach(|py| {
            let future = async move {
                *closed.lock().unwrap() = true;
                // Serialize teardown against callback registration and use.
                let _callback_operation_guard = callback_operation_lock.lock().await;
                // Release session connection back to pool
                release_session_connection(&session_connection).await;

                let mut cleanup_error = None;
                let transaction_active =
                    *transaction_state.lock().await == TransactionState::Active;
                if transaction_active {
                    callbacks::clear_active_handle(&transaction_connection);
                    let mut conn_guard = transaction_connection.lock().await;
                    if let Some(mut conn) = conn_guard.0.take() {
                        // Roll back before removing its authorizer and other
                        // SQLite callbacks, then sanitize before pool reuse.
                        let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                        let still_in_transaction =
                            callbacks::sqlite_connection_in_transaction(&mut conn)
                                .await
                                .unwrap_or(true);
                        if still_in_transaction {
                            conn.close_on_drop();
                            drop(conn);
                        } else if let Err(error) =
                            callbacks::release_callback_connection_after_cleanup(
                                &callback_context,
                                conn,
                            )
                            .await
                        {
                            cleanup_error = Some(error);
                        }
                    }
                    let mut trans_guard = transaction_state.lock().await;
                    *trans_guard = TransactionState::None;
                    drop(trans_guard);
                    let mut ex_guard = explicit_transaction.lock().await;
                    *ex_guard = false;
                }

                callbacks::clear_active_handle(&callback_context.callback_connection);
                let callback_connection = {
                    let mut callback_guard = callback_context.callback_connection.lock().await;
                    callback_guard.0.take()
                };
                if let Some(connection) = callback_connection {
                    if let Err(error) = callbacks::release_callback_connection_after_cleanup(
                        &callback_context,
                        connection,
                    )
                    .await
                    {
                        cleanup_error.get_or_insert(error);
                    }
                }
                callbacks::clear_callback_state(&callback_context);

                // Release our reference to the pool (do not close: pool is shared via global registry).
                if let Some(identity) = pool_registry_identity.as_deref() {
                    close_registered_pool_if_last(identity).await;
                }
                pool_registry_lease.lock().unwrap().take();

                if let Some(error) = cleanup_error {
                    Err(error)
                } else {
                    Ok(())
                }
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// No-op for aiosqlite API compatibility. Does nothing; use close() to close the connection.
    fn stop(&self) -> PyResult<()> {
        Ok(())
    }

    /// Begin a transaction.
    fn begin(self_: PyRef<Self>) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let explicit_transaction = Arc::clone(&self_.explicit_transaction);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let callback_features = Arc::clone(&self_.callback_features);
        let trace_callback = Arc::clone(&self_.trace_callback);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let timeout = Arc::clone(&self_.timeout);
        let isolation_level = Arc::clone(&self_.isolation_level);
        let closed = Arc::clone(&self_.closed);
        let callback_context = self_.callback_context();
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;
        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                let has_callbacks_flag = callbacks_enabled(&callback_features);
                // Keep callback operation locking ahead of transaction-state
                // locking, matching the transaction context manager's order.
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                // Implicit → explicit: if we have implicit transaction only, commit it and reuse conn for explicit.
                {
                    let trans_guard = transaction_state.lock().await;
                    let ex_guard = explicit_transaction.lock().await;
                    let implicit_only = *trans_guard == TransactionState::Active && !*ex_guard;
                    drop(ex_guard);
                    if implicit_only {
                        let mut conn_guard = transaction_connection.lock().await;
                        if let Some(mut conn) = conn_guard.0.take() {
                            drop(trans_guard);
                            let trace_cb = trace_callback.clone_callback();
                            if let Some(cb) = trace_cb {
                                #[allow(deprecated)]
                                Python::attach(|py| {
                                    let _ = cb.bind(py).call1(("COMMIT",));
                                });
                            }
                            if let Err(error) = sqlx::query("COMMIT").execute(&mut *conn).await {
                                let still_in_transaction =
                                    callbacks::sqlite_connection_in_transaction(&mut conn)
                                        .await
                                        .unwrap_or(true);
                                if still_in_transaction {
                                    // Deferred constraints and some other
                                    // COMMIT errors leave SQLite's transaction
                                    // open. Keep it reachable so rollback can
                                    // recover it instead of returning it to the
                                    // pool in an active transaction.
                                    conn_guard.0 = Some(conn);
                                } else {
                                    drop(conn_guard);
                                    if has_callbacks_flag {
                                        callbacks::finish_transaction_callback_connection(
                                            &callback_context,
                                            conn,
                                        )
                                        .await;
                                    } else {
                                        drop(conn);
                                    }
                                    *transaction_state.lock().await = TransactionState::None;
                                    *explicit_transaction.lock().await = false;
                                }
                                return Err(map_sqlx_error(error, &path, "COMMIT"));
                            }
                            let timeout_ms = {
                                let g = timeout.lock().unwrap();
                                (*g * 1000.0) as i64
                            };
                            let _ = sqlx::query(&format!("PRAGMA busy_timeout = {timeout_ms}"))
                                .execute(&mut *conn)
                                .await;
                            let level = isolation_level
                                .lock()
                                .unwrap()
                                .clone()
                                .unwrap_or_else(|| "IMMEDIATE".to_string());
                            let begin_sql = format!("BEGIN {level}");
                            let trace_cb = trace_callback.clone_callback();
                            if let Some(cb) = trace_cb {
                                #[allow(deprecated)]
                                Python::attach(|py| {
                                    let _ = cb.bind(py).call1((begin_sql.as_str(),));
                                });
                            }
                            let begin_result = sqlx::query(&begin_sql)
                                .execute(&mut *conn)
                                .await
                                .map_err(|e| map_sqlx_error(e, &path, &begin_sql));
                            if let Err(error) = begin_result {
                                callbacks::clear_active_handle(&callback_connection);
                                callbacks::clear_active_handle(&transaction_connection);
                                let still_in_transaction =
                                    callbacks::sqlite_connection_in_transaction(&mut conn)
                                        .await
                                        .unwrap_or(true);
                                if still_in_transaction {
                                    conn_guard.0 = Some(conn);
                                } else {
                                    drop(conn_guard);
                                    if has_callbacks_flag {
                                        callbacks::finish_transaction_callback_connection(
                                            &callback_context,
                                            conn,
                                        )
                                        .await;
                                    } else {
                                        drop(conn);
                                    }
                                    *transaction_state.lock().await = TransactionState::None;
                                    *explicit_transaction.lock().await = false;
                                }
                                return Err(error);
                            }
                            conn_guard.0 = Some(conn);
                            let mut tguard = transaction_state.lock().await;
                            *tguard = TransactionState::Active;
                            drop(tguard);
                            let mut eguard = explicit_transaction.lock().await;
                            *eguard = true;
                            return Ok(());
                        }
                    } else if trans_guard.is_active() {
                        return Err(OperationalError::new_err("Transaction already in progress"));
                    }
                } // Lock released

                let mut pending_conn = PoolConnectionSlot::default();
                let mut reserved_transaction = false;

                // Release session connection so we don't hold session + transaction
                release_session_connection(&session_connection).await;

                let result: Result<(), PyErr> = async {
                    // Ensure pool exists before acquiring transaction connection
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                        &idle_timeout_secs,
                    )
                    .await?;

                    // Keep the state lock through connection acquisition and
                    // BEGIN. Routing operations then wait until the transaction
                    // connection is installed instead of seeing an empty slot.
                    let mut trans_guard = transaction_state.lock().await;
                    if trans_guard.is_active() {
                        return Err(OperationalError::new_err("Transaction already in progress"));
                    }
                    *trans_guard = TransactionState::Starting;
                    reserved_transaction = true;

                    // Check if callbacks are set - if so, use callback connection for transaction
                    if has_callbacks_flag {
                        callbacks::rebind_callbacks(callback_context.clone()).await?;
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.0.take().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        pending_conn.0 = Some(conn);
                    } else {
                        let pool_size_val = {
                            let g = pool_size.lock().unwrap();
                            *g
                        };
                        let timeout_val = {
                            let g = connection_timeout_secs.lock().unwrap();
                            *g
                        };
                        let conn = acquire_with_pragmas(
                            &pool_clone,
                            &pragmas,
                            &path,
                            pool_size_val,
                            timeout_val,
                        )
                        .await?;
                        pending_conn.0 = Some(conn);
                    }

                    let conn = pending_conn.0.as_mut().ok_or_else(|| {
                        InternalError::new_err("internal error: pending_conn not set before BEGIN")
                    })?;

                    // Set PRAGMA busy_timeout on this connection to handle lock contention
                    // Convert timeout from seconds (float) to milliseconds (integer) for SQLite
                    let timeout_ms = {
                        let timeout_guard = timeout.lock().unwrap();
                        (*timeout_guard * 1000.0) as i64
                    };
                    let busy_timeout_query = format!("PRAGMA busy_timeout = {}", timeout_ms);
                    sqlx::query(&busy_timeout_query)
                        .execute(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, &busy_timeout_query))?;

                    let level = isolation_level
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| "IMMEDIATE".to_string());
                    let begin_sql = format!("BEGIN {level}");
                    let trace_cb = trace_callback.clone_callback();
                    if let Some(cb) = trace_cb {
                        #[allow(deprecated)]
                        Python::attach(|py| {
                            let _ = cb.bind(py).call1((begin_sql.as_str(),));
                        });
                    }
                    sqlx::query(&begin_sql)
                        .execute(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, &begin_sql))?;

                    // Store the connection for reuse in all transaction operations
                    {
                        let mut conn_guard = transaction_connection.lock().await;
                        conn_guard.0 = pending_conn.0.take();
                    }

                    *trans_guard = TransactionState::Active;
                    *explicit_transaction.lock().await = true;
                    drop(trans_guard);
                    drop(_callback_operation_guard);

                    // Run init_hook after transaction is active so hook's conn.execute() uses this connection
                    execute_init_hook_if_needed(
                        &init_hook,
                        &init_hook_present,
                        &init_hook_called,
                        init_hook_reentrant,
                        connection_self,
                    )
                    .await?;
                    Ok(())
                }
                .await;

                if result.is_err() && reserved_transaction {
                    callbacks::clear_active_handle(&callback_connection);
                    callbacks::clear_active_handle(&transaction_connection);
                    let mut trans_guard = transaction_state.lock().await;
                    let mut trans_conn_guard = transaction_connection.lock().await;
                    let conn = trans_conn_guard.0.take().or_else(|| pending_conn.0.take());
                    drop(trans_conn_guard);
                    if let Some(conn) = conn {
                        callbacks::cleanup_failed_transaction_connection(&callback_context, conn)
                            .await;
                    }
                    *trans_guard = TransactionState::None;
                    *explicit_transaction.lock().await = false;
                }

                result
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Commit the current transaction.
    fn commit(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let callback_operation_lock = Arc::clone(&self.callback_operation_lock);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let callback_context = self.callback_context();
        let explicit_transaction = Arc::clone(&self.explicit_transaction);
        let callback_connection_required = Arc::clone(&self.callback_connection_required);
        let user_functions = Arc::clone(&self.user_functions);
        let user_aggregates = Arc::clone(&self.user_aggregates);
        let user_collations = Arc::clone(&self.user_collations);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                let _callback_operation_guard = callback_operation_lock.lock().await;
                let mut trans_guard = transaction_state.lock().await;
                // DBAPI compat: commit() is a no-op when not in an explicit transaction
                if *trans_guard != TransactionState::Active {
                    return Ok(());
                }

                // Callback registrations are rebound to a checked-out handle on the next operation.
                let has_callbacks_flag = has_callbacks(
                    &callback_connection_required,
                    &user_functions,
                    &user_aggregates,
                    &user_collations,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Retrieve the stored transaction connection; if missing, treat as no-op (DBAPI compat)
                callbacks::clear_active_handle(&callback_connection);
                callbacks::clear_active_handle(&transaction_connection);
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = match conn_guard.0.take() {
                    Some(c) => c,
                    None => {
                        *trans_guard = TransactionState::None;
                        drop(trans_guard);
                        let mut ex_guard = explicit_transaction.lock().await;
                        *ex_guard = false;
                        return Ok(());
                    }
                };

                // Execute COMMIT on the same connection that started the transaction
                let trace_cb = trace_callback.clone_callback();
                if let Some(cb) = trace_cb {
                    #[allow(deprecated)]
                    Python::attach(|py| {
                        let _ = cb.bind(py).call1(("COMMIT",));
                    });
                }
                if let Err(error) = sqlx::query("COMMIT").execute(&mut *conn).await {
                    if callbacks::sqlite_connection_in_transaction(&mut conn)
                        .await
                        .unwrap_or(true)
                    {
                        conn_guard.0 = Some(conn);
                    } else {
                        drop(conn_guard);
                        callbacks::cleanup_failed_transaction_connection(&callback_context, conn)
                            .await;
                        *trans_guard = TransactionState::None;
                        drop(trans_guard);
                        *explicit_transaction.lock().await = false;
                    }
                    return Err(map_sqlx_error(error, &path, "COMMIT"));
                }

                if has_callbacks_flag {
                    callbacks::finish_transaction_callback_connection(&callback_context, conn)
                        .await;
                } else {
                    drop(conn);
                }

                *trans_guard = TransactionState::None;
                drop(trans_guard);
                let mut ex_guard = explicit_transaction.lock().await;
                *ex_guard = false;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Rollback the current transaction.
    fn rollback(&self) -> PyResult<Py<PyAny>> {
        let path = self.path.clone();
        let callback_operation_lock = Arc::clone(&self.callback_operation_lock);
        let transaction_state = Arc::clone(&self.transaction_state);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let callback_connection = Arc::clone(&self.callback_connection);
        let callback_context = self.callback_context();
        let explicit_transaction = Arc::clone(&self.explicit_transaction);
        // Callback registrations are rebound to a checked-out handle on the next operation.
        let callback_connection_required = Arc::clone(&self.callback_connection_required);
        let user_functions = Arc::clone(&self.user_functions);
        let user_aggregates = Arc::clone(&self.user_aggregates);
        let user_collations = Arc::clone(&self.user_collations);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        Python::attach(|py| {
            let future = async move {
                let _callback_operation_guard = callback_operation_lock.lock().await;
                let mut trans_guard = transaction_state.lock().await;
                // DBAPI compat: rollback() is a no-op when not in an explicit transaction
                if *trans_guard != TransactionState::Active {
                    return Ok(());
                }

                // Check whether this physical connection must be discarded after rollback.
                let has_callbacks_flag = has_callbacks(
                    &callback_connection_required,
                    &user_functions,
                    &user_aggregates,
                    &user_collations,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                );

                // Retrieve the stored transaction connection; if missing, treat as no-op (DBAPI compat)
                callbacks::clear_active_handle(&callback_connection);
                callbacks::clear_active_handle(&transaction_connection);
                let mut conn_guard = transaction_connection.lock().await;
                let mut conn = match conn_guard.0.take() {
                    Some(c) => c,
                    None => {
                        *trans_guard = TransactionState::None;
                        drop(trans_guard);
                        let mut ex_guard = explicit_transaction.lock().await;
                        *ex_guard = false;
                        return Ok(());
                    }
                };

                // Execute ROLLBACK on the same connection that started the transaction
                let trace_cb = trace_callback.clone_callback();
                if let Some(cb) = trace_cb {
                    #[allow(deprecated)]
                    Python::attach(|py| {
                        let _ = cb.bind(py).call1(("ROLLBACK",));
                    });
                }
                if let Err(error) = sqlx::query("ROLLBACK").execute(&mut *conn).await {
                    if callbacks::sqlite_connection_in_transaction(&mut conn)
                        .await
                        .unwrap_or(true)
                    {
                        conn_guard.0 = Some(conn);
                    } else {
                        drop(conn_guard);
                        callbacks::cleanup_failed_transaction_connection(&callback_context, conn)
                            .await;
                        *trans_guard = TransactionState::None;
                        drop(trans_guard);
                        *explicit_transaction.lock().await = false;
                    }
                    return Err(map_sqlx_error(error, &path, "ROLLBACK"));
                }

                if has_callbacks_flag {
                    callbacks::finish_transaction_callback_connection(&callback_context, conn)
                        .await;
                } else {
                    drop(conn);
                }

                *trans_guard = TransactionState::None;
                drop(trans_guard);
                let mut ex_guard = explicit_transaction.lock().await;
                *ex_guard = false;
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
    /// .. code-block:: python
    ///
    ///     # Simple query
    ///     await conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
    ///
    ///     # With positional parameters
    ///     await conn.execute("INSERT INTO users (name) VALUES (?)", ["Alice"])
    ///
    ///     # With named parameters
    ///     await conn.execute(
    ///         "INSERT INTO users (name, email) VALUES (:name, :email)",
    ///         {"name": "Bob", "email": "bob@example.com"}
    ///     )
    ///
    ///     # Using as context manager (returns cursor)
    ///     async with conn.execute("SELECT * FROM users") as cursor:
    ///         rows = await cursor.fetchall()
    #[pyo3(signature = (query, parameters = None, cursor = None))]
    fn execute(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
        cursor: Option<&Bound<'_, Cursor>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let session_connection = self_.session_connection.clone();
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_start_lock = Arc::clone(&self_.transaction_start_lock);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let explicit_transaction = Arc::clone(&self_.explicit_transaction);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let trace_callback = Arc::clone(&self_.trace_callback);
        // Prepared statement cache tracking (Phase 2.13)
        let query_usage_stats = Arc::clone(&self_.query_usage_stats);
        let query_usage_enabled = Arc::clone(&self_.query_usage_enabled);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let row_factory = Arc::clone(&self_.row_factory);
        let text_factory = Arc::clone(&self_.text_factory);
        let timeout = Arc::clone(&self_.timeout);
        let isolation_level = Arc::clone(&self_.isolation_level);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let converters = Arc::clone(&self_.converters);
        let include_query_in_errors = Arc::clone(&self_.include_query_in_errors);
        let callback_context = self_.callback_context();
        let connection_self: Py<Connection> = self_.into();

        // Raise immediately if connection is closed (cursor.execute() on closed connection, etc.)
        ensure_not_closed(&closed)?;

        // Clone query before processing (it may be moved)
        let original_query = query.clone();

        // Process parameters (sync; Python::attach acceptable here)
        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;

        // Track query usage for prepared statement cache analytics (Phase 2.13)
        track_query_usage_if_enabled(&query_usage_enabled, &query_usage_stats, &processed_query);

        // Check if this is a SELECT query (for lazy execution)
        let is_select = is_select_query(&processed_query);
        // True for INSERT/UPDATE/DELETE ... RETURNING; needed to fetch and cache rows to prevent re-execution
        let returns_result_rows = returns_result_rows(&processed_query);

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
        // When cursor is provided (from Cursor.execute()), reuse it and set processed state.
        // Otherwise create a new cursor. This allows await cursor.execute() to return self (aiosqlite compat).
        let cursor_arg = cursor.map(|c| c.clone().unbind());
        #[allow(deprecated)]
        let cursor = Python::attach(|py| -> PyResult<Py<Cursor>> {
            if let Some(c) = cursor_arg {
                let mut c_ref = c.borrow_mut(py);
                if *c_ref.cursor_closed.lock().unwrap() {
                    return Err(ProgrammingError::new_err(
                        "Cannot operate on a closed cursor.",
                    ));
                }
                c_ref.query = original_query.clone();
                *c_ref.parameters.lock().unwrap() = params_for_cursor;
                c_ref.processed_query = Some(processed_query.clone());
                c_ref.processed_params = Some(param_values.clone());
                *c_ref.current_index.lock().unwrap() = 0;
                *c_ref.results.lock().unwrap() = None;
                *c_ref.description.lock().unwrap() = None;
                *c_ref.pending_description.lock().unwrap() = None;
                *c_ref.lastrowid.lock().unwrap() = -1;
                *c_ref.rowcount.lock().unwrap() = -1;
                Ok(c.clone_ref(py))
            } else {
                let new_cursor = Cursor {
                    connection: connection_self.clone_ref(py),
                    query: original_query.clone(),
                    results: Arc::new(StdMutex::new(None)),
                    current_index: Arc::new(StdMutex::new(0)),
                    parameters: Arc::new(StdMutex::new(params_for_cursor)),
                    processed_query: Some(processed_query.clone()),
                    processed_params: Some(param_values.clone()),
                    connection_path: path.clone(),
                    connection_pool: Arc::clone(&pool),
                    connection_pragmas: Arc::clone(&pragmas),
                    pool_size: Arc::clone(&pool_size),
                    connection_timeout_secs: Arc::clone(&connection_timeout_secs),
                    idle_timeout_secs: Arc::clone(&idle_timeout_secs),
                    row_factory: Arc::clone(&row_factory),
                    text_factory: Arc::clone(&text_factory),
                    transaction_state: Arc::clone(&transaction_state),
                    transaction_connection: Arc::clone(&transaction_connection),
                    session_connection: session_connection.clone(),
                    callback_connection: Arc::clone(&callback_connection),
                    callback_context: callback_context.clone(),
                    adapters: Arc::clone(&adapters),
                    converters: Arc::clone(&converters),
                    include_query_in_errors: Arc::clone(&include_query_in_errors),
                    arraysize: Arc::new(StdMutex::new(1)),
                    description: Arc::new(StdMutex::new(None)),
                    pending_description: Arc::new(StdMutex::new(None)),
                    lastrowid: Arc::new(StdMutex::new(-1)),
                    rowcount: Arc::new(StdMutex::new(-1)),
                    row_factory_override: Arc::new(StdMutex::new(None)),
                    cursor_closed: Arc::new(StdMutex::new(false)),
                    closed: Arc::clone(&closed),
                };
                Py::new(py, new_cursor)
            }
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
        // Note: Python::attach is used here for sync context manager creation before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        // Note: Python::attach is used here for sync result conversion in async context.
        // The deprecation warning is acceptable as this is a sync operation within async.
        #[allow(deprecated)]
        Python::attach(|py| -> PyResult<Py<PyAny>> {
            let state = ConnectionExecutionState {
                path,
                pool: Arc::clone(&pool),
                session_connection: session_connection.clone(),
                pragmas: Arc::clone(&pragmas),
                pool_size: Arc::clone(&pool_size),
                connection_timeout_secs: Arc::clone(&connection_timeout_secs),
                idle_timeout_secs: Arc::clone(&idle_timeout_secs),
                transaction_state: Arc::clone(&transaction_state),
                transaction_start_lock: Arc::clone(&transaction_start_lock),
                transaction_connection: Arc::clone(&transaction_connection),
                callback_connection: Arc::clone(&callback_connection),
                callback_context: callback_context.clone(),
                adapters: Arc::clone(&adapters),
                converters: Arc::clone(&converters),
                trace_callback: Arc::clone(&trace_callback),
                init_hook: Arc::clone(&init_hook),
                init_hook_called: Arc::clone(&init_hook_called),
                init_hook_present: Arc::clone(&init_hook_present),
                last_rowid: Arc::clone(&last_rowid),
                last_changes: Arc::clone(&last_changes),
                timeout,
                isolation_level,
                include_query_in_errors: Arc::clone(&include_query_in_errors),
                closed,
                explicit_transaction: Arc::clone(&explicit_transaction),
            };
            let ctx_mgr = ExecuteContextManager {
                state,
                cursor: cursor.clone_ref(py),
                query: processed_query,
                param_values,
                is_select,
                returns_result_rows,
                connection: connection_self.clone_ref(py),
            };
            Py::new(py, ctx_mgr).map(|c| c.into())
        })
    }

    /// Execute a query multiple times with different parameters.
    fn execute_many(
        self_: PyRef<Self>,
        query: String,
        parameters: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let _timeout = Arc::clone(&self_.timeout);
        let adapters = Arc::clone(&self_.adapters);
        let query_usage_stats = Arc::clone(&self_.query_usage_stats);
        let query_usage_enabled = Arc::clone(&self_.query_usage_enabled);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let callback_context = self_.callback_context();
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        // Process parameter sets before entering the async future. Row mappings
        // are parsed with the same named-placeholder logic as execute(); other
        // iterable rows are normalized to positional lists.
        #[allow(deprecated)]
        let (query, processed_params) =
            Python::attach(|py| -> PyResult<(String, Vec<Vec<SqliteParam>>)> {
                let mut result = Vec::new();
                let mut processed_query: Option<String> = None;
                for param_set in parameters.try_iter()? {
                    let param_set = param_set?;
                    let row_parameters = if param_set.hasattr("keys")? {
                        py.get_type::<PyDict>().call1((param_set,))?
                    } else {
                        let values = param_set
                            .try_iter()?
                            .collect::<PyResult<Vec<Bound<'_, PyAny>>>>()?;
                        PyList::new(py, values)?.into_any()
                    };
                    let (row_query, row_values) =
                        process_parameters(py, &query, Some(&row_parameters), Some(&adapters))?;
                    if let Some(expected_query) = &processed_query {
                        if expected_query != &row_query {
                            return Err(ProgrammingError::new_err(
                                "executemany parameter sets must use the same binding style",
                            ));
                        }
                    } else {
                        processed_query = Some(row_query);
                    }
                    result.push(row_values);
                }
                Ok((processed_query.unwrap_or(query), result))
            })?;

        track_query_usage_count_if_enabled(
            &query_usage_enabled,
            &query_usage_stats,
            &query,
            processed_params.len(),
        );

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Priority: transaction > callbacks > pool
                // Note: Only check for Active state, not Starting (Starting means transaction is being set up,
                // and init_hook may need to execute queries using pool connection)
                let mut in_transaction = transaction_state.is_exact_active().await;

                // Resolve the pool once before init hooks; the common session path
                // reuses this handle rather than locking the pool slot again.
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };

                let mut total_changes = 0u64;
                let mut last_row_id = 0i64;

                if in_transaction {
                    // Use stored transaction connection. Release lock each iteration
                    // to match the execute-in-loop pattern (lock -> use -> release).
                    for param_values in processed_params.iter() {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        let result = bind_and_execute_on_connection(
                            &query,
                            param_values,
                            conn,
                            &path,
                            include_query_in_errors,
                        )
                        .await?;
                        total_changes += result.rows_affected();
                        last_row_id = result.last_insert_rowid();
                        drop(conn_guard);
                    }
                } else if has_callbacks_flag {
                    let execution_result: Result<(), PyErr> = async {
                        callbacks::rebind_callbacks(callback_context.clone()).await?;
                        if processed_params.is_empty() {
                            return Ok(());
                        }

                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        sqlx::query("BEGIN")
                            .execute(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, "BEGIN"))?;

                        let batch_result: Result<(), PyErr> = async {
                            for param_values in processed_params.iter() {
                                let result = bind_and_execute_on_connection(
                                    &query,
                                    param_values,
                                    conn,
                                    &path,
                                    include_query_in_errors,
                                )
                                .await?;
                                total_changes += result.rows_affected();
                                last_row_id = result.last_insert_rowid();
                            }
                            sqlx::query("COMMIT")
                                .execute(&mut **conn)
                                .await
                                .map_err(|e| map_sqlx_error(e, &path, "COMMIT"))?;
                            Ok(())
                        }
                        .await;

                        if batch_result.is_err()
                            && sqlx::query("ROLLBACK").execute(&mut **conn).await.is_err()
                        {
                            if let Some(mut conn) = conn_guard.0.take() {
                                conn.close_on_drop();
                            }
                        }
                        batch_result
                    }
                    .await;
                    callbacks::discard_callback_connection(&callback_context).await;
                    execution_result?;
                } else {
                    // Reuse the connection pinned to this logical Connection. Acquiring another
                    // pool connection here can deadlock when pool_size=1 because the session
                    // connection remains checked out between operations.
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for session query")
                    })?;
                    let mut conn_guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = &mut *conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock handle: {e}"))
                    })?;
                    let raw_db = handle.as_raw_handle().as_ptr();
                    let result = tokio::task::block_in_place(|| {
                        crate::batch::execute_many_raw_core(raw_db, &query, &processed_params)
                    });
                    drop(handle);
                    let (total_changes_val, last_row_id_val) = result.map_err(|(rc, msg)| {
                        crate::errors::map_sqlite_error_from_msg(
                            &path,
                            &query,
                            rc,
                            &msg,
                            include_query_in_errors,
                        )
                    })?;
                    total_changes = total_changes_val;
                    last_row_id = last_row_id_val;
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
    /// .. code-block:: python
    ///
    ///     # Default (list format)
    ///     rows = await conn.fetch_all("SELECT * FROM users")
    ///     # rows = [[1, "Alice"], [2, "Bob"]]
    ///
    ///     # With dict factory
    ///     conn.row_factory = "dict"
    ///     rows = await conn.fetch_all("SELECT * FROM users")
    ///     # rows = [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]
    ///
    ///     # With parameters
    ///     rows = await conn.fetch_all("SELECT * FROM users WHERE id > ?", [5])
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
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let row_factory = Arc::clone(&self_.row_factory);
        let text_factory = Arc::clone(&self_.text_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let trace_callback = Arc::clone(&self_.trace_callback);
        // Prepared statement cache tracking (Phase 2.13)
        let query_usage_stats = Arc::clone(&self_.query_usage_stats);
        let query_usage_enabled = Arc::clone(&self_.query_usage_enabled);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let converters = Arc::clone(&self_.converters);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let callback_context = self_.callback_context();
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        // Process parameters (sync; Python::attach acceptable here)
        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;

        // Track query usage for prepared statement cache analytics (Phase 2.13)
        track_query_usage_if_enabled(&query_usage_enabled, &query_usage_stats, &processed_query);

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;

                // Python-level trace callback.
                let trace_cb = trace_callback.clone_callback();
                if let Some(cb) = trace_cb {
                    #[allow(deprecated)]
                    Python::attach(|py| {
                        let _ = cb.bind(py).call1((processed_query.as_str(),));
                    });
                }
                // Priority: transaction > callbacks > pool
                let mut in_transaction = transaction_state.is_routing_active().await;

                // Resolve once before init hooks and reuse for session acquisition.
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                let rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                } else if has_callbacks_flag {
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let rows_result = bind_and_fetch_all_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await;
                    drop(conn_guard);
                    callbacks::discard_callback_connection(&callback_context).await;
                    rows_result?
                } else {
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for session query")
                    })?;
                    let mut conn_guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    bind_and_fetch_all_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                };

                // Convert rows using row_factory
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let guard = row_factory.lock().unwrap();
                    let factory_opt = guard.as_ref();
                    let tf_guard = text_factory.lock().unwrap();
                    let tf_opt = tf_guard.as_ref();
                    let conv_opt = Some(&converters);
                    let result_list = PyList::empty(py);
                    for row in rows.iter() {
                        let out = row_to_py_with_factory(py, row, factory_opt, tf_opt, conv_opt)?;
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
    /// .. code-block:: python
    ///
    ///     # Fetch user by ID (expects exactly one)
    ///     user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [1])
    ///     # user = [1, "Alice"]  # or dict/Row depending on row_factory
    ///
    ///     # This will raise if user doesn't exist
    ///     try:
    ///         user = await conn.fetch_one("SELECT * FROM users WHERE id = ?", [999])
    ///     except ProgrammingError:
    ///         print("User not found")
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
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let row_factory = Arc::clone(&self_.row_factory);
        let text_factory = Arc::clone(&self_.text_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let trace_callback = Arc::clone(&self_.trace_callback);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let converters = Arc::clone(&self_.converters);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let callback_context = self_.callback_context();
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        // Process parameters (sync; Python::attach acceptable here)
        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;

                // Python-level trace callback.
                let trace_cb = trace_callback.clone_callback();
                if let Some(cb) = trace_cb {
                    #[allow(deprecated)]
                    Python::attach(|py| {
                        let _ = cb.bind(py).call1((processed_query.as_str(),));
                    });
                }
                // Priority: transaction > callbacks > pool
                let mut in_transaction = transaction_state.is_routing_active().await;

                // Resolve once before init hooks and reuse for session acquisition.
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                let row = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_one_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                } else if has_callbacks_flag {
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let row_result = bind_and_fetch_one_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await;
                    drop(conn_guard);
                    callbacks::discard_callback_connection(&callback_context).await;
                    row_result?
                } else {
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for session query")
                    })?;
                    let mut conn_guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    bind_and_fetch_one_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                };

                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let guard = row_factory.lock().unwrap();
                    let factory_opt = guard.as_ref();
                    let tf_guard = text_factory.lock().unwrap();
                    let tf_opt = tf_guard.as_ref();
                    let conv_opt = Some(&converters);
                    let out = row_to_py_with_factory(py, &row, factory_opt, tf_opt, conv_opt)?;
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
    /// .. code-block:: python
    ///
    ///     # Fetch user by ID (may not exist)
    ///     user = await conn.fetch_optional("SELECT * FROM users WHERE id = ?", [1])
    ///     if user:
    ///         print(f"Found: {user}")
    ///     else:
    ///         print("User not found")
    ///
    ///     # Safe for optional lookups
    ///     user = await conn.fetch_optional(
    ///         "SELECT * FROM users WHERE email = ?",
    ///         ["alice@example.com"]
    ///     )
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
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let row_factory = Arc::clone(&self_.row_factory);
        let text_factory = Arc::clone(&self_.text_factory);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let converters = Arc::clone(&self_.converters);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let callback_context = self_.callback_context();
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        // Process parameters (sync; Python::attach acceptable here)
        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Priority: transaction > callbacks > pool
                let mut in_transaction = transaction_state.is_routing_active().await;

                // Resolve once before init hooks and reuse for session acquisition.
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };

                // Execute init_hook if needed (before any operations)
                execute_init_hook_if_needed(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                let opt = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                } else if has_callbacks_flag {
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let opt_result = bind_and_fetch_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await;
                    drop(conn_guard);
                    callbacks::discard_callback_connection(&callback_context).await;
                    opt_result?
                } else {
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for session query")
                    })?;
                    let mut conn_guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    bind_and_fetch_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                };

                match opt {
                    Some(row) => Python::attach(|py| -> PyResult<Py<PyAny>> {
                        let guard = row_factory.lock().unwrap();
                        let factory_opt = guard.as_ref();
                        let tf_guard = text_factory.lock().unwrap();
                        let tf_opt = tf_guard.as_ref();
                        let conv_opt = Some(&converters);
                        let out = row_to_py_with_factory(py, &row, factory_opt, tf_opt, conv_opt)?;
                        Ok(out.unbind())
                    }),
                    None => Python::attach(|py| -> PyResult<Py<PyAny>> { Ok(py.None()) }),
                }
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Execute a narrow, opt-in raw SQLite scalar lookup. This bypasses SQLx's
    /// general query future and result-row construction while retaining the
    /// existing pool/session locking and transaction routing.
    #[pyo3(signature = (query, parameters = None, blob = false))]
    fn raw_fetch_scalar(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
        blob: bool,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let callback_operation_lock = Arc::clone(&self_.callback_operation_lock);
        let callback_features = Arc::clone(&self_.callback_features);
        let trace_callback = Arc::clone(&self_.trace_callback);
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let query_usage_stats = Arc::clone(&self_.query_usage_stats);
        let query_usage_enabled = Arc::clone(&self_.query_usage_enabled);
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;
        track_query_usage_if_enabled(&query_usage_enabled, &query_usage_stats, &processed_query);

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                invoke_trace_callback(&trace_callback, &processed_query);
                if callbacks_enabled(&callback_features) {
                    return Err(NotSupportedError::new_err(
                        "raw_fetch_scalar() is unavailable when SQLite callbacks are configured",
                    ));
                }

                let mut in_transaction = transaction_state.is_routing_active().await;
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };
                execute_init_hook_if_needed_fast(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;
                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }
                // Serialize raw-handle use with callback registration/removal.
                // Callback handoff can otherwise move or finalize a cached
                // statement while this synchronous SQLite operation is using it.
                let _callback_operation_guard =
                    match Arc::clone(&callback_operation_lock).try_lock_owned() {
                        Ok(guard) => guard,
                        Err(_) => callback_operation_lock.clone().lock_owned().await,
                    };
                if callbacks_enabled(&callback_features) {
                    return Err(NotSupportedError::new_err(
                        "raw_fetch_scalar() is unavailable when SQLite callbacks are configured",
                    ));
                }

                let raw = if in_transaction {
                    let mut guard = transaction_connection.lock().await;
                    let conn = guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock SQLite handle: {e}"))
                    })?;
                    let db = handle.as_raw_handle().as_ptr();
                    callbacks::register_active_handle(&transaction_connection, db as usize);
                    let result = tokio::task::block_in_place(|| {
                        crate::batch::fetch_scalar_raw_core(
                            db,
                            &processed_query,
                            &param_values,
                            blob,
                            None,
                        )
                    });
                    callbacks::clear_active_handle(&transaction_connection);
                    result
                } else {
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for raw query")
                    })?;
                    let mut guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock SQLite handle: {e}"))
                    })?;
                    let db = handle.as_raw_handle().as_ptr();
                    let active_slot = session_connection.raw_slot();
                    callbacks::register_active_handle(&active_slot, db as usize);
                    let statement_cache = if session_connection.retain() {
                        Some(session_connection.raw_statement_cache())
                    } else {
                        None
                    };
                    let result = tokio::task::block_in_place(|| {
                        crate::batch::fetch_scalar_raw_core(
                            db,
                            &processed_query,
                            &param_values,
                            blob,
                            statement_cache.as_deref(),
                        )
                    });
                    callbacks::clear_active_handle(&active_slot);
                    result
                };

                let raw = raw.map_err(|(rc, message)| {
                    if blob && rc & 0xff == libsqlite3_sys::SQLITE_MISMATCH {
                        return pyo3::exceptions::PyTypeError::new_err(message);
                    }
                    if rc & 0xff == libsqlite3_sys::SQLITE_ERROR {
                        if let Some(columns) = message
                            .strip_prefix("raw scalar queries must return exactly one column; got ")
                        {
                            return if columns == "0" {
                                ProgrammingError::new_err(
                                    "raw_fetch_scalar() requires a statement that returns rows",
                                )
                            } else {
                                ProgrammingError::new_err(format!(
                                    "raw_fetch_scalar() requires exactly one result column; got {columns}"
                                ))
                            };
                        }
                    }
                    map_sqlite_error_from_msg(
                        &path,
                        &processed_query,
                        rc,
                        &message,
                        include_query_in_errors,
                    )
                })?;
                #[allow(deprecated)]
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    Ok(match raw {
                        None | Some(RawScalar::Null) => py.None(),
                        Some(RawScalar::Integer(value)) => {
                            PyInt::new(py, value).into_any().unbind()
                        }
                        Some(RawScalar::Real(value)) => PyFloat::new(py, value).into_any().unbind(),
                        Some(RawScalar::Text(value)) => {
                            PyString::new(py, &value).into_any().unbind()
                        }
                        Some(RawScalar::Blob(value)) => {
                            PyBytes::new(py, &value).into_any().unbind()
                        }
                    })
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Initialize the schema used by SQLiteCache without routing through the
    /// Python DB-API cursor/factory path.
    #[pyo3(name = "_cache_initialize", signature = (table_name))]
    fn cache_initialize(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let operation = cache::initialize_operation(table_name)?;
        cache::start_cache_operation(self_, operation)
    }

    /// Read a live cache BLOB with Rust-side key binding and expiration time.
    #[pyo3(name = "_cache_get", signature = (table_name, key))]
    fn cache_get(self_: PyRef<Self>, table_name: String, key: String) -> PyResult<Py<PyAny>> {
        let operation = cache::get_operation(table_name, key)?;
        cache::start_cache_operation(self_, operation)
    }

    /// Store a cache BLOB using a Rust-side TTL clock and direct SQLx binding.
    #[pyo3(
        name = "_cache_set",
        signature = (table_name, key, value, ttl_seconds=None)
    )]
    fn cache_set(
        self_: PyRef<Self>,
        table_name: String,
        key: String,
        value: &Bound<'_, PyBytes>,
        ttl_seconds: Option<f64>,
    ) -> PyResult<Py<PyAny>> {
        let operation =
            cache::set_operation(table_name, key, value.as_bytes().to_vec(), ttl_seconds)?;
        cache::start_cache_operation(self_, operation)
    }

    /// Delete one cache key and return whether a row was removed.
    #[pyo3(name = "_cache_delete", signature = (table_name, key))]
    fn cache_delete(self_: PyRef<Self>, table_name: String, key: String) -> PyResult<Py<PyAny>> {
        let operation = cache::delete_operation(table_name, key)?;
        cache::start_cache_operation(self_, operation)
    }

    /// Delete a bounded number of expired cache rows.
    #[pyo3(
        name = "_cache_cleanup_expired",
        signature = (table_name, limit)
    )]
    fn cache_cleanup_expired(
        self_: PyRef<Self>,
        table_name: String,
        limit: i64,
    ) -> PyResult<Py<PyAny>> {
        let operation = cache::cleanup_expired_operation(table_name, limit)?;
        cache::start_cache_operation(self_, operation)
    }

    /// Fetch one scalar column without constructing a general row or applying a
    /// row factory. This is intended for repeated local lookups such as cache
    /// reads; use ``fetch_optional`` for DB-API-compatible row behavior.
    #[pyo3(signature = (query, parameters = None, *, _require_blob = false))]
    fn fetch_scalar(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
        _require_blob: bool,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let callback_connection = Arc::clone(&self_.callback_connection);
        let callback_features = Arc::clone(&self_.callback_features);
        let callback_context = self_.callback_context();
        let trace_callback = Arc::clone(&self_.trace_callback);
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let adapters = Arc::clone(&self_.adapters);
        let converters = Arc::clone(&self_.converters);
        let text_factory = Arc::clone(&self_.text_factory);
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        let query_usage_stats = Arc::clone(&self_.query_usage_stats);
        let query_usage_enabled = Arc::clone(&self_.query_usage_enabled);
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;
        track_query_usage_if_enabled(&query_usage_enabled, &query_usage_stats, &processed_query);

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                invoke_trace_callback(&trace_callback, &processed_query);
                let mut in_transaction = transaction_state.is_routing_active().await;
                let mut pool_for_operation = if !in_transaction {
                    Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    )
                } else {
                    None
                };
                execute_init_hook_if_needed_fast(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                in_transaction = transaction_state.is_routing_active().await;
                if !in_transaction && pool_for_operation.is_none() {
                    pool_for_operation = Some(
                        get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
                        )
                        .await?,
                    );
                }

                let has_callbacks_flag = callbacks_enabled(&callback_features);
                let callback_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                let row = if in_transaction {
                    let mut guard = transaction_connection.lock().await;
                    let conn = guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_fetch_scalar_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                } else if has_callbacks_flag {
                    let mut guard = callback_connection.lock().await;
                    let conn = guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let result = bind_and_fetch_scalar_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await;
                    drop(guard);
                    drop(callback_guard);
                    callbacks::discard_callback_connection(&callback_context).await;
                    result?
                } else {
                    let pool_clone = pool_for_operation.as_ref().ok_or_else(|| {
                        OperationalError::new_err("Pool not available for session query")
                    })?;
                    let mut guard = lock_session_connection_from_pool(
                        &path,
                        pool_clone,
                        &session_connection,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                    )
                    .await?;
                    let conn = guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Session connection not available")
                    })?;
                    bind_and_fetch_scalar_optional_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                };

                #[allow(deprecated)]
                Python::attach(|py| -> PyResult<Py<PyAny>> {
                    let Some(row) = row else {
                        return Ok(py.None());
                    };
                    if _require_blob {
                        let raw_value = row.try_get_raw(0).map_err(|error| {
                            ProgrammingError::new_err(format!(
                                "could not inspect SQLite result type: {error}"
                            ))
                        })?;
                        let type_info = raw_value.type_info();
                        if !raw_value.is_null() && type_info.name() != "BLOB" {
                            return Err(pyo3::exceptions::PyTypeError::new_err(
                                "fetch_blob() requires a BLOB or NULL result",
                            ));
                        }
                    }
                    let tf_guard = text_factory.lock().unwrap();
                    let value = crate::conversion::sqlite_value_to_py(
                        py,
                        &row,
                        0,
                        tf_guard.as_ref(),
                        Some(&converters),
                    )?;
                    Ok(value)
                })
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Execute an INSERT/UPDATE/DELETE and return the last insert row ID (aiosqlite-compatible helper).
    ///
    /// Runs the statement, updates `last_rowid`/`changes`, and returns `last_insert_rowid()`.
    /// Use for INSERTs; for UPDATE/DELETE the returned rowid is typically 0.
    #[pyo3(signature = (query, parameters = None))]
    fn execute_insert(
        self_: PyRef<Self>,
        query: String,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let callback_connection = Arc::clone(&self_.callback_connection);
        let last_rowid = Arc::clone(&self_.last_rowid);
        let last_changes = Arc::clone(&self_.last_changes);
        let adapters = Arc::clone(&self_.adapters);
        let callback_context = self_.callback_context();

        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        #[allow(deprecated)]
        let (processed_query, param_values) =
            Python::attach(|py| process_parameters(py, &query, parameters, Some(&adapters)))?;

        if is_select_query(&processed_query) {
            return Err(ProgrammingError::new_err(
                "execute_insert expects INSERT/UPDATE/DELETE, not SELECT",
            ));
        }

        let closed = Arc::clone(&self_.closed);
        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                let in_transaction = transaction_state.is_routing_active().await;

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                let result = if !in_transaction {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                        &idle_timeout_secs,
                    )
                    .await?;

                    if has_callbacks_flag {
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        let query_result = bind_and_execute_on_connection(
                            &processed_query,
                            &param_values,
                            conn,
                            &path,
                            include_query_in_errors,
                        )
                        .await;
                        drop(conn_guard);
                        callbacks::discard_callback_connection(&callback_context).await;
                        query_result?
                    } else {
                        let mut conn_guard = lock_session_connection_from_pool(
                            &path,
                            &pool_clone,
                            &session_connection,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                        )
                        .await?;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Session connection not available")
                        })?;
                        bind_and_execute_on_connection(
                            &processed_query,
                            &param_values,
                            conn,
                            &path,
                            include_query_in_errors,
                        )
                        .await?
                    }
                } else {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    bind_and_execute_on_connection(
                        &processed_query,
                        &param_values,
                        conn,
                        &path,
                        include_query_in_errors,
                    )
                    .await?
                };

                let rowid = result.last_insert_rowid();
                let changes = result.rows_affected();
                *last_rowid.lock().await = rowid;
                *last_changes.lock().await = changes;

                Ok(rowid)
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
        let idle_timeout_secs = Arc::clone(&slf.idle_timeout_secs);
        let row_factory = Arc::clone(&slf.row_factory);
        let text_factory = Arc::clone(&slf.text_factory);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let session_connection = slf.session_connection.clone();
        let callback_connection = Arc::clone(&slf.callback_connection);
        let callback_context = slf.callback_context();
        let closed = Arc::clone(&slf.closed);
        let adapters = Arc::clone(&slf.adapters);
        let converters = Arc::clone(&slf.converters);
        let include_query_in_errors = Arc::clone(&slf.include_query_in_errors);
        Ok(Cursor {
            connection: slf.into(),
            query: String::new(),
            results: Arc::new(StdMutex::new(None)),
            current_index: Arc::new(StdMutex::new(0)),
            parameters: Arc::new(StdMutex::new(None)),
            processed_query: None,  // No processed query for cursor() method
            processed_params: None, // No processed params for cursor() method
            connection_path: path,
            connection_pool: pool,
            connection_pragmas: pragmas,
            pool_size,
            connection_timeout_secs,
            idle_timeout_secs,
            row_factory,
            text_factory,
            transaction_state,
            transaction_connection,
            session_connection,
            callback_connection,
            callback_context,
            adapters,
            converters,
            include_query_in_errors,
            arraysize: Arc::new(StdMutex::new(1)),
            description: Arc::new(StdMutex::new(None)),
            pending_description: Arc::new(StdMutex::new(None)),
            lastrowid: Arc::new(StdMutex::new(-1)),
            rowcount: Arc::new(StdMutex::new(-1)),
            row_factory_override: Arc::new(StdMutex::new(None)),
            cursor_closed: Arc::new(StdMutex::new(false)),
            closed,
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
        let idle_timeout_secs = Arc::clone(&slf.idle_timeout_secs);
        let row_factory = Arc::clone(&slf.row_factory);
        let text_factory = Arc::clone(&slf.text_factory);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let session_connection = slf.session_connection.clone();
        let callback_connection = Arc::clone(&slf.callback_connection);
        let callback_context = slf.callback_context();
        let closed = Arc::clone(&slf.closed);
        let adapters = Arc::clone(&slf.adapters);
        let converters = Arc::clone(&slf.converters);
        let include_query_in_errors = Arc::clone(&slf.include_query_in_errors);
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
            idle_timeout_secs,
            row_factory,
            text_factory,
            transaction_state,
            transaction_connection,
            session_connection,
            callback_connection,
            callback_context,
            adapters,
            converters,
            include_query_in_errors,
            arraysize: Arc::new(StdMutex::new(1)),
            description: Arc::new(StdMutex::new(None)),
            pending_description: Arc::new(StdMutex::new(None)),
            lastrowid: Arc::new(StdMutex::new(-1)),
            rowcount: Arc::new(StdMutex::new(-1)),
            row_factory_override: Arc::new(StdMutex::new(None)),
            cursor_closed: Arc::new(StdMutex::new(false)),
            closed,
        })
    }

    /// Return an async context manager for a transaction.
    /// On __aenter__ calls begin(); on __aexit__ calls commit() or rollback().
    fn transaction(slf: PyRef<Self>) -> PyResult<TransactionContextManager> {
        let path = slf.path.clone();
        let pool = Arc::clone(&slf.pool);
        let session_connection = slf.session_connection.clone();
        let pragmas = Arc::clone(&slf.pragmas);
        let pool_size = Arc::clone(&slf.pool_size);
        let connection_timeout_secs = Arc::clone(&slf.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&slf.idle_timeout_secs);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let init_hook = Arc::clone(&slf.init_hook);
        let init_hook_called = Arc::clone(&slf.init_hook_called);
        let init_hook_present = Arc::clone(&slf.init_hook_present);
        let timeout = Arc::clone(&slf.timeout);
        let isolation_level = Arc::clone(&slf.isolation_level);
        let explicit_transaction = Arc::clone(&slf.explicit_transaction);
        let trace_callback = Arc::clone(&slf.trace_callback);
        let callback_context = slf.callback_context();
        let connection: Py<Connection> = slf.into();
        Ok(TransactionContextManager {
            path,
            pool,
            session_connection,
            pragmas,
            pool_size,
            connection_timeout_secs,
            idle_timeout_secs,
            transaction_state,
            transaction_connection,
            connection,
            init_hook,
            init_hook_called,
            init_hook_present,
            timeout,
            isolation_level,
            explicit_transaction,
            trace_callback,
            callback_context,
        })
    }

    /// Return an async context manager for a savepoint.
    /// Requires an active transaction. On __aenter__ runs SAVEPOINT &lt;name&gt;;
    /// on __aexit__ runs RELEASE SAVEPOINT (success) or ROLLBACK TO SAVEPOINT (exception).
    #[pyo3(signature = (name = None))]
    fn savepoint(slf: PyRef<Self>, name: Option<String>) -> SavepointContextManager {
        let path = slf.path.clone();
        let transaction_connection = Arc::clone(&slf.transaction_connection);
        let transaction_state = Arc::clone(&slf.transaction_state);
        let sp_name = name.unwrap_or_else(next_savepoint_name);
        SavepointContextManager {
            path,
            transaction_connection,
            transaction_state,
            name: sp_name,
        }
    }

    /// Set a PRAGMA value on the database connection.
    fn set_pragma(
        self_: PyRef<Self>,
        name: String,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let path = self_.path.clone();
        let pool = Arc::clone(&self_.pool);
        let pragmas = Arc::clone(&self_.pragmas);
        let pool_size = Arc::clone(&self_.pool_size);
        let connection_timeout_secs = Arc::clone(&self_.connection_timeout_secs);
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        let session_connection = self_.session_connection.clone();
        let callback_connection = Arc::clone(&self_.callback_connection);
        let callback_context = self_.callback_context();
        let include_query_in_errors = *self_.include_query_in_errors.lock().unwrap();
        // Init hook infrastructure (Phase 2.11)
        let init_hook = Arc::clone(&self_.init_hook);
        let init_hook_called = Arc::clone(&self_.init_hook_called);
        let init_hook_present = Arc::clone(&self_.init_hook_present);
        let closed = Arc::clone(&self_.closed);
        let connection_self = self_.into();
        let init_hook_reentrant =
            capture_init_hook_reentrancy(&init_hook_present, &init_hook_called, &connection_self)?;

        // Convert value to string for PRAGMA
        // Note: Python::attach is used here for sync PRAGMA value conversion before async execution.
        // The deprecation warning is acceptable as this is a sync context.
        #[allow(deprecated)]
        let pragma_value = Python::attach(|_py| -> PyResult<String> {
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

        // Safety: PRAGMA names and values come from user input, but PRAGMA statements
        // are limited in scope. SQLite PRAGMA names are identifiers (alphanumeric + underscore),
        // and values are typically simple (strings, integers, or keywords like "WAL", "NORMAL").
        // However, to be safe, we validate that the name doesn't contain SQL injection patterns.
        // Note: Full validation would require a whitelist of valid PRAGMA names, but that's
        // overly restrictive. The current approach relies on SQLite's PRAGMA parser which
        // will reject invalid PRAGMA names/values.
        let pragma_query = format!("PRAGMA {name} = {pragma_value}");

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Apply PRAGMA to the current connection (transaction or session) if we have one,
                // so subsequent operations on this Connection see the new value (e.g. fetch_all after set_pragma).
                {
                    let mut conn_guard = transaction_connection.lock().await;
                    if let Some(ref mut conn) = conn_guard.0 {
                        sqlx::query(&pragma_query)
                            .execute(&mut **conn)
                            .await
                            .map_err(|e| {
                                map_sqlx_error_with_visibility(
                                    e,
                                    &path,
                                    &pragma_query,
                                    include_query_in_errors,
                                )
                            })?;
                        return Ok(());
                    }
                }
                {
                    // Callback-bound connections are retained in a separate
                    // slot when session affinity is enabled. Serialize with
                    // callback operations before checking either slot so a
                    // concurrent callback setup/teardown cannot move the
                    // connection between the check and the PRAGMA update.
                    let _callback_operation_guard =
                        callback_context.callback_operation_lock.lock().await;
                    {
                        let mut callback_guard = callback_connection.lock().await;
                        if let Some(ref mut conn) = callback_guard.0 {
                            let pragmas_list = pragmas.lock().unwrap().clone();
                            apply_pragmas_to_connection(conn, &pragmas_list, &path).await?;
                            // The connection may already have this PRAGMA
                            // fingerprint while its live value was overridden
                            // directly, so enforce the requested value too.
                            sqlx::query(&pragma_query)
                                .execute(&mut **conn)
                                .await
                                .map_err(|e| {
                                    map_sqlx_error_with_visibility(
                                        e,
                                        &path,
                                        &pragma_query,
                                        include_query_in_errors,
                                    )
                                })?;
                            return Ok(());
                        }
                    }
                    {
                        let mut conn_guard = session_connection.lock().await;
                        if let Some(ref mut conn) = conn_guard.0 {
                            let pragmas_list = pragmas.lock().unwrap().clone();
                            apply_pragmas_to_connection(conn, &pragmas_list, &path).await?;
                            sqlx::query(&pragma_query)
                                .execute(&mut **conn)
                                .await
                                .map_err(|e| {
                                    map_sqlx_error_with_visibility(
                                        e,
                                        &path,
                                        &pragma_query,
                                        include_query_in_errors,
                                    )
                                })?;
                            return Ok(());
                        }
                    }
                }
                // No current connection: ensure pool exists, run init_hook if needed, acquire a connection and apply PRAGMA.
                let pool_clone = get_or_create_pool(
                    &path,
                    &pool,
                    &pragmas,
                    &pool_size,
                    &connection_timeout_secs,
                    &idle_timeout_secs,
                )
                .await?;

                execute_init_hook_if_needed(
                    &init_hook,
                    &init_hook_present,
                    &init_hook_called,
                    init_hook_reentrant,
                    connection_self,
                )
                .await?;

                let pool_size_val = {
                    let g = pool_size.lock().unwrap();
                    *g
                };
                let timeout_val = {
                    let g = connection_timeout_secs.lock().unwrap();
                    *g
                };
                let mut conn =
                    acquire_with_pragmas(&pool_clone, &pragmas, &path, pool_size_val, timeout_val)
                        .await?;
                // Apply the explicitly requested value even if the per-handle
                // PRAGMA fingerprint says the configured set is already applied.
                sqlx::query(&pragma_query)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        map_sqlx_error_with_visibility(
                            e,
                            &path,
                            &pragma_query,
                            include_query_in_errors,
                        )
                    })?;
                drop(conn);

                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Interrupt a long-running query (Phase 3.9, aiosqlite-compatible).
    /// Interrupts the callback connection when present (UDFs, trace, authorizer, etc.);
    /// no-op when no callbacks are configured.
    fn interrupt(&self) -> PyResult<Py<PyAny>> {
        let callback_connection = Arc::clone(&self.callback_connection);
        let transaction_connection = Arc::clone(&self.transaction_connection);
        let session_slot = self.session_connection.raw_slot();
        let callback_connection_required = Arc::clone(&self.callback_connection_required);
        let user_functions = Arc::clone(&self.user_functions);
        let user_aggregates = Arc::clone(&self.user_aggregates);
        let user_collations = Arc::clone(&self.user_collations);
        let trace_callback = Arc::clone(&self.trace_callback);
        let authorizer_callback = Arc::clone(&self.authorizer_callback);
        let progress_handler = Arc::clone(&self.progress_handler);
        let closed = Arc::clone(&self.closed);
        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                let active_raw_operation = callbacks::interrupt_active_handle(&session_slot)
                    || callbacks::interrupt_active_handle(&transaction_connection)
                    || callbacks::interrupt_active_handle(&callback_connection);
                if !has_callbacks(
                    &callback_connection_required,
                    &user_functions,
                    &user_aggregates,
                    &user_collations,
                    &trace_callback,
                    &authorizer_callback,
                    &progress_handler,
                ) && !active_raw_operation
                {
                    return Ok(());
                }
                if active_raw_operation {
                    return Ok(());
                }

                // The query task holds the callback/transaction slot mutex while it
                // executes. Read the registered raw handle before attempting that
                // mutex so interrupt() can reach an in-flight SQLite operation.
                if callbacks::interrupt_active_handle(&transaction_connection)
                    || callbacks::interrupt_active_handle(&callback_connection)
                {
                    return Ok(());
                }

                let Ok(mut conn_guard) = callback_connection.try_lock() else {
                    return Ok(());
                };
                let Some(conn) = conn_guard.0.as_mut() else {
                    return Ok(());
                };
                let sqlite_conn: &mut SqliteConnection = conn;
                let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                    OperationalError::new_err(format!("Failed to lock handle: {e}"))
                })?;
                let raw_db = handle.as_raw_handle().as_ptr();
                unsafe { sqlite3_interrupt(raw_db) };
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Enable or disable loading SQLite extensions.
    fn enable_load_extension(&self, enabled: bool) -> PyResult<Py<PyAny>> {
        let callback_context = self.callback_context();
        let closed = Arc::clone(&self.closed);

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                let _callback_operation_guard =
                    callback_context.callback_operation_lock.lock().await;
                *callback_context.extension_loading_allowed.lock().unwrap() = enabled;
                let extensions_loaded = !callback_context
                    .loaded_extensions
                    .lock()
                    .unwrap()
                    .is_empty();
                *callback_context
                    .callback_connection_required
                    .lock()
                    .unwrap() = enabled || extensions_loaded;
                if enabled || extensions_loaded {
                    crate::pool::refresh_callback_features(&callback_context);
                }
                callbacks::rebind_callbacks(callback_context.clone()).await?;
                callbacks::discard_callback_connection(&callback_context).await;
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Load a SQLite extension from the specified file.
    /// Extension loading must be enabled first using enable_load_extension(true).
    fn load_extension(&self, name: String) -> PyResult<Py<PyAny>> {
        let callback_context = self.callback_context();
        let closed = Arc::clone(&self.closed);

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Check if extension loading is enabled
                let enabled = *callback_context.extension_loading_allowed.lock().unwrap();

                if !enabled {
                    return Err(OperationalError::new_err(
                        "Extension loading is not enabled. Call enable_load_extension(true) first.",
                    ));
                }

                let _callback_operation_guard =
                    callback_context.callback_operation_lock.lock().await;
                callbacks::rebind_callbacks(callback_context.clone()).await?;
                let load_result: Result<(), PyErr> = async {
                    let mut conn_guard = callback_context.callback_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let sqlite_conn: &mut SqliteConnection = conn;
                    let mut handle = sqlite_conn.lock_handle().await.map_err(|e| {
                        OperationalError::new_err(format!("Failed to lock handle: {e}"))
                    })?;
                    let raw_db = handle.as_raw_handle().as_ptr();
                    let name_cstr = CString::new(name.clone()).map_err(|e| {
                        OperationalError::new_err(format!("Invalid extension name: {e}"))
                    })?;
                    let mut errmsg: *mut std::ffi::c_char = std::ptr::null_mut();
                    let result = unsafe {
                        sqlite3_load_extension(
                            raw_db,
                            name_cstr.as_ptr(),
                            std::ptr::null(),
                            &mut errmsg,
                        )
                    };
                    if result != SQLITE_OK {
                        let error_msg = if errmsg.is_null() {
                            format!("SQLite error code {result}")
                        } else {
                            let message = unsafe {
                                cstr_from_c_char_ptr(errmsg).to_string_lossy().into_owned()
                            };
                            unsafe { sqlite3_free(errmsg.cast()) };
                            message
                        };
                        return Err(OperationalError::new_err(format!(
                            "Failed to load extension '{name}': {error_msg}"
                        )));
                    }
                    if !errmsg.is_null() {
                        unsafe { sqlite3_free(errmsg.cast()) };
                    }
                    Ok(())
                }
                .await;
                callbacks::discard_callback_connection(&callback_context).await;
                load_result?;
                let mut extensions = callback_context.loaded_extensions.lock().unwrap();
                if !extensions.contains(&name) {
                    extensions.push(name);
                }
                *callback_context
                    .callback_connection_required
                    .lock()
                    .unwrap() = true;
                crate::pool::refresh_callback_features(&callback_context);
                Ok(())
            };
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }

    /// Create or remove a user-defined SQL function.
    /// If func is None, the function is removed.
    /// deterministic: if true, mark function as deterministic (SQLite 3.8.3+); enables optimizations.
    #[pyo3(signature = (name, nargs, func, deterministic = false))]
    fn create_function(
        &self,
        name: String,
        nargs: i32,
        func: Option<Py<PyAny>>,
        deterministic: bool,
    ) -> PyResult<Py<PyAny>> {
        if !(-1..=127).contains(&nargs) {
            return Err(ProgrammingError::new_err(format!(
                "Invalid nargs for create_function: {nargs}. Expected -1..=127."
            )));
        }
        if deterministic {
            let v = unsafe { sqlite3_libversion_number() };
            if v < 3008003 {
                return Err(NotSupportedError::new_err(format!(
                    "create_function(deterministic=True) requires SQLite 3.8.3 or newer; got {}.{}.{}",
                    v / 1_000_000,
                    (v / 1000) % 1000,
                    v % 1000
                )));
            }
        }

        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };

        Python::attach(|py| {
            let func_clone = func.as_ref().map(|f| f.clone_ref(py));
            future_into_py(
                py,
                callbacks::create_function_impl(ctx, name, nargs, func_clone, deterministic),
            )
            .map(|bound| bound.unbind())
        })
    }

    /// Create or remove a custom SQL aggregate function.
    /// The aggregate class must implement `step(self, *args)` and `finalize(self)`.
    /// If aggregate_class is None, the aggregate is removed.
    #[pyo3(signature = (name, num_params, aggregate_class))]
    fn create_aggregate(
        &self,
        name: String,
        num_params: i32,
        aggregate_class: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        if !(-1..=127).contains(&num_params) {
            return Err(ProgrammingError::new_err(format!(
                "Invalid num_params for create_aggregate: {num_params}. Expected -1..=127."
            )));
        }

        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };

        Python::attach(|py| {
            future_into_py(
                py,
                callbacks::create_aggregate_impl(ctx, name, num_params, aggregate_class),
            )
            .map(|bound| bound.unbind())
        })
    }

    /// Create or remove a custom collation. The callable receives (s1: str, s2: str) and returns -1, 0, or 1.
    /// If callable is None, the collation is removed.
    #[pyo3(signature = (name, callable))]
    fn create_collation(&self, name: String, callable: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };
        Python::attach(|py| {
            future_into_py(py, callbacks::create_collation_impl(ctx, name, callable))
                .map(|bound| bound.unbind())
        })
    }

    /// Register an adapter for a Python type. When binding parameters, if the value's type
    /// matches, adapter(value) is called and the result is used. Pass adapter=None to remove
    /// adapters for that type.
    #[pyo3(signature = (type_, adapter))]
    fn register_adapter(&self, type_: Py<PyAny>, adapter: Option<Py<PyAny>>) -> PyResult<()> {
        let adapters = Arc::clone(&self.adapters);
        if let Some(adapter) = adapter {
            #[allow(deprecated)]
            Python::attach(|py| {
                let mut guard = adapters.lock().unwrap();
                guard.push((type_.clone_ref(py), adapter.clone_ref(py)));
                adapters.set_enabled(true);
                Ok(())
            })
        } else {
            #[allow(deprecated)]
            Python::attach(|py| {
                let type_bound = type_.bind(py);
                let mut guard = adapters.lock().unwrap();
                guard.retain(|(t, _)| !t.bind(py).get_type().is(type_bound.get_type()));
                adapters.set_enabled(!guard.is_empty());
                Ok(())
            })
        }
    }

    /// Register a converter for a declared column type. When reading rows, if the column's
    /// declared type matches typename, converter(bytes) is called and the result is used.
    /// Pass converter=None to remove the converter for that type.
    #[pyo3(signature = (typename, converter))]
    fn register_converter(&self, typename: &str, converter: Option<Py<PyAny>>) -> PyResult<()> {
        let key = typename.to_uppercase();
        let converters = Arc::clone(&self.converters);
        let mut guard = converters.lock().unwrap();
        if let Some(c) = converter {
            guard.insert(key, c);
        } else {
            guard.remove(&key);
        }
        converters.set_enabled(!guard.is_empty());
        Ok(())
    }

    /// Set or clear the trace callback.
    /// The callback receives SQL strings as they are executed.
    fn set_trace_callback(&self, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };

        Python::attach(|py| {
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            {
                ctx.trace_callback.replace(callback_clone);
            }
            future_into_py(py, callbacks::set_trace_callback_impl(ctx, callback))
                .map(|bound| bound.unbind())
        })
    }

    /// Set or clear the authorizer callback.
    /// The callback receives (action, arg1, arg2, arg3, arg4) and returns an int (SQLITE_OK, SQLITE_DENY, etc.).
    fn set_authorizer(&self, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };
        Python::attach(|py| {
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            {
                let mut auth_guard = ctx.authorizer_callback.lock().unwrap();
                *auth_guard = callback_clone;
            }
            // Publish callback routing before yielding the future. Otherwise
            // concurrent SQL can observe the new authorizer registry while the
            // lock-free routing summary still sends it through an unprotected
            // session connection.
            if callback.is_some() {
                crate::pool::refresh_callback_features(&ctx);
            }
            future_into_py(py, callbacks::set_authorizer_impl(ctx, callback))
                .map(|bound| bound.unbind())
        })
    }

    /// Set or clear the progress handler callback.
    /// The callback is called every N VDBE operations and returns True to continue, False to abort.
    fn set_progress_handler(&self, n: i32, callback: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        let ctx = callbacks::CallbackContext {
            closed: Arc::clone(&self.closed),
            path: self.path.clone(),
            pool: Arc::clone(&self.pool),
            pragmas: Arc::clone(&self.pragmas),
            pool_size: Arc::clone(&self.pool_size),
            connection_timeout_secs: Arc::clone(&self.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self.idle_timeout_secs),
            transaction_connection: Arc::clone(&self.transaction_connection),
            session_connection: self.session_connection.clone(),
            callback_connection: Arc::clone(&self.callback_connection),
            callback_operation_lock: Arc::clone(&self.callback_operation_lock),
            callback_connection_required: Arc::clone(&self.callback_connection_required),
            callback_features: Arc::clone(&self.callback_features),
            extension_loading_allowed: Arc::clone(&self.extension_loading_allowed),
            loaded_extensions: Arc::clone(&self.loaded_extensions),
            user_functions: Arc::clone(&self.user_functions),
            user_aggregates: Arc::clone(&self.user_aggregates),
            user_collations: Arc::clone(&self.user_collations),
            trace_callback: Arc::clone(&self.trace_callback),
            authorizer_callback: Arc::clone(&self.authorizer_callback),
            progress_handler: Arc::clone(&self.progress_handler),
            authorizer_callback_ctx_ptr: Arc::clone(&self.authorizer_callback_ctx_ptr),
            progress_handler_ctx_ptr: Arc::clone(&self.progress_handler_ctx_ptr),
            skip_release: false,
        };
        Python::attach(|py| {
            let callback_clone = callback.as_ref().map(|c| c.clone_ref(py));
            {
                let mut progress_guard = ctx.progress_handler.lock().unwrap();
                *progress_guard = callback_clone.map(|c| (n, c));
            }
            if callback.is_some() {
                crate::pool::refresh_callback_features(&ctx);
            }
            future_into_py(py, callbacks::set_progress_handler_impl(ctx, n, callback))
                .map(|bound| bound.unbind())
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
        let idle_timeout_secs = Arc::clone(&self_.idle_timeout_secs);
        let transaction_state = Arc::clone(&self_.transaction_state);
        let transaction_connection = Arc::clone(&self_.transaction_connection);
        // Callback infrastructure (Phase 2.7)
        let callback_connection = Arc::clone(&self_.callback_connection);
        let closed = Arc::clone(&self_.closed);
        let callback_context = self_.callback_context();

        Python::attach(|py| {
            let future = async move {
                ensure_not_closed(&closed)?;
                // Priority: transaction > callbacks > pool
                let in_transaction = transaction_state.is_routing_active().await;

                let has_callbacks_flag = callbacks_enabled(&callback_context.callback_features);
                let _callback_operation_guard = if has_callbacks_flag {
                    Some(callback_context.callback_operation_lock.lock().await)
                } else {
                    None
                };
                if has_callbacks_flag && !in_transaction {
                    callbacks::rebind_callbacks(callback_context.clone()).await?;
                }

                // Helper function to encode bytes as hex
                fn bytes_to_hex(bytes: &[u8]) -> String {
                    bytes.iter().map(|b| format!("{b:02x}")).collect()
                }

                // Get connection for queries
                // We need to handle different connection types
                let mut statements = Vec::new();
                statements.push("BEGIN TRANSACTION;".to_string());

                // Query sqlite_master - use appropriate connection
                let schema_rows = if in_transaction {
                    let mut conn_guard = transaction_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Transaction connection not available")
                    })?;
                    sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 WHEN 'view' THEN 3 ELSE 4 END, name")
                        .fetch_all(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"))?
                } else if has_callbacks_flag {
                    let mut conn_guard = callback_connection.lock().await;
                    let conn = conn_guard.0.as_mut().ok_or_else(|| {
                        OperationalError::new_err("Callback connection not available")
                    })?;
                    let schema_result = sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 WHEN 'view' THEN 3 ELSE 4 END, name")
                        .fetch_all(&mut **conn)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"));
                    drop(conn_guard);
                    callbacks::discard_callback_connection(&callback_context).await;
                    schema_result?
                } else {
                    let pool_clone = get_or_create_pool(
                        &path,
                        &pool,
                        &pragmas,
                        &pool_size,
                        &connection_timeout_secs,
                        &idle_timeout_secs,
                    )
                    .await?;
                    sqlx::query("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 WHEN 'view' THEN 3 ELSE 4 END, name")
                        .fetch_all(&pool_clone)
                        .await
                        .map_err(|e| map_sqlx_error(e, &path, "SELECT FROM sqlite_master"))?
                };

                // Collect table names for data dumping
                let mut table_names = Vec::new();
                let mut index_statements = Vec::new();
                let mut view_statements = Vec::new();
                let mut trigger_statements = Vec::new();

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
                            // Skip system indexes
                            "index" if !name.starts_with("sqlite_") => {
                                index_statements.push(format!("{sql_stmt};"));
                            }
                            "view" => {
                                view_statements.push(format!("{sql_stmt};"));
                            }
                            "trigger" => {
                                trigger_statements.push(format!("{sql_stmt};"));
                            }
                            _ => {}
                        }
                    }
                }

                // Helper function to escape SQL string
                let escape_sql_string = |s: &str| -> String { s.replace("'", "''") };

                // Helper to safely quote SQLite identifiers (table/column names).
                // This prevents malformed SQL and avoids identifier-based SQL injection in iterdump output.
                fn quote_ident_part(ident: &str) -> String {
                    format!("\"{}\"", ident.replace('"', "\"\""))
                }

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
                // Table names from sqlite_master are single identifiers. Dots inside a
                // legal table name must not be interpreted as schema separators.
                for table_name in table_names {
                    let quoted_table = quote_ident_part(&table_name);
                    let query = format!("SELECT * FROM {quoted_table}");
                    let rows = if in_transaction {
                        let mut conn_guard = transaction_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Transaction connection not available")
                        })?;
                        sqlx::query(&query)
                            .fetch_all(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query))?
                    } else if has_callbacks_flag {
                        callbacks::rebind_callbacks(callback_context.clone()).await?;
                        let mut conn_guard = callback_connection.lock().await;
                        let conn = conn_guard.0.as_mut().ok_or_else(|| {
                            OperationalError::new_err("Callback connection not available")
                        })?;
                        let rows_result = sqlx::query(&query)
                            .fetch_all(&mut **conn)
                            .await
                            .map_err(|e| map_sqlx_error(e, &path, &query));
                        drop(conn_guard);
                        callbacks::discard_callback_connection(&callback_context).await;
                        rows_result?
                    } else {
                        let pool_clone = get_or_create_pool(
                            &path,
                            &pool,
                            &pragmas,
                            &pool_size,
                            &connection_timeout_secs,
                            &idle_timeout_secs,
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
                            rows[0]
                                .columns()
                                .get(i)
                                .map(|c| c.name().to_string())
                                .unwrap_or_else(|| format!("column_{i}"))
                        })
                        .collect();

                    // Generate INSERT statements
                    let insert_table = quote_ident_part(&table_name);
                    let insert_cols: Vec<String> =
                        column_names.iter().map(|c| quote_ident_part(c)).collect();
                    for row in rows {
                        let mut values = Vec::new();
                        for i in 0..column_count {
                            values.push(format_value(&row, i));
                        }
                        let values_str = values.join(", ");
                        statements.push(format!(
                            "INSERT INTO {} ({}) VALUES ({});",
                            insert_table,
                            insert_cols.join(", "),
                            values_str
                        ));
                    }
                }

                // Build secondary schema objects after loading rows. Creating triggers
                // earlier would fire them while replaying the INSERT statements and
                // mutate data a second time. Views precede triggers so INSTEAD OF
                // triggers can refer to their target views.
                statements.extend(index_statements);
                statements.extend(view_statements);
                statements.extend(trigger_statements);

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
    fn get_tables(self_: PyRef<Self>, name: Option<String>) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_tables(ctx, name)).map(|bound| bound.unbind())
        })
    }

    /// Get table information (columns) for a specific table.
    fn get_table_info(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_table_info(ctx, table_name)).map(|bound| bound.unbind())
        })
    }

    /// Get list of indexes in the database.
    #[pyo3(signature = (table_name = None))]
    fn get_indexes(self_: PyRef<Self>, table_name: Option<String>) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_indexes(ctx, table_name)).map(|bound| bound.unbind())
        })
    }

    /// Get foreign key constraints for a specific table.
    fn get_foreign_keys(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_foreign_keys(ctx, table_name))
                .map(|bound| bound.unbind())
        })
    }

    /// Get comprehensive schema information for a table or all tables.
    #[pyo3(signature = (table_name = None))]
    fn get_schema(self_: PyRef<Self>, table_name: Option<String>) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_schema(ctx, table_name)).map(|bound| bound.unbind())
        })
    }

    /// Get list of views in the database.
    #[pyo3(signature = (name = None))]
    fn get_views(self_: PyRef<Self>, name: Option<String>) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_views(ctx, name)).map(|bound| bound.unbind())
        })
    }

    /// Get list of indexes for a specific table using PRAGMA index_list.
    fn get_index_list(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_index_list(ctx, table_name)).map(|bound| bound.unbind())
        })
    }

    /// Get information about columns in an index using PRAGMA index_info.
    fn get_index_info(self_: PyRef<Self>, index_name: String) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_index_info(ctx, index_name)).map(|bound| bound.unbind())
        })
    }

    /// Get extended table information using PRAGMA table_xinfo (SQLite 3.26.0+).
    /// Returns additional information beyond table_info, including hidden columns.
    fn get_table_xinfo(self_: PyRef<Self>, table_name: String) -> PyResult<Py<PyAny>> {
        let ctx = Self::build_schema_context(self_)?;
        Python::attach(|py| {
            future_into_py(py, schema::get_table_xinfo(ctx, table_name)).map(|bound| bound.unbind())
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
        let source = backup::BackupSourceContext {
            closed: Arc::clone(&self_.closed),
            path: self_.path.clone(),
            pool: Arc::clone(&self_.pool),
            pragmas: Arc::clone(&self_.pragmas),
            pool_size: Arc::clone(&self_.pool_size),
            connection_timeout_secs: Arc::clone(&self_.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&self_.idle_timeout_secs),
            transaction_state: Arc::clone(&self_.transaction_state),
            transaction_connection: Arc::clone(&self_.transaction_connection),
            callback_connection: Arc::clone(&self_.callback_connection),
            callback_operation_lock: Arc::clone(&self_.callback_operation_lock),
            callback_connection_required: Arc::clone(&self_.callback_connection_required),
            user_functions: Arc::clone(&self_.user_functions),
            user_aggregates: Arc::clone(&self_.user_aggregates),
            user_collations: Arc::clone(&self_.user_collations),
            trace_callback: Arc::clone(&self_.trace_callback),
            authorizer_callback: Arc::clone(&self_.authorizer_callback),
            progress_handler: Arc::clone(&self_.progress_handler),
        };
        let name = name.to_string();
        Python::attach(|py| {
            let progress_callback = progress.as_ref().map(|p| p.clone_ref(py));
            let target_enum = if target.bind(py).is_instance_of::<Connection>() {
                let target_py = target.clone_ref(py);
                let target_conn = target_py
                    .bind(py)
                    .cast::<Connection>()
                    .map_err(|_| OperationalError::new_err("Failed to cast target connection"))?;
                let t = target_conn.borrow();
                backup::BackupTarget::Rapsqlite(backup::BackupTargetRapsqliteContext {
                    path: t.path.clone(),
                    pool: Arc::clone(&t.pool),
                    pragmas: Arc::clone(&t.pragmas),
                    pool_size: Arc::clone(&t.pool_size),
                    connection_timeout_secs: Arc::clone(&t.connection_timeout_secs),
                    idle_timeout_secs: Arc::clone(&t.idle_timeout_secs),
                    transaction_state: Arc::clone(&t.transaction_state),
                    transaction_connection: Arc::clone(&t.transaction_connection),
                    callback_connection: Arc::clone(&t.callback_connection),
                    callback_operation_lock: Arc::clone(&t.callback_operation_lock),
                    callback_connection_required: Arc::clone(&t.callback_connection_required),
                    user_functions: Arc::clone(&t.user_functions),
                    user_aggregates: Arc::clone(&t.user_aggregates),
                    user_collations: Arc::clone(&t.user_collations),
                    trace_callback: Arc::clone(&t.trace_callback),
                    authorizer_callback: Arc::clone(&t.authorizer_callback),
                    progress_handler: Arc::clone(&t.progress_handler),
                })
            } else {
                backup::BackupTarget::Sqlite3(target.clone_ref(py))
            };
            let future =
                backup::run_backup(source, target_enum, name, pages, progress_callback, sleep);
            future_into_py(py, future).map(|bound| bound.unbind())
        })
    }
}
