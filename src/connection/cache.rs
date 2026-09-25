//! Native cache operations with Rust-side TTL handling and SQLite bindings.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyInt};
use pyo3_async_runtimes::tokio::future_into_py;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqliteConnection;
use sqlx::Sqlite;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::batch::{fetch_scalar_raw_core, RawScalar};
use crate::errors::{map_sqlite_error_from_msg, map_sqlx_error_with_visibility};
use crate::pool::{
    callbacks_enabled, execute_init_hook_if_needed_fast, get_or_create_pool,
    lock_session_connection, PoolConnectionSlot, PoolSlot, SessionConnectionSlot,
};
use crate::types::SqliteParam;
use crate::{OperationalError, ProgrammingError, ValueError};

use super::{
    clear_active_handle, discard_callback_connection, ensure_not_closed, invoke_trace_callback,
    rebind_callbacks, CallbackContext, Connection,
};
use crate::types::TransactionState;

pub(super) struct CacheExecutionContext {
    path: String,
    pool: Arc<Mutex<PoolSlot>>,
    pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    transaction_state: Arc<Mutex<TransactionState>>,
    transaction_connection: Arc<Mutex<crate::pool::PoolConnectionSlot>>,
    callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    callback_operation_lock: Arc<Mutex<()>>,
    callback_context: CallbackContext,
    session_connection: SessionConnectionSlot,
    callback_features: Arc<AtomicU8>,
    trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_called: Arc<StdMutex<bool>>,
    init_hook_present: Arc<AtomicBool>,
    closed: Arc<StdMutex<bool>>,
    include_query_in_errors: bool,
    query_cache: Arc<StdMutex<std::collections::HashMap<String, u64>>>,
    query_usage_enabled: Arc<AtomicBool>,
}

impl CacheExecutionContext {
    pub(super) fn from_connection(connection: &Connection) -> Self {
        Self {
            path: connection.path.clone(),
            pool: Arc::clone(&connection.pool),
            pragmas: Arc::clone(&connection.pragmas),
            pool_size: Arc::clone(&connection.pool_size),
            connection_timeout_secs: Arc::clone(&connection.connection_timeout_secs),
            idle_timeout_secs: Arc::clone(&connection.idle_timeout_secs),
            transaction_state: Arc::clone(&connection.transaction_state),
            transaction_connection: Arc::clone(&connection.transaction_connection),
            callback_connection: Arc::clone(&connection.callback_connection),
            callback_operation_lock: Arc::clone(&connection.callback_operation_lock),
            callback_context: connection.callback_context(),
            session_connection: connection.session_connection.clone(),
            callback_features: Arc::clone(&connection.callback_features),
            trace_callback: Arc::clone(&connection.trace_callback),
            init_hook: Arc::clone(&connection.init_hook),
            init_hook_called: Arc::clone(&connection.init_hook_called),
            init_hook_present: Arc::clone(&connection.init_hook_present),
            closed: Arc::clone(&connection.closed),
            include_query_in_errors: *connection.include_query_in_errors.lock().unwrap(),
            query_cache: Arc::clone(&connection.query_cache),
            query_usage_enabled: Arc::clone(&connection.query_usage_enabled),
        }
    }
}

pub(super) enum CacheOperation {
    Initialize {
        create_table: String,
        create_index: String,
    },
    Get {
        query: String,
        key: String,
    },
    Set {
        query: String,
        key: String,
        value: Vec<u8>,
        ttl_seconds: Option<f64>,
    },
    Delete {
        query: String,
        key: String,
    },
    CleanupExpired {
        query: String,
        limit: i64,
    },
}

enum CacheResult {
    None,
    InitializedInTransaction(bool),
    Value(Option<Vec<u8>>),
    Deleted(bool),
    DeletedCount(u64),
}

