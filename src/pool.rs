//! Pool creation and connection-management helpers.
//!
//! Uses a path-based global pool registry so multiple Connection objects
//! connecting to the same database path share one SqlitePool, improving
//! concurrent operation performance (e.g. many `connect(path)` calls).

use libsqlite3_sys::{sqlite3_finalize, sqlite3_stmt};
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::into_future;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell};

use crate::types::{ProgressHandler, TraceCallback, UserAggregates, UserCollations, UserFunctions};
use crate::OperationalError;

/// Shared initialization barrier for a Connection's optional Python init hook.
/// `started` preserves the fast routing checks; `completion` lets unrelated
/// first-use operations wait while allowing reentrant hook calls to proceed.
pub(crate) struct InitHookState {
    started: AtomicBool,
    completion: OnceCell<Result<(), Arc<str>>>,
}

impl InitHookState {
    pub(crate) fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            completion: OnceCell::new(),
        }
    }

    pub(crate) fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    pub(crate) fn is_in_progress(&self) -> bool {
        self.is_started() && self.completion.get().is_none()
    }
}

/// Snapshot Python's task-local reentrancy marker at the synchronous API
/// boundary, before the operation is moved onto the async runtime.
pub(crate) fn capture_init_hook_reentrancy(
    init_hook_present: &AtomicBool,
    init_hook_state: &InitHookState,
    connection: &Py<crate::Connection>,
) -> PyResult<bool> {
    if !init_hook_present.load(Ordering::Acquire) || init_hook_state.completion.get().is_some() {
        return Ok(false);
    }

    #[allow(deprecated)]
    Python::attach(|py| -> PyResult<bool> {
        py.import("rapsqlite._init_hook")?
            .getattr("_is_active")?
            .call1((connection.bind(py),))?
            .extract()
    })
}

fn sqlite_url_for_path(path: &str) -> String {
    if path == ":memory:" {
        return "sqlite::memory:".to_string();
    }

    // Allow explicit URLs for advanced use.
    // We intentionally do NOT treat `C:\...` as a URL.
    if path.starts_with("sqlite:") || path.starts_with("file:") {
        return path.to_string();
    }

    // Ensure we can create the DB file if missing.
    let mode = "mode=rwc";

    // Windows absolute path like `C:\foo\bar.db` or `C:/foo/bar.db`
    let windows_abs = path.len() >= 3
        && path.as_bytes()[1] == b':'
        && (path.as_bytes()[2] == b'\\' || path.as_bytes()[2] == b'/');
    if windows_abs {
        let p = path.replace('\\', "/");
        return format!("sqlite:///{}?{}", p, mode);
    }

    // POSIX absolute path: sqlite:///tmp/test.db
    if path.starts_with('/') {
        return format!("sqlite:///{}?{}", path.trim_start_matches('/'), mode);
    }

    // Relative path.
    format!("sqlite:{}?{}", path, mode)
}

fn drop_on_background_tokio<T: Send + 'static>(value: T) {
    // Best-effort: dropping sqlx pools/connections can require a Tokio runtime.
    // If we get dropped outside of Tokio (e.g. Python GC at shutdown), spawn a
    // short-lived runtime on a background thread to run the destructor.
    std::thread::spawn(move || {
        // If runtime creation fails, fall back to dropping anyway; this should be rare.
        if let Ok(rt) = tokio::runtime::Runtime::new() {
            let _guard = rt.enter();
            drop(value);
        } else {
            drop(value);
        }
    });
}

/// Close a pooled connection even when called from synchronous Python code
/// outside a Tokio runtime. SQLx requires a Tokio context for
/// `close_on_drop`, so close it on a background runtime when necessary.
pub(crate) fn close_pool_connection_on_drop(mut connection: PoolConnection<sqlx::Sqlite>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        connection.close_on_drop();
        drop(connection);
        return;
    }

    std::thread::spawn(move || match tokio::runtime::Runtime::new() {
        Ok(runtime) => {
            let _ = runtime.block_on(connection.close());
        }
        Err(_) => drop(connection),
    });
}

/// Wrapper around `Option<PoolConnection>` that, when dropped outside a Tokio context,
/// forgets the connection instead of dropping it. This prevents sqlx's `PoolConnection::Drop`
/// from running without a runtime (e.g. during Python GC/shutdown).
#[derive(Default)]
pub(crate) struct PoolConnectionSlot(pub(crate) Option<PoolConnection<sqlx::Sqlite>>);

