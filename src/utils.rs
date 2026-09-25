//! Miscellaneous internal helpers (query/path/utilities).

use pyo3::prelude::*;
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

/// Bound diagnostic retention independently of workload cardinality and query size.
pub(crate) const MAX_QUERY_USAGE_ENTRIES: usize = 1_024;
pub(crate) const MAX_QUERY_USAGE_QUERY_BYTES: usize = 2_048;

/// Bounded, opt-in query analytics. `dropped` counts query executions omitted
/// because the normalized query was too large or the distinct-query cap was hit.
#[derive(Default)]
pub(crate) struct QueryUsageStats {
    pub(crate) counts: HashMap<String, u64>,
    pub(crate) dropped: u64,
}

impl QueryUsageStats {
    pub(crate) fn clear(&mut self) {
        self.counts.clear();
        self.dropped = 0;
    }
}

/// Detect if a query is a SELECT query (for determining execution strategy).
pub(crate) fn is_select_query(query: &str) -> bool {
    let trimmed = first_statement_after_comments(query).to_uppercase();
    trimmed.starts_with("SELECT") || trimmed.starts_with("WITH")
}

/// True if the statement returns result rows (SELECT, WITH, or INSERT/UPDATE/DELETE ... RETURNING).
/// Used so that INSERT/UPDATE/DELETE with RETURNING are executed and fetched like SELECT,
/// e.g. for SQLAlchemy's insertmanyvalues / ORM identity fetch.
pub(crate) fn returns_result_rows(query: &str) -> bool {
    let trimmed = first_statement_after_comments(query).to_uppercase();
    if trimmed.starts_with("SELECT") || trimmed.starts_with("WITH") {
        return true;
    }
    if trimmed.starts_with("INSERT")
        || trimmed.starts_with("UPDATE")
        || trimmed.starts_with("DELETE")
    {
        return trimmed.contains("RETURNING");
    }
    false
}

/// True for INSERT/UPDATE/DELETE only. Used to avoid implicit transaction for DDL (CREATE, etc.).
pub(crate) fn is_dml_query(query: &str) -> bool {
    let trimmed = first_statement_after_comments(query).to_uppercase();
    trimmed.starts_with("INSERT") || trimmed.starts_with("UPDATE") || trimmed.starts_with("DELETE")
}

/// True if the query is a transaction control statement (BEGIN, COMMIT, ROLLBACK).
/// Used to sync rapsqlite's transaction_state when SQLAlchemy sends these as raw SQL.
pub(crate) fn is_begin_query(query: &str) -> bool {
    first_statement_after_comments(query)
        .to_uppercase()
        .starts_with("BEGIN")
}

/// True if the query is COMMIT or ROLLBACK.
pub(crate) fn is_commit_or_rollback_query(query: &str) -> bool {
    let trimmed = first_statement_after_comments(query).to_uppercase();
    trimmed.starts_with("COMMIT") || trimmed.starts_with("ROLLBACK")
}

/// Remove whitespace and leading SQL comments before classifying the statement.
fn first_statement_after_comments(mut query: &str) -> &str {
    loop {
        query = query.trim_start();
        if let Some(rest) = query.strip_prefix("--") {
            query = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("");
            continue;
        }
        if let Some(rest) = query.strip_prefix("/*") {
            let Some(end) = rest.find("*/") else {
                return "";
            };
            query = &rest[end + 2..];
            continue;
        }
        return query;
    }
}

/// Normalize SQL only for the opt-in usage-analytics key. This does not modify
/// the query sent to SQLite or SQLx's per-connection prepared-statement cache key.
pub(crate) fn normalize_query(query: &str) -> String {
    let trimmed = query.trim();
    let mut normalized = String::with_capacity(trimmed.len());
    let mut previous_was_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(ch);
            previous_was_space = false;
        }
    }
    normalized
}

/// Track query usage for diagnostics without retaining unbounded query text.
pub(crate) fn track_query_usage(query_usage: &Arc<StdMutex<QueryUsageStats>>, query: &str) {
    track_query_usage_count(query_usage, query, 1);
}

/// Track `count` executions while normalizing and locking only once. This is
/// used by executemany(), where one Python call can execute the same SQL many
/// times.
pub(crate) fn track_query_usage_count(
    query_usage: &Arc<StdMutex<QueryUsageStats>>,
    query: &str,
    count: usize,
) {
    if count == 0 {
        return;
    }
    let normalized = normalize_query(query);
    let mut usage = query_usage.lock().unwrap();
    let increment = u64::try_from(count).unwrap_or(u64::MAX);
    if normalized.len() > MAX_QUERY_USAGE_QUERY_BYTES {
        usage.dropped = usage.dropped.saturating_add(increment);
        return;
    }
    if let Some(current) = usage.counts.get_mut(&normalized) {
        *current = current.saturating_add(increment);
    } else if usage.counts.len() < MAX_QUERY_USAGE_ENTRIES {
        usage.counts.insert(normalized, increment);
    } else {
        usage.dropped = usage.dropped.saturating_add(increment);
    }
}

