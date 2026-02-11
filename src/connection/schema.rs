//! Schema introspection (get_tables, get_table_info, get_indexes, etc.).
//! Single responsibility: run read-only introspection queries and convert results to Python.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyInt, PyList, PyString};
use sqlx::Row;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use crate::pool::{
    acquire_with_pragmas, ensure_callback_connection, execute_init_hook_if_needed,
    get_or_create_pool, has_callbacks, PoolConnectionSlot, PoolSlot,
};
use crate::query::bind_and_fetch_all_on_connection;
use crate::types::{
    ProgressHandler, TransactionState, UserAggregates, UserCollations, UserFunctions,
};
use crate::OperationalError;

use super::Connection;

use super::ensure_not_closed;

/// One table's introspection: (name, table_info rows, index rows, foreign_key rows)
type TableIntrospection = (
    String,
    Vec<sqlx::sqlite::SqliteRow>,
    Vec<sqlx::sqlite::SqliteRow>,
    Vec<sqlx::sqlite::SqliteRow>,
);

/// Context for running schema introspection queries. Built from Connection and passed
/// into schema async functions so they don't depend on the Connection pyclass.
pub(crate) struct SchemaContext {
    pub path: String,
    pub pool: Arc<Mutex<PoolSlot>>,
    pub pragmas: Arc<StdMutex<Vec<(String, String)>>>,
    pub pool_size: Arc<StdMutex<Option<usize>>>,
    pub connection_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub idle_timeout_secs: Arc<StdMutex<Option<u64>>>,
    pub transaction_state: Arc<Mutex<TransactionState>>,
    pub transaction_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub callback_connection: Arc<Mutex<PoolConnectionSlot>>,
    pub load_extension_enabled: Arc<StdMutex<bool>>,
    pub user_functions: UserFunctions,
    pub user_aggregates: UserAggregates,
    pub user_collations: UserCollations,
    pub trace_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub authorizer_callback: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub progress_handler: ProgressHandler,
    pub init_hook: Arc<StdMutex<Option<Py<PyAny>>>>,
    pub init_hook_called: Arc<StdMutex<bool>>,
    pub closed: Arc<StdMutex<bool>>,
    pub connection_self: Py<Connection>,
}