impl Drop for PoolConnectionSlot {
    fn drop(&mut self) {
        self.release();
    }
}

impl PoolConnectionSlot {
    fn release(&mut self) {
        if let Some(pc) = self.0.take() {
            if tokio::runtime::Handle::try_current().is_err() {
                drop_on_background_tokio(pc);
            } else {
                drop(pc);
            }
        }
    }
}

/// Wrapper around `Option<SqlitePool>` that, when dropped outside a Tokio context,
/// forgets the pool instead of dropping it. This prevents sqlx pool shutdown from
/// dropping connections (PoolConnection) without a runtime (e.g. during Python GC/shutdown).
#[derive(Default)]
pub(crate) struct PoolSlot(pub(crate) Option<SqlitePool>);

impl Drop for PoolSlot {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            if tokio::runtime::Handle::try_current().is_err() {
                drop_on_background_tokio(p);
            } else {
                drop(p);
            }
        }
    }
}

/// Immutable per-Connection reference to its initialized shared pool.
/// Pool handles are initialized once, then cloned without an async mutex.
#[derive(Default)]
pub(crate) struct PoolHandle(OnceLock<SqlitePool>);

impl PoolHandle {
    pub(crate) fn get(&self) -> Option<&SqlitePool> {
        self.0.get()
    }

    fn set(&self, pool: SqlitePool) {
        // A concurrent first operation may already have initialized this handle.
        // Dropping a redundant SQLx clone here occurs inside the Tokio runtime.
        let _ = self.0.set(pool);
    }
}

impl Drop for PoolHandle {
    fn drop(&mut self) {
        if let Some(pool) = self.0.take() {
            if tokio::runtime::Handle::try_current().is_err() {
                drop_on_background_tokio(pool);
            } else {
                drop(pool);
            }
        }
    }
}

/// RAII guard for a PoolConnection taken out of a slot (e.g. during backup).
/// When dropped, never runs sqlx's PoolConnection::Drop (which requires Tokio);
/// instead forgets the connection if still held. Callers must explicitly restore
/// on the success path via take_for_restore() and then put the connection back
/// into the slot.
#[derive(Default)]
pub(crate) struct TakenConnectionGuard(
    Option<(Arc<Mutex<PoolConnectionSlot>>, PoolConnection<sqlx::Sqlite>)>,
);

impl TakenConnectionGuard {
    pub(crate) fn new(
        slot: Arc<Mutex<PoolConnectionSlot>>,
        conn: PoolConnection<sqlx::Sqlite>,
    ) -> Self {
        Self(Some((slot, conn)))
    }

    /// Mutable reference to the held connection, if any.
    pub(crate) fn as_mut(&mut self) -> Option<&mut PoolConnection<sqlx::Sqlite>> {
        self.0.as_mut().map(|(_, c)| c)
    }

    /// Take the (slot, connection) for explicit restore. Leaves the guard empty so Drop is a no-op.
    pub(crate) fn take_for_restore(
        &mut self,
    ) -> Option<(Arc<Mutex<PoolConnectionSlot>>, PoolConnection<sqlx::Sqlite>)> {
        self.0.take()
    }

    /// Close and release the held connection instead of restoring it to its slot.
    pub(crate) fn discard(&mut self) {
        if let Some((_, mut conn)) = self.0.take() {
            conn.close_on_drop();
            drop(conn);
        }
    }
}

impl Drop for TakenConnectionGuard {
    fn drop(&mut self) {
        if let Some((_, conn)) = self.0.take() {
            if tokio::runtime::Handle::try_current().is_err() {
                drop_on_background_tokio(conn);
            } else {
                drop(conn);
            }
        }
    }
}

/// Minimum pool size when creating a shared pool so many concurrent
/// Connection objects to the same path can acquire connections.
const SHARED_POOL_MIN_CONNECTIONS: u32 = 25;

/// A small per-logical-connection cache for raw statements. Statements are
/// only reused while session affinity retains their physical SQLite handle.
pub(crate) struct RawStatementCache {
    statements: StdMutex<HashMap<String, usize>>,
    has_statements: AtomicBool,
}

impl Default for RawStatementCache {
    fn default() -> Self {
        Self {
            statements: StdMutex::new(HashMap::new()),
            has_statements: AtomicBool::new(false),
        }
    }
}