pub(super) fn initialize_operation(table_name: String) -> PyResult<CacheOperation> {
    let table = quoted_table_name(&table_name)?;
    // Cache table names cannot contain `:`, keeping this internal index name
    // out of the namespace available to other SQLiteCache tables.
    let index = format!("\"rapsqlite_cache_expiration:{table_name}\"");
    Ok(CacheOperation::Initialize {
        create_table: format!(
            concat!(
                "CREATE TABLE IF NOT EXISTS {} (",
                "key TEXT PRIMARY KEY NOT NULL, ",
                "value BLOB NOT NULL, ",
                "expires_at REAL",
                ") WITHOUT ROWID"
            ),
            table
        ),
        create_index: format!(
            concat!(
                "CREATE INDEX IF NOT EXISTS {} ON {} (expires_at) ",
                "WHERE expires_at IS NOT NULL"
            ),
            index, table
        ),
    })
}

pub(super) fn get_operation(table_name: String, key: String) -> PyResult<CacheOperation> {
    let table = quoted_table_name(&table_name)?;
    Ok(CacheOperation::Get {
        query: format!(
            concat!(
                "SELECT value FROM {} WHERE key = ? ",
                "AND (expires_at IS NULL OR expires_at > ?)"
            ),
            table
        ),
        key,
    })
}

pub(super) fn set_operation(
    table_name: String,
    key: String,
    value: Vec<u8>,
    ttl_seconds: Option<f64>,
) -> PyResult<CacheOperation> {
    let table = quoted_table_name(&table_name)?;
    Ok(CacheOperation::Set {
        query: format!(
            concat!(
                "INSERT INTO {} (key, value, expires_at) VALUES (?, ?, ?) ",
                "ON CONFLICT(key) DO UPDATE SET ",
                "value = excluded.value, expires_at = excluded.expires_at"
            ),
            table
        ),
        key,
        value,
        ttl_seconds,
    })
}

pub(super) fn delete_operation(table_name: String, key: String) -> PyResult<CacheOperation> {
    let table = quoted_table_name(&table_name)?;
    Ok(CacheOperation::Delete {
        query: format!("DELETE FROM {table} WHERE key = ?"),
        key,
    })
}

pub(super) fn cleanup_expired_operation(
    table_name: String,
    limit: i64,
) -> PyResult<CacheOperation> {
    if limit < 1 {
        return Err(ValueError::new_err("limit must be at least 1"));
    }
    let table = quoted_table_name(&table_name)?;
    Ok(CacheOperation::CleanupExpired {
        query: format!(
            concat!(
                "DELETE FROM {} WHERE key IN (",
                "SELECT key FROM {} ",
                "WHERE expires_at IS NOT NULL AND expires_at <= ? ",
                "ORDER BY expires_at LIMIT ?",
                ")"
            ),
            table, table
        ),
        limit,
    })
}

fn quoted_table_name(table_name: &str) -> PyResult<String> {
    if table_name.is_empty()
        || table_name.len() > 100
        || !table_name
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                0 => byte.is_ascii_alphabetic() || byte == b'_',
                _ => byte.is_ascii_alphanumeric() || byte == b'_',
            })
    {
        return Err(ValueError::new_err(
            "table_name must be a SQLite identifier of 1 to 100 letters, digits, and underscores, starting with a letter or underscore",
        ));
    }
    quoted_identifier(table_name)
}

fn quoted_identifier(identifier: &str) -> PyResult<String> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                0 => byte.is_ascii_alphabetic() || byte == b'_',
                _ => byte.is_ascii_alphanumeric() || byte == b'_',
            })
    {
        return Err(ValueError::new_err("invalid SQLite identifier"));
    }
    Ok(format!("\"{identifier}\""))
}

pub(super) fn start_cache_operation(
    connection: PyRef<'_, Connection>,
    operation: CacheOperation,
) -> PyResult<Py<PyAny>> {
    let context = CacheExecutionContext::from_connection(&connection);
    ensure_not_closed(&context.closed)?;
    let connection_self = connection.into();

    for query in operation.queries() {
        crate::utils::track_query_usage_if_enabled(
            &context.query_usage_enabled,
            &context.query_cache,
            query,
        );
    }

    Python::attach(|py| {
        let future = async move {
            let result = execute_cache_operation(context, connection_self, operation).await?;
            Python::attach(|py| result.into_py(py))
        };
        future_into_py(py, future).map(|bound| bound.unbind())
    })
}