/// Run a single read-only introspection query using the appropriate connection
/// (transaction > callback > pool). Caller converts rows to Python.
pub(crate) async fn run_introspection_query(
    ctx: &SchemaContext,
    query: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, PyErr> {
    ensure_not_closed(&ctx.closed)?;

    let in_transaction = {
        let g = ctx.transaction_state.lock().await;
        g.is_active()
    };

    if !in_transaction {
        get_or_create_pool(
            &ctx.path,
            &ctx.pool,
            &ctx.pragmas,
            &ctx.pool_size,
            &ctx.connection_timeout_secs,
            &ctx.idle_timeout_secs,
        )
        .await?;
    }

    #[allow(deprecated)]
    let connection_for_hook = Python::with_gil(|py| ctx.connection_self.clone_ref(py));
    execute_init_hook_if_needed(&ctx.init_hook, &ctx.init_hook_called, connection_for_hook).await?;

    let has_callbacks_flag = has_callbacks(
        &ctx.load_extension_enabled,
        &ctx.user_functions,
        &ctx.user_aggregates,
        &ctx.user_collations,
        &ctx.trace_callback,
        &ctx.authorizer_callback,
        &ctx.progress_handler,
    );

    let rows = if in_transaction {
        let mut conn_guard = ctx.transaction_connection.lock().await;
        let conn = conn_guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Transaction connection not available"))?;
        bind_and_fetch_all_on_connection(query, &[], conn, &ctx.path).await?
    } else if has_callbacks_flag {
        ensure_callback_connection(
            &ctx.path,
            &ctx.pool,
            &ctx.callback_connection,
            &ctx.pragmas,
            &ctx.pool_size,
            &ctx.connection_timeout_secs,
            &ctx.idle_timeout_secs,
        )
        .await?;
        let mut conn_guard = ctx.callback_connection.lock().await;
        let conn = conn_guard
            .0
            .as_mut()
            .ok_or_else(|| OperationalError::new_err("Callback connection not available"))?;
        bind_and_fetch_all_on_connection(query, &[], conn, &ctx.path).await?
    } else {
        let pool_clone = get_or_create_pool(
            &ctx.path,
            &ctx.pool,
            &ctx.pragmas,
            &ctx.pool_size,
            &ctx.connection_timeout_secs,
            &ctx.idle_timeout_secs,
        )
        .await?;
        let pool_size_val = *ctx.pool_size.lock().map_err(|_| {
            crate::InternalError::new_err("internal error: mutex poisoned in schema")
        })?;
        let timeout_val = *ctx.connection_timeout_secs.lock().map_err(|_| {
            crate::InternalError::new_err("internal error: mutex poisoned in schema")
        })?;
        let mut conn = acquire_with_pragmas(
            &pool_clone,
            &ctx.pragmas,
            &ctx.path,
            pool_size_val,
            timeout_val,
        )
        .await?;
        bind_and_fetch_all_on_connection(query, &[], &mut conn, &ctx.path).await?
    };

    Ok(rows)
}

/// get_tables: list of table names (Python list of str).
pub(crate) async fn get_tables(ctx: SchemaContext, name: Option<String>) -> PyResult<Py<PyAny>> {
    let query = if let Some(ref table_name) = name {
        format!(
            "SELECT name FROM sqlite_master WHERE type='table' AND name = '{}' AND name NOT LIKE 'sqlite_%'",
            table_name.replace("'", "''")
        )
    } else {
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
    };

    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            if let Ok(table_name) = row.try_get::<String, _>(0) {
                result_list.append(PyString::new(py, &table_name))?;
            }
        }
        Ok(result_list.into())
    })
}