impl RawStatementCache {
    const MAX_STATEMENTS: usize = 64;
    const MAX_QUERY_BYTES: usize = 2_048;

    pub(crate) fn get(&self, query: &str) -> Option<usize> {
        self.statements
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(query)
            .copied()
    }

    pub(crate) fn insert(&self, query: String, statement: usize) -> bool {
        if query.len() > Self::MAX_QUERY_BYTES {
            return false;
        }
        let mut statements = self
            .statements
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if statements.len() >= Self::MAX_STATEMENTS || statements.contains_key(&query) {
            return false;
        }
        statements.insert(query, statement);
        self.has_statements.store(true, Ordering::Release);
        true
    }

    pub(crate) fn clear(&self) {
        if !self.has_statements.load(Ordering::Relaxed) {
            return;
        }
        let mut statements = self
            .statements
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pending: Vec<usize> = statements.drain().map(|(_, statement)| statement).collect();
        self.has_statements.store(false, Ordering::Release);
        drop(statements);
        for statement in pending {
            // Statements are finalized before their retained PoolConnection is
            // returned or closed. The slot lock excludes concurrent users.
            unsafe {
                sqlite3_finalize(statement as *mut sqlite3_stmt);
            }
        }
    }
}

impl Drop for RawStatementCache {
    fn drop(&mut self) {
        self.clear();
    }
}

/// A registry entry stays alive while at least one Connection uses this identity.
#[derive(Default)]
struct RegisteredPool {
    pool: PoolSlot,
    users: usize,
    configured_max_connections: Option<u32>,
}

/// Per-Connection slot for retaining one pooled connection across operations.
#[derive(Clone)]
pub(crate) struct SessionConnectionSlot {
    // Drop raw statements before the slot can drop its last pooled connection.
    // Explicit close/release clears these earlier while holding the slot lock;
    // this field order also makes implicit object destruction safe.
    raw_statement_cache: Arc<RawStatementCache>,
    slot: Arc<Mutex<PoolConnectionSlot>>,
    retain: Arc<AtomicBool>,
    retain_transition: Arc<StdMutex<()>>,
}

impl SessionConnectionSlot {
    fn new() -> Self {
        Self {
            raw_statement_cache: Arc::new(RawStatementCache::default()),
            slot: Arc::new(Mutex::new(PoolConnectionSlot::default())),
            retain: Arc::new(AtomicBool::new(false)),
            retain_transition: Arc::new(StdMutex::new(())),
        }
    }

    pub(crate) async fn lock(&self) -> tokio::sync::OwnedMutexGuard<PoolConnectionSlot> {
        Arc::clone(&self.slot).lock_owned().await
    }

    pub(crate) fn set_retain(&self, retain: bool) {
        let _transition = self
            .retain_transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.retain.store(retain, Ordering::Release);
        if !retain {
            // The property setter is synchronous. Release immediately when the
            // slot is idle; an operation already holding it observes the updated
            // atomic flag in SessionConnectionGuard::drop and releases it there.
            if let Ok(mut slot) = self.slot.try_lock() {
                self.raw_statement_cache.clear();
                slot.release();
            }
        }
    }

    pub(crate) fn retain(&self) -> bool {
        self.retain.load(Ordering::Acquire)
    }

    pub(crate) fn raw_slot(&self) -> Arc<Mutex<PoolConnectionSlot>> {
        Arc::clone(&self.slot)
    }

    pub(crate) fn raw_statement_cache(&self) -> Arc<RawStatementCache> {
        Arc::clone(&self.raw_statement_cache)
    }
}

/// Holds a logical Connection's session slot for one operation. Return the lease
/// to SQLx when the operation finishes so idle Connection wrappers do not consume
/// the pool's bounded capacity.
pub(crate) struct SessionConnectionGuard {
    guard: Option<tokio::sync::OwnedMutexGuard<PoolConnectionSlot>>,
    retain: Arc<AtomicBool>,
    raw_statement_cache: Arc<RawStatementCache>,
    retain_transition: Arc<StdMutex<()>>,
}

impl Deref for SessionConnectionGuard {
    type Target = PoolConnectionSlot;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_deref()
            .expect("session connection guard already released")
    }
}

impl DerefMut for SessionConnectionGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_deref_mut()
            .expect("session connection guard already released")
    }
}