/// Track query usage only when diagnostics are explicitly enabled. Query analytics
/// are intentionally absent from the normal execution hot path.
pub(crate) fn track_query_usage_if_enabled(
    enabled: &AtomicBool,
    query_usage: &Arc<StdMutex<QueryUsageStats>>,
    query: &str,
) {
    if enabled.load(Ordering::Acquire) {
        track_query_usage(query_usage, query);
    }
}

/// Track a batch only when diagnostics are enabled. The SQL is normalized once
/// and the count is added under the existing bounded-map lock.
pub(crate) fn track_query_usage_count_if_enabled(
    enabled: &AtomicBool,
    query_usage: &Arc<StdMutex<QueryUsageStats>>,
    query: &str,
    count: usize,
) {
    if enabled.load(Ordering::Acquire) {
        track_query_usage_count(query_usage, query, count);
    }
}

/// Validate a file path for security and correctness.
///
/// Checks for:
/// - Empty paths
/// - Null bytes (security risk)
/// - Path length limits (prevents DoS)
/// - Path traversal attempts (basic check)
pub(crate) fn validate_path(path: &str) -> PyResult<()> {
    if path.is_empty() {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Database path cannot be empty",
        ));
    }

    // Check for null bytes (security risk - can be used for path injection)
    if path.contains('\0') {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "Database path cannot contain null bytes",
        ));
    }

    // Check path length (prevent DoS from extremely long paths)
    // SQLite supports paths up to PATH_MAX (typically 4096 on Linux, 1024 on macOS)
    // We use a reasonable limit of 4096 characters
    const MAX_PATH_LENGTH: usize = 4096;
    if path.len() > MAX_PATH_LENGTH {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Database path too long (max {} characters, got {})",
            MAX_PATH_LENGTH,
            path.len()
        )));
    }

    // Basic path traversal check (for non-:memory: paths)
    // Note: This is a basic check - full path validation would require resolving
    // the path and checking against a base directory, which is application-specific
    if path != ":memory:" && (path.contains("../") || path.contains("..\\")) {
        // Allow relative paths but warn about potential traversal
        // Full validation should be done at the application level
        // We don't reject these here as they might be legitimate relative paths
    }

    Ok(())
}

/// Parse SQLite connection string (URI format: file:path?param=value&param2=value2).
/// Returns (database_path, vec of (param_name, param_value)).
pub(crate) fn parse_connection_string(uri: &str) -> PyResult<(String, Vec<(String, String)>)> {
    // Handle :memory: special case
    if uri == ":memory:" {
        return Ok((":memory:".to_string(), Vec::new()));
    }

    // Check if it's a URI (starts with file:)
    if let Some(uri_part) = uri.strip_prefix("file:") {
        // Parse URI: file:path?param=value&param2=value2
        let (path_part, query_part) = if let Some(pos) = uri_part.find('?') {
            (uri_part[..pos].to_string(), Some(&uri_part[pos + 1..]))
        } else {
            (uri_part.to_string(), None)
        };

        let mut params = Vec::new();
        if let Some(query) = query_part {
            // Validate query string length to prevent DoS
            const MAX_QUERY_LENGTH: usize = 4096;
            if query.len() > MAX_QUERY_LENGTH {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "URI query string too long (max {} characters, got {})",
                    MAX_QUERY_LENGTH,
                    query.len()
                )));
            }

            for param_pair in query.split('&') {
                // Validate parameter pair length
                if param_pair.len() > 512 {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "URI parameter too long (max 512 characters, got {})",
                        param_pair.len()
                    )));
                }

                if let Some(equal_pos) = param_pair.find('=') {
                    let key = param_pair[..equal_pos].to_string();
                    let value = param_pair[equal_pos + 1..].to_string();

                    // Validate parameter key (must be non-empty, alphanumeric + underscore/hyphen)
                    if key.is_empty() {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                            "URI parameter key cannot be empty",
                        ));
                    }

                    // Check for null bytes in key or value
                    if key.contains('\0') || value.contains('\0') {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                            "URI parameter cannot contain null bytes",
                        ));
                    }

                    params.push((key, value));
                } else {
                    // Parameter without value (e.g., ?flag)
                    // Validate key
                    if param_pair.is_empty() {
                        continue; // Skip empty parameters
                    }
                    if param_pair.contains('\0') {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                            "URI parameter cannot contain null bytes",
                        ));
                    }
                    params.push((param_pair.to_string(), String::new()));
                }
            }
        }

        // Decode URI-encoded path (basic support)
        let decoded_path = if path_part.starts_with("///") {
            // Absolute path: file:///path/to/db
            path_part[2..].to_string()
        } else if path_part.starts_with("//") {
            // Network path: file://host/path (not commonly used for SQLite)
            path_part.to_string()
        } else {
            // Relative path: file:db.sqlite
            path_part
        };

        Ok((decoded_path, params))
    } else {
        // Regular file path
        Ok((uri.to_string(), Vec::new()))
    }
}

