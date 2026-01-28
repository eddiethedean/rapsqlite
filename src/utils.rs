//! Miscellaneous internal helpers (query/path/utilities).

use pyo3::prelude::*;
use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::{Arc, Mutex as StdMutex};

/// Detect if a query is a SELECT query (for determining execution strategy).
pub(crate) fn is_select_query(query: &str) -> bool {
    let trimmed = query.trim().to_uppercase();
    trimmed.starts_with("SELECT") || trimmed.starts_with("WITH")
}

/// Normalize a SQL query by removing extra whitespace and standardizing formatting.
/// This helps improve prepared statement cache hit rates by ensuring queries with
/// different whitespace are treated as identical.
///
/// **Prepared Statement Caching (Phase 2.13):**
/// sqlx (the underlying database library) automatically caches prepared statements
/// per connection. When the same query is executed multiple times on the same
/// connection, sqlx reuses the prepared statement, providing significant performance
/// benefits. This normalization function ensures that queries with only whitespace
/// differences are treated as identical, maximizing cache hit rates.
///
/// The prepared statement cache is managed entirely by sqlx and does not require
/// explicit configuration. Each connection in the pool maintains its own cache,
/// and statements are automatically prepared on first use and reused for subsequent
/// executions of the same query.
pub(crate) fn normalize_query(query: &str) -> String {
    // Remove leading/trailing whitespace
    let trimmed = query.trim();
    // Replace multiple whitespace characters with single space
    let normalized: String = trimmed
        .chars()
        .fold((String::new(), false), |(acc, was_space), ch| {
            let is_space = ch.is_whitespace();
            if is_space && was_space {
                // Skip multiple consecutive spaces
                (acc, true)
            } else if is_space {
                // Replace any whitespace with single space
                (acc + " ", true)
            } else {
                (acc + &ch.to_string(), false)
            }
        })
        .0;
    normalized
}

/// Track query usage in the cache for analytics and optimization.
/// This helps identify frequently used queries that benefit from prepared statement caching.
pub(crate) fn track_query_usage(query_cache: &Arc<StdMutex<HashMap<String, u64>>>, query: &str) {
    let normalized = normalize_query(query);
    let mut cache = query_cache.lock().unwrap();
    *cache.entry(normalized).or_insert(0) += 1;
}

/// Validate a file path for security and correctness.
pub(crate) fn validate_path(path: &str) -> PyResult<()> {
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
            for param_pair in query.split('&') {
                if let Some(equal_pos) = param_pair.find('=') {
                    let key = param_pair[..equal_pos].to_string();
                    let value = param_pair[equal_pos + 1..].to_string();
                    params.push((key, value));
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

// Helper function to work around Rust version differences in CStr::from_ptr
// The signature of CStr::from_ptr varies by Rust version and platform
#[inline]
pub(crate) unsafe fn cstr_from_i8_ptr(ptr: *const i8) -> &'static CStr {
    // In Rust 1.93.0+, CStr::from_ptr accepts *const i8 directly
    // This helper provides a consistent interface across Rust versions
    CStr::from_ptr(ptr)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