impl SessionConnectionGuard {
    /// Transfer the physical connection to a transaction while invalidating any
    /// raw statements cached against its SQLite handle. The statements must be
    /// finalized before SQLx can return or close the connection independently.
    pub(crate) fn take_connection(&mut self) -> Option<PoolConnection<sqlx::Sqlite>> {
        self.raw_statement_cache.clear();
        self.guard
            .as_deref_mut()
            .expect("session connection guard already released")
            .0
            .take()
    }
}

impl Drop for SessionConnectionGuard {
    fn drop(&mut self) {
        // Serialize the retain check with set_retain(). Explicitly unlock the
        // slot before releasing this transition lock so a concurrent setter
        // can either release an idle connection itself or leave that work to
        // this guard, with no check/unlock race.
        let transition = self
            .retain_transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.retain.load(Ordering::Acquire) {
            self.raw_statement_cache.clear();
            if let Some(guard) = self.guard.as_deref_mut() {
                guard.release();
            }
        }
        drop(self.guard.take());
        drop(transition);
    }
}

/// Global registry: database identity -> shared pool and active Connection count.
fn global_registry() -> &'static StdMutex<HashMap<String, RegisteredPool>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, RegisteredPool>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Keeps a registry entry alive for one Connection and removes it when that
/// connection closes or is dropped. PoolSlot handles drops outside Tokio.
pub(crate) struct PoolRegistryLease {
    identity: Option<String>,
}

impl PoolRegistryLease {
    pub(crate) fn new(identity: String) -> (Self, SessionConnectionSlot) {
        let mut registry = global_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = registry.entry(identity.clone()).or_default();
        entry.users += 1;
        (
            Self {
                identity: Some(identity.clone()),
            },
            SessionConnectionSlot::new(),
        )
    }

    /// Configure the shared pool before first use. The first explicit pool size
    /// wins; once the SQLx pool exists, its actual maximum is authoritative.
    pub(crate) fn configure_pool_size(&self, requested: Option<usize>) {
        let Some(identity) = self.identity.as_ref() else {
            return;
        };
        let mut registry = global_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = registry.get_mut(identity) else {
            return;
        };
        if entry.pool.0.is_none() && entry.configured_max_connections.is_none() {
            if let Some(size) = requested {
                entry.configured_max_connections = Some(size.max(1).min(u32::MAX as usize) as u32);
            }
        }
    }

    pub(crate) fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    pub(crate) fn release(&mut self) {
        let Some(identity) = self.identity.take() else {
            return;
        };
        let mut registry = global_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remove = if let Some(entry) = registry.get_mut(&identity) {
            entry.users = entry.users.saturating_sub(1);
            entry.users == 0
        } else {
            false
        };
        if remove {
            registry.remove(&identity);
        }
    }
}

impl Drop for PoolRegistryLease {
    fn drop(&mut self) {
        self.release();
    }
}

/// Close and remove the pool held by a registry entry after its last Connection closes.
pub(crate) async fn close_registered_pool_if_last(identity: &str) {
    let pool = {
        let mut registry = global_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = registry.get_mut(identity) else {
            return;
        };
        if entry.users > 1 {
            return;
        }
        entry.pool.0.take()
    };
    if let Some(pool) = pool {
        pool.close().await;
    }
}

/// Create a helpful error message for pool acquisition failures.
pub(crate) fn pool_acquisition_error(
    path: &str,
    error: &sqlx::Error,
    pool_size: Option<usize>,
    timeout: Option<u64>,
) -> PyErr {
    let error_str = error.to_string();
    let is_timeout = error_str.contains("timeout") || error_str.contains("timed out");

    let mut msg = format!("Failed to acquire connection from pool at {path}: {error_str}");

    if is_timeout {
        msg.push_str("\n\nPossible solutions:");
        msg.push_str("\n  - Increase pool_size (current: ");
        msg.push_str(
            &pool_size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "1 (default)".to_string()),
        );
        msg.push(')');
        msg.push_str("\n  - Increase connection_timeout (current: ");
        msg.push_str(
            &timeout
                .map(|t| format!("{}s", t))
                .unwrap_or_else(|| "30s (default)".to_string()),
        );
        msg.push(')');
        msg.push_str("\n  - Ensure connections are properly released (use async context managers)");
        msg.push_str("\n  - Check for long-running transactions that hold connections");
    }

    OperationalError::new_err(msg)
}