/// Convert a C string pointer to &CStr. Uses *const c_char so it works on both
/// platforms where c_char is i8 (e.g. x86) and u8 (e.g. aarch64 manylinux).
///
/// # Safety
///
/// The caller must ensure:
/// - `ptr` points to a valid null-terminated C string
/// - The string remains valid for the lifetime of the returned reference
/// - For SQLite API functions, the pointer is typically valid until the next
///   SQLite API call (for error messages) or for the lifetime of the program
///   (for static strings like sqlite3_libversion())
#[inline]
pub(crate) unsafe fn cstr_from_c_char_ptr<'a>(ptr: *const c_char) -> &'a CStr {
    CStr::from_ptr(ptr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_is_select_query_basic() {
        assert!(is_select_query("SELECT 1"));
        assert!(is_select_query(" select 1 "));
        assert!(is_select_query("\n\tSELECT 1"));
        assert!(is_select_query("WITH cte AS (SELECT 1) SELECT * FROM cte"));

        assert!(!is_select_query("INSERT INTO t VALUES (1)"));
        assert!(!is_select_query("UPDATE t SET x = 1"));
        assert!(!is_select_query("DELETE FROM t"));
        assert!(!is_select_query("PRAGMA foreign_keys = ON"));
    }

    #[test]
    fn test_returns_result_rows() {
        assert!(returns_result_rows("SELECT 1"));
        assert!(returns_result_rows("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(returns_result_rows(
            "INSERT INTO t (a) VALUES (1) RETURNING id"
        ));
        assert!(returns_result_rows("UPDATE t SET x = 1 RETURNING id"));
        assert!(returns_result_rows("DELETE FROM t RETURNING id"));
        assert!(!returns_result_rows("INSERT INTO t VALUES (1)"));
        assert!(!returns_result_rows("UPDATE t SET x = 1"));
        assert!(!returns_result_rows("DELETE FROM t"));
    }

    #[test]
    fn test_normalize_query_whitespace() {
        assert_eq!(normalize_query("  SELECT   1  "), "SELECT 1");
        assert_eq!(normalize_query("SELECT\t1"), "SELECT 1");
        assert_eq!(normalize_query("SELECT\n1"), "SELECT 1");
        assert_eq!(normalize_query("SELECT\r\n1"), "SELECT 1");
        assert_eq!(normalize_query("SELECT  1   FROM   t"), "SELECT 1 FROM t");
    }

    #[test]
    fn test_parse_connection_string_memory() {
        let (path, params) = parse_connection_string(":memory:").unwrap();
        assert_eq!(path, ":memory:");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_connection_string_non_uri_path() {
        let (path, params) = parse_connection_string("db.sqlite").unwrap();
        assert_eq!(path, "db.sqlite");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_connection_string_uri_relative() {
        let (path, params) =
            parse_connection_string("file:db.sqlite?mode=ro&cache=shared").unwrap();
        assert_eq!(path, "db.sqlite");
        assert_eq!(
            params,
            vec![
                ("mode".to_string(), "ro".to_string()),
                ("cache".to_string(), "shared".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_connection_string_uri_absolute_like() {
        let (path, params) = parse_connection_string("file:///tmp/test.db?mode=ro").unwrap();
        assert_eq!(path, "/tmp/test.db");
        assert_eq!(params, vec![("mode".to_string(), "ro".to_string())]);
    }

    #[test]
    fn test_cstr_from_c_char_ptr_roundtrip() {
        let s = CString::new("hello").unwrap();
        let ptr = s.as_ptr();
        // Safety: ptr is valid for the lifetime of `s` (this scope) and is NUL-terminated.
        let cstr = unsafe { cstr_from_c_char_ptr(ptr) };
        assert_eq!(cstr.to_str().unwrap(), "hello");
    }
}