impl CacheOperation {
    fn queries(&self) -> Vec<&str> {
        match self {
            Self::Initialize {
                create_table,
                create_index,
            } => vec![create_table, create_index],
            Self::Get { query, .. }
            | Self::Set { query, .. }
            | Self::Delete { query, .. }
            | Self::CleanupExpired { query, .. } => vec![query],
        }
    }
}

impl CacheResult {
    fn into_py(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match self {
            Self::None => py.None(),
            Self::InitializedInTransaction(value) => {
                PyBool::new(py, value).to_owned().into_any().unbind()
            }
            Self::Value(Some(value)) => PyBytes::new(py, &value).into_any().unbind(),
            Self::Value(None) => py.None(),
            Self::Deleted(value) => PyBool::new(py, value).to_owned().into_any().unbind(),
            Self::DeletedCount(value) => PyInt::new(py, value as i64).into_any().unbind(),
        })
    }
}

async fn execute_cache_operation(
    context: CacheExecutionContext,
    connection_self: Py<Connection>,
    operation: CacheOperation,
) -> PyResult<CacheResult> {
    ensure_not_closed(&context.closed)?;

    for query in operation.queries() {
        invoke_trace_callback(&context.trace_callback, query);
    }

    let in_transaction = context.transaction_state.lock().await.is_active();
    if !in_transaction {
        get_or_create_pool(
            &context.path,
            &context.pool,
            &context.pragmas,
            &context.pool_size,
            &context.connection_timeout_secs,
            &context.idle_timeout_secs,
        )
        .await?;
    }
    execute_init_hook_if_needed_fast(
        &context.init_hook,
        &context.init_hook_called,
        &context.init_hook_present,
        connection_self,
    )
    .await?;
    ensure_not_closed(&context.closed)?;

    let has_callbacks = callbacks_enabled(&context.callback_features);
    let _callback_guard = if has_callbacks {
        Some(context.callback_operation_lock.lock().await)
    } else {
        None
    };
    if has_callbacks && !in_transaction {
        rebind_callbacks(context.callback_context.clone()).await?;
    }
    let raw_handle_slot = if has_callbacks {
        None
    } else if in_transaction {
        Some(Arc::clone(&context.transaction_connection))
    } else {
        Some(context.session_connection.raw_slot())
    };

    if in_transaction {
        let mut guard = context.transaction_connection.lock().await;
        let connection = guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
        execute_on_connection(
            connection,
            &context.path,
            context.include_query_in_errors,
            in_transaction,
            raw_handle_slot.as_ref(),
            &operation,
        )
        .await
    } else if has_callbacks {
        let mut guard = context.callback_connection.lock().await;
        let connection = guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
        let result = execute_on_connection(
            connection,
            &context.path,
            context.include_query_in_errors,
            in_transaction,
            raw_handle_slot.as_ref(),
            &operation,
        )
        .await;
        drop(guard);
        discard_callback_connection(&context.callback_context).await;
        result
    } else {
        let mut guard = lock_session_connection(
            &context.path,
            &context.pool,
            &context.session_connection,
            &context.pragmas,
            &context.pool_size,
            &context.connection_timeout_secs,
            &context.idle_timeout_secs,
        )
        .await?;
        let connection = guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Session connection not available"))?;
        execute_on_connection(
            connection,
            &context.path,
            context.include_query_in_errors,
            in_transaction,
            raw_handle_slot.as_ref(),
            &operation,
        )
        .await
    }
}