/// get_table_info: list of column dicts (PRAGMA table_info).
pub(crate) async fn get_table_info(ctx: SchemaContext, table_name: String) -> PyResult<Py<PyAny>> {
    let escaped = table_name.replace("'", "''");
    let query = format!("PRAGMA table_info('{escaped}')");
    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(cid) = row.try_get::<i64, _>(0) {
                dict.set_item("cid", PyInt::new(py, cid))?;
            }
            if let Ok(name) = row.try_get::<String, _>(1) {
                dict.set_item("name", PyString::new(py, &name))?;
            }
            if let Ok(col_type) = row.try_get::<String, _>(2) {
                dict.set_item("type", PyString::new(py, &col_type))?;
            }
            if let Ok(notnull) = row.try_get::<i64, _>(3) {
                dict.set_item("notnull", PyInt::new(py, notnull))?;
            }
            let dflt_val: Py<PyAny> = if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                PyString::new(py, &val).into()
            } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                PyInt::new(py, val).into()
            } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                PyFloat::new(py, val).into()
            } else {
                py.None()
            };
            dict.set_item("dflt_value", dflt_val)?;
            if let Ok(pk) = row.try_get::<i64, _>(5) {
                dict.set_item("pk", PyInt::new(py, pk))?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_indexes: list of index dicts (name, table, unique, sql).
pub(crate) async fn get_indexes(
    ctx: SchemaContext,
    table_name: Option<String>,
) -> PyResult<Py<PyAny>> {
    let query = if let Some(ref tbl_name) = table_name {
        let escaped = tbl_name.replace("'", "''");
        format!("SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND tbl_name = '{escaped}' AND name NOT LIKE 'sqlite_%' ORDER BY name")
    } else {
        "SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
    };

    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(name) = row.try_get::<String, _>(0) {
                dict.set_item("name", PyString::new(py, &name))?;
            }
            if let Ok(tbl_name) = row.try_get::<String, _>(1) {
                dict.set_item("table", PyString::new(py, &tbl_name))?;
            }
            let unique = if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                if sql.to_uppercase().contains("UNIQUE") {
                    1
                } else {
                    0
                }
            } else {
                0
            };
            dict.set_item("unique", PyInt::new(py, unique))?;
            if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                dict.set_item("sql", PyString::new(py, &sql))?;
            } else {
                dict.set_item("sql", py.None())?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_foreign_keys: list of FK dicts (PRAGMA foreign_key_list).
pub(crate) async fn get_foreign_keys(
    ctx: SchemaContext,
    table_name: String,
) -> PyResult<Py<PyAny>> {
    let escaped = table_name.replace("'", "''");
    let query = format!("PRAGMA foreign_key_list('{escaped}')");
    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(id) = row.try_get::<i64, _>(0) {
                dict.set_item("id", PyInt::new(py, id))?;
            }
            if let Ok(seq) = row.try_get::<i64, _>(1) {
                dict.set_item("seq", PyInt::new(py, seq))?;
            }
            if let Ok(ref_table) = row.try_get::<String, _>(2) {
                dict.set_item("table", PyString::new(py, &ref_table))?;
            }
            if let Ok(from_col) = row.try_get::<String, _>(3) {
                dict.set_item("from", PyString::new(py, &from_col))?;
            }
            if let Ok(to_col) = row.try_get::<String, _>(4) {
                dict.set_item("to", PyString::new(py, &to_col))?;
            }
            if let Ok(on_update) = row.try_get::<String, _>(5) {
                dict.set_item("on_update", PyString::new(py, &on_update))?;
            }
            if let Ok(on_delete) = row.try_get::<String, _>(6) {
                dict.set_item("on_delete", PyString::new(py, &on_delete))?;
            }
            if let Ok(match_val) = row.try_get::<String, _>(7) {
                dict.set_item("match", PyString::new(py, &match_val))?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_views: list of view names.
pub(crate) async fn get_views(ctx: SchemaContext, name: Option<String>) -> PyResult<Py<PyAny>> {
    let query = if let Some(ref view_name) = name {
        format!(
            "SELECT name FROM sqlite_master WHERE type='view' AND name = '{}'",
            view_name.replace("'", "''")
        )
    } else {
        "SELECT name FROM sqlite_master WHERE type='view' ORDER BY name".to_string()
    };

    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            if let Ok(view_name) = row.try_get::<String, _>(0) {
                result_list.append(PyString::new(py, &view_name))?;
            }
        }
        Ok(result_list.into())
    })
}

