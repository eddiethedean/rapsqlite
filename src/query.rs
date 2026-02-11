//! Query execution/fetch helpers built on top of sqlx.

use pyo3::prelude::*;
use sqlx::pool::PoolConnection;

use crate::types::SqliteParam;

type SqliteQuery<'q> = sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>;

/// Build a sqlx query with all parameters bound, with a consistent limit.
///
/// All call sites share the same parameter limit and binding logic so that
/// execute/fetch paths behave identically.
fn build_bound_query<'q>(
    query: &'q str,
    params: &'q [SqliteParam],
) -> Result<SqliteQuery<'q>, sqlx::Error> {
    if params.is_empty() {
        return Ok(sqlx::query(query));
    }

    if params.len() > 50 {
        return Err(sqlx::Error::Protocol(format!(
            "Too many parameters ({}). Currently supporting up to 50 parameters.",
            params.len()
        )));
    }

    let qb = match params.len() {
        1 => bind_chain!(query, params, 0),
        2 => bind_chain!(query, params, 0, 1),
        3 => bind_chain!(query, params, 0, 1, 2),
        4 => bind_chain!(query, params, 0, 1, 2, 3),
        5 => bind_chain!(query, params, 0, 1, 2, 3, 4),
        6 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5),
        7 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6),
        8 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7),
        9 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        17 => bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16),
        18 => {
            bind_chain!(query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17)
        }
        19 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18
        ),
        20 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19
        ),
        21 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20
        ),
        22 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21
        ),
        23 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22
        ),
        24 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23
        ),
        25 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24
        ),
        26 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25
        ),
        27 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26
        ),
        28 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27
        ),
        29 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28
        ),
        30 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29
        ),
        31 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30
        ),
        32 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31
        ),
        33 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32
        ),
        34 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33
        ),
        35 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34
        ),
        36 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35
        ),
        37 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36
        ),
        38 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37
        ),
        39 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38
        ),
        40 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39
        ),
        41 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40
        ),
        42 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41
        ),
        43 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42
        ),
        44 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43
        ),
        45 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44
        ),
        46 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45
        ),
        47 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45, 46
        ),
        48 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45, 46, 47
        ),
        49 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45, 46, 47, 48
        ),
        50 => bind_chain!(
            query, params, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 43, 44, 45, 46, 47, 48, 49
        ),
        _ => unreachable!(),
    };

    Ok(qb)
}

/// Helper to bind parameters and execute on a specific connection.
/// Similar to bind_and_execute but takes a PoolConnection instead of Pool.
pub(crate) async fn bind_and_execute_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<sqlx::sqlite::SqliteQueryResult, PyErr> {
    // Use &mut **conn to access the underlying connection that implements Executor
    let result = match params.len() {
        0 => sqlx::query(query).execute(&mut **conn).await,
        1 => match &params[0] {
            SqliteParam::Null => {
                sqlx::query(query)
                    .bind(Option::<i64>::None)
                    .execute(&mut **conn)
                    .await
            }
            SqliteParam::Int(v) => sqlx::query(query).bind(*v).execute(&mut **conn).await,
            SqliteParam::Real(v) => sqlx::query(query).bind(*v).execute(&mut **conn).await,
            SqliteParam::Text(v) => {
                sqlx::query(query)
                    .bind(v.as_str())
                    .execute(&mut **conn)
                    .await
            }
            SqliteParam::Blob(v) => {
                sqlx::query(query)
                    .bind(v.as_slice())
                    .execute(&mut **conn)
                    .await
            }
        },
        _ => {
            // For multiple parameters, use bind_query_multiple_on_connection
            bind_query_multiple_on_connection(query, params, conn).await
        }
    };

    result.map_err(|e| crate::map_sqlx_error(e, path, query))
}

/// Helper to bind multiple parameters to a query and execute on a connection.
pub(crate) async fn bind_query_multiple_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    let query_builder = build_bound_query(query, params)?;
    query_builder.execute(&mut **conn).await
}

/// Helper to bind parameters and fetch all rows on a specific connection.
pub(crate) async fn bind_and_fetch_all_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, PyErr> {
    let query_builder =
        build_bound_query(query, params).map_err(|e| crate::map_sqlx_error(e, path, query))?;
    query_builder
        .fetch_all(&mut **conn)
        .await
        .map_err(|e| crate::map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch one row on a specific connection.
pub(crate) async fn bind_and_fetch_one_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<sqlx::sqlite::SqliteRow, PyErr> {
    let query_builder =
        build_bound_query(query, params).map_err(|e| crate::map_sqlx_error(e, path, query))?;
    query_builder
        .fetch_one(&mut **conn)
        .await
        .map_err(|e| crate::map_sqlx_error(e, path, query))
}

/// Helper to bind parameters and fetch optional row on a specific connection.
pub(crate) async fn bind_and_fetch_optional_on_connection(
    query: &str,
    params: &[SqliteParam],
    conn: &mut PoolConnection<sqlx::Sqlite>,
    path: &str,
) -> Result<Option<sqlx::sqlite::SqliteRow>, PyErr> {
    let query_builder =
        build_bound_query(query, params).map_err(|e| crate::map_sqlx_error(e, path, query))?;
    query_builder
        .fetch_optional(&mut **conn)
        .await
        .map_err(|e| crate::map_sqlx_error(e, path, query))
}