async fn execute_on_connection(
    connection: &mut PoolConnection<Sqlite>,
    path: &str,
    include_query_in_errors: bool,
    in_transaction: bool,
    raw_handle_slot: Option<&Arc<Mutex<PoolConnectionSlot>>>,
    operation: &CacheOperation,
) -> PyResult<CacheResult> {
    match operation {
        CacheOperation::Initialize {
            create_table,
            create_index,
        } => {
            sqlx::query(create_table)
                .execute(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(
                        error,
                        path,
                        create_table,
                        include_query_in_errors,
                    )
                })?;
            sqlx::query(create_index)
                .execute(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(
                        error,
                        path,
                        create_index,
                        include_query_in_errors,
                    )
                })?;
            Ok(CacheResult::InitializedInTransaction(in_transaction))
        }
        CacheOperation::Get { query, key } => {
            let now = unix_time_seconds()?;
            if let Some(raw_handle_slot) = raw_handle_slot {
                let value = fetch_blob_raw(
                    connection,
                    raw_handle_slot,
                    path,
                    query,
                    key,
                    now,
                    include_query_in_errors,
                )
                .await?;
                return Ok(CacheResult::Value(value));
            }
            let value = sqlx::query_scalar::<_, Vec<u8>>(query)
                .bind(key.as_str())
                .bind(now)
                .fetch_optional(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(error, path, query, include_query_in_errors)
                })?;
            Ok(CacheResult::Value(value))
        }
        CacheOperation::Set {
            query,
            key,
            value,
            ttl_seconds,
        } => {
            let expires_at = match ttl_seconds {
                Some(ttl) => {
                    if !ttl.is_finite() || *ttl < 0.0 {
                        return Err(ValueError::new_err("ttl must be finite and non-negative"));
                    }
                    let expiration = unix_time_seconds()? + ttl;
                    if !expiration.is_finite() {
                        return Err(ValueError::new_err("ttl is too large"));
                    }
                    Some(expiration)
                }
                None => None,
            };
            sqlx::query(query)
                .bind(key.as_str())
                .bind(value.as_slice())
                .bind(expires_at)
                .execute(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(error, path, query, include_query_in_errors)
                })?;
            Ok(CacheResult::None)
        }
        CacheOperation::Delete { query, key } => {
            let result = sqlx::query(query)
                .bind(key.as_str())
                .execute(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(error, path, query, include_query_in_errors)
                })?;
            Ok(CacheResult::Deleted(result.rows_affected() > 0))
        }
        CacheOperation::CleanupExpired { query, limit } => {
            let now = unix_time_seconds()?;
            let result = sqlx::query(query)
                .bind(now)
                .bind(*limit)
                .execute(&mut **connection)
                .await
                .map_err(|error| {
                    map_sqlx_error_with_visibility(error, path, query, include_query_in_errors)
                })?;
            Ok(CacheResult::DeletedCount(result.rows_affected()))
        }
    }
}

async fn fetch_blob_raw(
    connection: &mut PoolConnection<Sqlite>,
    active_handle_slot: &Arc<Mutex<PoolConnectionSlot>>,
    path: &str,
    query: &str,
    key: &str,
    now: f64,
    include_query_in_errors: bool,
) -> PyResult<Option<Vec<u8>>> {
    let sqlite_connection: &mut SqliteConnection = connection;
    let mut handle = sqlite_connection.lock_handle().await.map_err(|error| {
        OperationalError::new_err(format!("Failed to lock SQLite handle: {error}"))
    })?;
    let database = handle.as_raw_handle().as_ptr();
    super::callbacks::register_active_handle(active_handle_slot, database as usize);
    let parameters = [SqliteParam::Text(key.to_owned()), SqliteParam::Real(now)];
    let result =
        tokio::task::block_in_place(|| fetch_scalar_raw_core(database, query, &parameters, true));
    clear_active_handle(active_handle_slot);
    let value = result.map_err(|(code, message)| {
        map_sqlite_error_from_msg(path, query, code, &message, include_query_in_errors)
    })?;
    match value {
        None | Some(RawScalar::Null) => Ok(None),
        Some(RawScalar::Blob(value)) => Ok(Some(value)),
        Some(_) => Err(ProgrammingError::new_err(
            "cache reads require a BLOB or NULL result",
        )),
    }
}

fn unix_time_seconds() -> PyResult<f64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            ValueError::new_err(format!("system clock is before Unix epoch: {error}"))
        })?;
    Ok(duration.as_secs_f64())
}