/// get_index_list: PRAGMA index_list (seq, name, unique, origin, partial).
pub(crate) async fn get_index_list(ctx: SchemaContext, table_name: String) -> PyResult<Py<PyAny>> {
    let escaped = table_name.replace("'", "''");
    let query = format!("PRAGMA index_list('{escaped}')");
    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(seq) = row.try_get::<i64, _>(0) {
                dict.set_item("seq", PyInt::new(py, seq))?;
            }
            if let Ok(name) = row.try_get::<String, _>(1) {
                dict.set_item("name", PyString::new(py, &name))?;
            }
            if let Ok(unique) = row.try_get::<i64, _>(2) {
                dict.set_item("unique", PyInt::new(py, unique))?;
            }
            if let Ok(Some(origin)) = row.try_get::<Option<String>, _>(3) {
                dict.set_item("origin", PyString::new(py, &origin))?;
            } else {
                dict.set_item("origin", py.None())?;
            }
            if let Ok(partial) = row.try_get::<i64, _>(4) {
                dict.set_item("partial", PyInt::new(py, partial))?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_index_info: PRAGMA index_info (seqno, cid, name).
pub(crate) async fn get_index_info(ctx: SchemaContext, index_name: String) -> PyResult<Py<PyAny>> {
    let escaped = index_name.replace("'", "''");
    let query = format!("PRAGMA index_info('{escaped}')");
    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(seqno) = row.try_get::<i64, _>(0) {
                dict.set_item("seqno", PyInt::new(py, seqno))?;
            }
            if let Ok(cid) = row.try_get::<i64, _>(1) {
                dict.set_item("cid", PyInt::new(py, cid))?;
            }
            if let Ok(name) = row.try_get::<String, _>(2) {
                dict.set_item("name", PyString::new(py, &name))?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_table_xinfo: PRAGMA table_xinfo (cid, name, type, notnull, dflt_value, pk, hidden).
pub(crate) async fn get_table_xinfo(ctx: SchemaContext, table_name: String) -> PyResult<Py<PyAny>> {
    let escaped = table_name.replace("'", "''");
    let query = format!("PRAGMA table_xinfo('{escaped}')");
    let rows = run_introspection_query(&ctx, &query).await?;

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let result_list = PyList::empty(py);
        for row in rows.iter() {
            let dict = PyDict::new(py);
            if let Ok(cid) = row.try_get::<i64, _>(0) {
                dict.set_item("cid", PyInt::new(py, cid))?;
            }
            if let Ok(name) = row.try_get::<String, _>(1) {
                dict.set_item("name", PyString::new(py, &name))?;
            }
            if let Ok(col_type) = row.try_get::<String, _>(2) {
                dict.set_item("type", PyString::new(py, &col_type))?;
            }
            if let Ok(notnull) = row.try_get::<i64, _>(3) {
                dict.set_item("notnull", PyInt::new(py, notnull))?;
            }
            let dflt_val: Py<PyAny> = if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                PyString::new(py, &val).into()
            } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                PyInt::new(py, val).into()
            } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                PyFloat::new(py, val).into()
            } else {
                py.None()
            };
            dict.set_item("dflt_value", dflt_val)?;
            if let Ok(pk) = row.try_get::<i64, _>(5) {
                dict.set_item("pk", PyInt::new(py, pk))?;
            }
            if let Ok(hidden) = row.try_get::<i64, _>(6) {
                dict.set_item("hidden", PyInt::new(py, hidden))?;
            }
            result_list.append(dict)?;
        }
        Ok(result_list.into())
    })
}