/// Apply this Connection's PRAGMAs to a pooled connection so session pragmas
/// (e.g. set via set_pragma) are in effect when using a shared pool.
pub(crate) async fn apply_pragmas_to_connection(
    conn: &mut PoolConnection<sqlx::Sqlite>,
    pragmas: &[(String, String)],
    path: &str,
) -> Result<(), PyErr> {
    if pragmas.is_empty() {
        return Ok(());
    }
    let connection_id = {
        let sqlite_conn: &mut sqlx::SqliteConnection = conn;
        let mut handle = sqlite_conn
            .lock_handle()
            .await
            .map_err(|e| OperationalError::new_err(format!("Failed to lock SQLite handle: {e}")))?;
        handle.as_raw_handle().as_ptr() as usize
    };
    let fingerprint = pragma_fingerprint(pragmas);
    if pragmas_are_applied(connection_id, fingerprint) {
        return Ok(());
    }

    for (name, value) in pragmas {
        let pragma_query = format!("PRAGMA {name} = {value}");
        sqlx::query(&pragma_query)
            .execute(&mut **conn)
            .await
            .map_err(|e| {
                crate::errors::map_sqlx_error_with_visibility(e, path, &pragma_query, false)
            })?;
    }
    mark_pragmas_applied(connection_id, fingerprint);
    Ok(())
}

fn pragma_fingerprint(pragmas: &[(String, String)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    pragmas.hash(&mut hasher);
    hasher.finish()
}

fn applied_pragmas() -> &'static StdMutex<HashMap<usize, u64>> {
    static APPLIED: OnceLock<StdMutex<HashMap<usize, u64>>> = OnceLock::new();
    APPLIED.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn pragmas_are_applied(connection_id: usize, fingerprint: u64) -> bool {
    applied_pragmas()
        .lock()
        .map(|applied| applied.get(&connection_id).copied() == Some(fingerprint))
        .unwrap_or(false)
}

/// Forget state associated with a raw SQLite handle address when SQLx has
/// created a new physical connection. SQLite may reuse an address after the
/// previous handle is closed, so the address alone is not a stable identity.
fn forget_applied_pragmas(connection_id: usize) {
    if let Ok(mut applied) = applied_pragmas().lock() {
        applied.remove(&connection_id);
    }
}

fn mark_pragmas_applied(connection_id: usize, fingerprint: u64) {
    if let Ok(mut applied) = applied_pragmas().lock() {
        // Physical handles are normally reused by SQLx. Keep this bounded in case a
        // workload repeatedly creates and destroys pools with unique addresses.
        if applied.len() >= 4096 {
            applied.clear();
        }
        applied.insert(connection_id, fingerprint);
    }
}

/// Acquire a connection from the pool and apply the Connection's pragmas to it.
/// Use this whenever a Connection acquires a connection so set_pragma and
/// connect(pragmas=...) are respected with a shared pool.
pub(crate) async fn acquire_with_pragmas(
    pool: &SqlitePool,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    path: &str,
    pool_size_val: Option<usize>,
    timeout_val: Option<u64>,
) -> Result<PoolConnection<sqlx::Sqlite>, PyErr> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| pool_acquisition_error(path, &e, pool_size_val, timeout_val))?;
    let pragmas_list = pragmas.lock().unwrap().clone();
    apply_pragmas_to_connection(&mut conn, &pragmas_list, path).await?;
    Ok(conn)
}