/// get_schema: comprehensive schema for one table or all tables (multiple queries).
pub(crate) async fn get_schema(
    ctx: SchemaContext,
    table_name: Option<String>,
) -> PyResult<Py<PyAny>> {
    let tables_query = if let Some(ref tbl_name) = table_name {
        format!(
            "SELECT name FROM sqlite_master WHERE type='table' AND name = '{}' AND name NOT LIKE 'sqlite_%'",
            tbl_name.replace("'", "''")
        )
    } else {
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name".to_string()
    };

    let tables_rows = run_introspection_query(&ctx, &tables_query).await?;
    let mut table_names = Vec::new();
    for row in tables_rows.iter() {
        if let Ok(name) = row.try_get::<String, _>(0) {
            table_names.push(name);
        }
    }

    let mut tables_info: Vec<TableIntrospection> = Vec::new();
    for tbl_name in &table_names {
        let info_query = format!("PRAGMA table_info('{}')", tbl_name.replace("'", "''"));
        let info_rows = run_introspection_query(&ctx, &info_query).await?;
        let indexes_query = format!(
            "SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND tbl_name = '{}' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            tbl_name.replace("'", "''")
        );
        let indexes_rows = run_introspection_query(&ctx, &indexes_query).await?;
        let fk_query = format!("PRAGMA foreign_key_list('{}')", tbl_name.replace("'", "''"));
        let fk_rows = run_introspection_query(&ctx, &fk_query).await?;
        tables_info.push((tbl_name.clone(), info_rows, indexes_rows, fk_rows));
    }

    #[allow(deprecated)]
    Python::with_gil(|py| -> PyResult<Py<PyAny>> {
        let schema_dict = PyDict::new(py);
        if let Some(ref tbl_name) = table_name {
            if let Some((_, info_rows, indexes_rows, fk_rows)) = tables_info.first() {
                let columns_list = PyList::empty(py);
                for row in info_rows.iter() {
                    let dict = PyDict::new(py);
                    if let Ok(cid) = row.try_get::<i64, _>(0) {
                        dict.set_item("cid", PyInt::new(py, cid))?;
                    }
                    if let Ok(name) = row.try_get::<String, _>(1) {
                        dict.set_item("name", PyString::new(py, &name))?;
                    }
                    if let Ok(col_type) = row.try_get::<String, _>(2) {
                        dict.set_item("type", PyString::new(py, &col_type))?;
                    }
                    if let Ok(notnull) = row.try_get::<i64, _>(3) {
                        dict.set_item("notnull", PyInt::new(py, notnull))?;
                    }
                    let dflt_val: Py<PyAny> =
                        if let Ok(Some(val)) = row.try_get::<Option<String>, _>(4) {
                            PyString::new(py, &val).into()
                        } else if let Ok(Some(val)) = row.try_get::<Option<i64>, _>(4) {
                            PyInt::new(py, val).into()
                        } else if let Ok(Some(val)) = row.try_get::<Option<f64>, _>(4) {
                            PyFloat::new(py, val).into()
                        } else {
                            py.None()
                        };
                    dict.set_item("dflt_value", dflt_val)?;
                    if let Ok(pk) = row.try_get::<i64, _>(5) {
                        dict.set_item("pk", PyInt::new(py, pk))?;
                    }
                    columns_list.append(dict)?;
                }
                schema_dict.set_item("columns", columns_list)?;
                let indexes_list = PyList::empty(py);
                for row in indexes_rows.iter() {
                    let dict = PyDict::new(py);
                    if let Ok(name) = row.try_get::<String, _>(0) {
                        dict.set_item("name", PyString::new(py, &name))?;
                    }
                    if let Ok(tn) = row.try_get::<String, _>(1) {
                        dict.set_item("table", PyString::new(py, &tn))?;
                    }
                    let unique = if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                        if sql.to_uppercase().contains("UNIQUE") {
                            1
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    dict.set_item("unique", PyInt::new(py, unique))?;
                    if let Ok(Some(sql)) = row.try_get::<Option<String>, _>(2) {
                        dict.set_item("sql", PyString::new(py, &sql))?;
                    } else {
                        dict.set_item("sql", py.None())?;
                    }
                    indexes_list.append(dict)?;
                }
                schema_dict.set_item("indexes", indexes_list)?;
                let fk_list = PyList::empty(py);
                for row in fk_rows.iter() {
                    let dict = PyDict::new(py);
                    if let Ok(id) = row.try_get::<i64, _>(0) {
                        dict.set_item("id", PyInt::new(py, id))?;
                    }
                    if let Ok(seq) = row.try_get::<i64, _>(1) {
                        dict.set_item("seq", PyInt::new(py, seq))?;
                    }
                    if let Ok(ref_table) = row.try_get::<String, _>(2) {
                        dict.set_item("table", PyString::new(py, &ref_table))?;
                    }
                    if let Ok(from_col) = row.try_get::<String, _>(3) {
                        dict.set_item("from", PyString::new(py, &from_col))?;
                    }
                    if let Ok(to_col) = row.try_get::<String, _>(4) {
                        dict.set_item("to", PyString::new(py, &to_col))?;
                    }
                    if let Ok(on_update) = row.try_get::<String, _>(5) {
                        dict.set_item("on_update", PyString::new(py, &on_update))?;
                    }
                    if let Ok(on_delete) = row.try_get::<String, _>(6) {
                        dict.set_item("on_delete", PyString::new(py, &on_delete))?;
                    }
                    if let Ok(match_val) = row.try_get::<String, _>(7) {
                        dict.set_item("match", PyString::new(py, &match_val))?;
                    }
                    fk_list.append(dict)?;
                }
                schema_dict.set_item("foreign_keys", fk_list)?;
                schema_dict.set_item("table_name", PyString::new(py, tbl_name))?;
            }
        } else {
            let tables_list = PyList::empty(py);
            for (tbl_name, _, _, _) in &tables_info {
                let table_dict = PyDict::new(py);
                table_dict.set_item("name", PyString::new(py, tbl_name))?;
                tables_list.append(table_dict)?;
            }
            schema_dict.set_item("tables", tables_list)?;
        }
        Ok(schema_dict.into())
    })
}