/// Helper to get or create pool and apply PRAGMAs.
/// Uses a global path-based registry so connections to the same path share one pool.
pub(crate) async fn get_or_create_pool(
    path: &str,
    pool: &PoolHandle,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<SqlitePool, PyErr> {
    // Fast path: initialized once per Connection, with no async mutex lookup.
    if let Some(pool) = pool.get() {
        return Ok(pool.clone());
    }

    let registry = global_registry();

    // Check global registry for an existing pool for this path.
    let from_registry = {
        let reg = registry.lock().unwrap();
        reg.get(path).and_then(|entry| entry.pool.0.clone())
    };
    if let Some(shared_clone) = from_registry {
        pool.set(shared_clone);
        return Ok(pool.get().expect("pool handle initialized").clone());
    }

    // No pool for this path: create one, then register or use existing (race).
    let requested_max = {
        let g = pool_size.lock().unwrap();
        match *g {
            Some(configured) => configured.max(1).min(u32::MAX as usize) as u32,
            None => SHARED_POOL_MIN_CONNECTIONS,
        }
    };
    let max_conn = {
        let mut reg = global_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = reg.entry(path.to_string()).or_default();
        if entry.configured_max_connections.is_none() {
            entry.configured_max_connections = Some(requested_max);
        }
        entry.configured_max_connections.unwrap_or(requested_max)
    };
    let timeout_secs = {
        let g = connection_timeout_secs.lock().unwrap();
        *g
    };
    let idle_secs = {
        let g = idle_timeout_secs.lock().unwrap();
        *g
    };
    let mut opts = SqlitePoolOptions::new().max_connections(max_conn);
    let timeout = timeout_secs.unwrap_or(30);
    opts = opts.acquire_timeout(Duration::from_secs(timeout));
    if let Some(idle) = idle_secs {
        opts = opts.idle_timeout(Some(Duration::from_secs(idle)));
    }
    let after_connect_pragmas = pragmas.lock().unwrap().clone();
    opts = opts.after_connect(move |conn, _| {
        let pragmas = after_connect_pragmas.clone();
        Box::pin(async move {
            // SQLx invokes this hook exactly when a physical SQLite
            // connection is created. Raw sqlite3 addresses can be reused
            // after a connection is replaced, so invalidate any state left
            // behind for this address even when the initial PRAGMA snapshot
            // is empty. Later set_pragma() calls are then applied on acquire.
            let connection_id = {
                let mut handle = conn.lock_handle().await?;
                handle.as_raw_handle().as_ptr() as usize
            };
            forget_applied_pragmas(connection_id);
            for (name, value) in &pragmas {
                let pragma_query = format!("PRAGMA {name} = {value}");
                sqlx::query(&pragma_query).execute(&mut *conn).await?;
            }
            if !pragmas.is_empty() {
                mark_pragmas_applied(connection_id, pragma_fingerprint(&pragmas));
            }
            Ok(())
        })
    });
    let url = sqlite_url_for_path(path);
    let new_pool = opts.connect(&url).await.map_err(|e| {
        OperationalError::new_err(format!("Failed to connect to database at {path}: {e}"))
    })?;

    let to_use = {
        let mut reg = registry.lock().unwrap();
        let entry = reg.entry(path.to_string()).or_default();
        if let Some(existing) = entry.pool.0.clone() {
            Some(existing)
        } else {
            entry.pool.0 = Some(new_pool.clone());
            None
        }
    };
    match to_use {
        Some(existing) => {
            pool.set(existing);
            Ok(pool.get().expect("pool handle initialized").clone())
        }
        None => {
            pool.set(new_pool);
            Ok(pool.get().expect("pool handle initialized").clone())
        }
    }
}

/// Helper to ensure callback connection exists.
/// This acquires a connection from the pool and stores it for callback installation.
/// The connection is stored in the callback_connection mutex and should be accessed via that mutex.
/// Note: Accessing the raw sqlite3* handle from PoolConnection requires further research
/// into sqlx 0.8's API. This is a known limitation that needs to be resolved.
pub(crate) async fn ensure_callback_connection(
    path: &str,
    pool: &PoolHandle,
    callback_connection: &Arc<Mutex<PoolConnectionSlot>>,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<(), PyErr> {
    let mut callback_guard = callback_connection.lock().await;
    if callback_guard.0.is_none() {
        // Get or create pool first
        let pool_clone = get_or_create_pool(
            path,
            pool,
            pragmas,
            pool_size,
            connection_timeout_secs,
            idle_timeout_secs,
        )
        .await?;

        // Acquire a connection from the pool
        let pool_size_val = {
            let g = pool_size.lock().unwrap();
            *g
        };
        let timeout_val = {
            let g = connection_timeout_secs.lock().unwrap();
            *g
        };
        let pool_conn =
            acquire_with_pragmas(&pool_clone, pragmas, path, pool_size_val, timeout_val).await?;

        callback_guard.0 = Some(pool_conn);
    }
    Ok(())
}

/// Execute init_hook if it hasn't been called yet.
/// This should be called from the first operation method that uses the pool.
pub(crate) async fn execute_init_hook_if_needed(
    init_hook: &Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_present: &AtomicBool,
    init_hook_state: &InitHookState,
    reentrant: bool,
    connection: Py<crate::Connection>,
) -> Result<(), PyErr> {
    if !init_hook_present.load(Ordering::Acquire) {
        return Ok(());
    }

    // Once initialization is complete, the common path needs no Python/GIL
    // round-trip to check whether this operation is reentrant.
    if let Some(result) = init_hook_state.completion.get() {
        return match result.as_ref() {
            Ok(()) => Ok(()),
            Err(error) => Err(OperationalError::new_err(error.to_string())),
        };
    }

    // This was captured at the synchronous Python API boundary. Looking up a
    // ContextVar from a Tokio worker thread would observe a different context.
    if reentrant {
        return Ok(());
    }

    let result = init_hook_state
        .completion
        .get_or_init(|| async {
            init_hook_state.started.store(true, Ordering::Release);

            #[allow(deprecated)]
            let hook_opt = Python::attach(|py| {
                let guard = init_hook.lock().unwrap();
                guard.as_ref().map(|hook| hook.clone_ref(py))
            });
            let Some(hook) = hook_opt else {
                return Ok(());
            };

            #[allow(deprecated)]
            let coro_future = Python::attach(|py| -> PyResult<_> {
                let coro = py
                    .import("rapsqlite._init_hook")?
                    .getattr("_run_hook")?
                    .call1((hook.bind(py), connection.bind(py)))?;
                into_future(coro).map_err(|error| {
                    OperationalError::new_err(format!(
                        "Failed to convert init_hook coroutine to future: {error}"
                    ))
                })
            })
            .map_err(|error| Arc::<str>::from(error.to_string()))?;

            coro_future.await.map_err(|error| {
                Arc::<str>::from(format!("init_hook raised an exception: {error}"))
            })?;
            Ok(())
        })
        .await;

    match result.as_ref() {
        Ok(()) => Ok(()),
        Err(error) => Err(OperationalError::new_err(error.to_string())),
    }
}

/// Fast wrapper for hot paths: most connections have no init hook, so an atomic
/// presence check avoids taking the per-connection mutex on every operation.
pub(crate) async fn execute_init_hook_if_needed_fast(
    init_hook: &Arc<StdMutex<Option<Py<PyAny>>>>,
    init_hook_present: &AtomicBool,
    init_hook_state: &InitHookState,
    reentrant: bool,
    connection: Py<crate::Connection>,
) -> Result<(), PyErr> {
    execute_init_hook_if_needed(
        init_hook,
        init_hook_present,
        init_hook_state,
        reentrant,
        connection,
    )
    .await
}

/// Ensure the Connection has a session connection from the pool (acquire and store if None).
/// Used to reuse one connection per Connection for many queries when not in a transaction
/// and not using callbacks, matching aiosqlite behavior and improving concurrent-read performance.
pub(crate) async fn lock_session_connection(
    path: &str,
    pool: &PoolHandle,
    session_connection: &SessionConnectionSlot,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
    idle_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<SessionConnectionGuard, PyErr> {
    let pool_clone = get_or_create_pool(
        path,
        pool,
        pragmas,
        pool_size,
        connection_timeout_secs,
        idle_timeout_secs,
    )
    .await?;
    lock_session_connection_from_pool(
        path,
        &pool_clone,
        session_connection,
        pragmas,
        pool_size,
        connection_timeout_secs,
    )
    .await
}

/// Lock a session connection using a pool handle the caller already resolved.
/// Query paths that must initialize the pool before running an init hook can
/// reuse that handle here instead of taking the pool-slot mutex a second time.
pub(crate) async fn lock_session_connection_from_pool(
    path: &str,
    pool_clone: &SqlitePool,
    session_connection: &SessionConnectionSlot,
    pragmas: &Arc<StdMutex<Vec<(String, String)>>>,
    pool_size: &Arc<StdMutex<Option<usize>>>,
    connection_timeout_secs: &Arc<StdMutex<Option<u64>>>,
) -> Result<SessionConnectionGuard, PyErr> {
    let mut guard = session_connection.lock().await;
    if guard.0.is_none() {
        session_connection.raw_statement_cache.clear();
        let pool_size_val = *pool_size.lock().unwrap();
        let timeout_val = *connection_timeout_secs.lock().unwrap();
        let conn =
            acquire_with_pragmas(pool_clone, pragmas, path, pool_size_val, timeout_val).await?;
        guard.0 = Some(conn);
    } else if let Some(conn) = guard.0.as_mut() {
        let pragmas_list = pragmas.lock().unwrap().clone();
        apply_pragmas_to_connection(conn, &pragmas_list, path).await?;
    }
    Ok(SessionConnectionGuard {
        retain: Arc::clone(&session_connection.retain),
        guard: Some(guard),
        raw_statement_cache: session_connection.raw_statement_cache(),
        retain_transition: Arc::clone(&session_connection.retain_transition),
    })
}

/// Release the session connection (return to pool). Call on close() and when starting a transaction.
pub(crate) async fn release_session_connection(session_connection: &SessionConnectionSlot) {
    let mut guard = session_connection.lock().await;
    session_connection.raw_statement_cache.clear();
    let _ = guard.0.take();
}

/// Check if any callbacks are currently set.
pub(crate) fn has_callbacks(
    callback_connection_required: &Arc<StdMutex<bool>>,
    user_functions: &UserFunctions,
    user_aggregates: &UserAggregates,
    user_collations: &UserCollations,
    _trace_callback: &TraceCallback,
    authorizer_callback: &Arc<StdMutex<Option<Py<PyAny>>>>,
    progress_handler: &ProgressHandler,
) -> bool {
    // Safety: StdMutex::lock() only fails if the mutex is poisoned (another thread panicked).
    // In Python's GIL context and with proper error handling, this is extremely unlikely.
    // These are read-only operations, so unwrap() is acceptable.
    let load_ext = *callback_connection_required.lock().unwrap();
    let has_functions = !user_functions.lock().unwrap().is_empty();
    let has_aggregates = !user_aggregates.lock().unwrap().is_empty();
    let has_collations = !user_collations.lock().unwrap().is_empty();
    let has_authorizer = authorizer_callback.lock().unwrap().is_some();
    let has_progress = progress_handler.lock().unwrap().is_some();

    load_ext || has_functions || has_aggregates || has_collations || has_authorizer || has_progress
}

// Callback configuration is almost always empty for cache-style workloads. Keep a
// compact, lock-free summary for the hot path and update it only when callback
// configuration changes. The detailed registries remain the source of truth for
// installation/rebinding and compatibility behavior.
pub(crate) const CALLBACK_FEATURE_EXTENSION: u8 = 1 << 0;
pub(crate) const CALLBACK_FEATURE_FUNCTIONS: u8 = 1 << 1;
pub(crate) const CALLBACK_FEATURE_AGGREGATES: u8 = 1 << 2;
pub(crate) const CALLBACK_FEATURE_COLLATIONS: u8 = 1 << 3;
pub(crate) const CALLBACK_FEATURE_AUTHORIZER: u8 = 1 << 4;
pub(crate) const CALLBACK_FEATURE_PROGRESS: u8 = 1 << 5;

#[inline]
pub(crate) fn callbacks_enabled(features: &Arc<AtomicU8>) -> bool {
    features.load(Ordering::Acquire) != 0
}

pub(crate) fn refresh_callback_features(ctx: &crate::connection::CallbackContext) {
    let mut features = 0u8;
    if *ctx.callback_connection_required.lock().unwrap() {
        features |= CALLBACK_FEATURE_EXTENSION;
    }
    if !ctx.user_functions.lock().unwrap().is_empty() {
        features |= CALLBACK_FEATURE_FUNCTIONS;
    }
    if !ctx.user_aggregates.lock().unwrap().is_empty() {
        features |= CALLBACK_FEATURE_AGGREGATES;
    }
    if !ctx.user_collations.lock().unwrap().is_empty() {
        features |= CALLBACK_FEATURE_COLLATIONS;
    }
    if ctx.authorizer_callback.lock().unwrap().is_some() {
        features |= CALLBACK_FEATURE_AUTHORIZER;
    }
    if ctx.progress_handler.lock().unwrap().is_some() {
        features |= CALLBACK_FEATURE_PROGRESS;
    }
    ctx.callback_features.store(features, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::{forget_applied_pragmas, mark_pragmas_applied, pragmas_are_applied};

    #[test]
    fn fresh_connection_invalidates_reused_raw_handle_address() {
        let reused_address = usize::MAX - 1024;
        let fingerprint = 0x5a17;

        mark_pragmas_applied(reused_address, fingerprint);
        assert!(pragmas_are_applied(reused_address, fingerprint));

        forget_applied_pragmas(reused_address);

        assert!(!pragmas_are_applied(reused_address, fingerprint));
    }
}
