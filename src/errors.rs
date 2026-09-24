//! Error mapping helpers (sqlx -> Python exceptions, raw SQLite C API).

use pyo3::prelude::*;

use crate::exceptions::{DatabaseError, IntegrityError, OperationalError, ProgrammingError};

/// Sanitize a query string to remove potentially sensitive information.
/// Replaces common sensitive patterns with placeholders.
///
/// This is a best-effort sanitization. For production use with highly sensitive data,
/// consider setting `include_query_in_errors=False` to exclude queries entirely.
fn sanitize_query(query: &str) -> String {
    // ASCII lowercasing preserves UTF-8 byte lengths, so offsets still refer to
    // the original query even when it contains non-ASCII text.
    let query_lower = query.to_ascii_lowercase();

    // Simple pattern matching for common sensitive fields
    // Note: This is basic sanitization - full regex would be better but requires
    // additional dependencies. For production, consider excluding queries entirely.

    // Match patterns like "password='value'" or "password=value" or "PASSWORD = value"
    let sensitive_keywords = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "api-key",
        "auth_token",
        "auth-token",
    ];

    let mut ranges = Vec::new();
    for keyword in sensitive_keywords {
        let mut search_from = 0;
        while let Some(relative_pos) = query_lower[search_from..].find(keyword) {
            let pos = search_from + relative_pos;
            let after_keyword = pos + keyword.len();
            if let Some(relative_eq) = query[after_keyword..].find('=') {
                let after_eq = after_keyword + relative_eq + 1;
                let start = query[after_eq..]
                    .char_indices()
                    .find(|(_, ch)| !ch.is_whitespace())
                    .map(|(offset, _)| after_eq + offset)
                    .unwrap_or(query.len());

                if start < query.len() {
                    let first = query[start..].chars().next().unwrap();
                    let end = if first == '\'' || first == '"' {
                        let rest_start = start + first.len_utf8();
                        let mut scan_from = rest_start;
                        loop {
                            let Some(relative_quote) = query[scan_from..].find(first) else {
                                break query.len();
                            };
                            let quote_at = scan_from + relative_quote;
                            let after_quote = quote_at + first.len_utf8();
                            if query[after_quote..].starts_with(first) {
                                // SQL escapes a quote inside a literal by doubling it.
                                scan_from = after_quote + first.len_utf8();
                            } else {
                                break after_quote;
                            }
                        }
                    } else {
                        query[start..]
                            .find(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';')
                            .map(|offset| start + offset)
                            .unwrap_or(query.len())
                    };
                    if end > start {
                        ranges.push((start, end));
                    }
                }
            }
            search_from = after_keyword;
        }
    }

    ranges.sort_unstable();
    let mut sanitized = String::with_capacity(query.len());
    let mut copied_to = 0;
    for (start, end) in ranges {
        if start < copied_to {
            continue;
        }
        sanitized.push_str(&query[copied_to..start]);
        sanitized.push_str("***");
        copied_to = end;
    }
    sanitized.push_str(&query[copied_to..]);
    sanitized
}

/// Map sqlx error to appropriate Python exception.
///
/// Queries are automatically sanitized to remove sensitive patterns (passwords, tokens, etc.).
/// For production use with highly sensitive data, consider excluding queries entirely
/// by setting `include_query_in_errors=False` on the connection.
pub(crate) fn map_sqlx_error(e: sqlx::Error, path: &str, query: &str) -> PyErr {
    // Always sanitize queries to remove sensitive information
    let sanitized_query = sanitize_query(query);
    map_sqlx_error_with_query_visibility(e, path, &sanitized_query, true)
}

/// Map sqlx error while respecting the connection's query visibility setting.
pub(crate) fn map_sqlx_error_with_visibility(
    e: sqlx::Error,
    path: &str,
    query: &str,
    include_query: bool,
) -> PyErr {
    let sanitized_query = sanitize_query(query);
    map_sqlx_error_with_query_visibility(e, path, &sanitized_query, include_query)
}

/// Map sqlx error to appropriate Python exception with query visibility control.
pub(crate) fn map_sqlx_error_with_query_visibility(
    e: sqlx::Error,
    path: &str,
    query: &str,
    include_query: bool,
) -> PyErr {
    use sqlx::Error as SqlxError;

    let error_msg = if include_query {
        let sanitized_query = sanitize_query(query);
        format!("Failed to execute query on database {path}: {e}\nQuery: {sanitized_query}")
    } else {
        format!("Failed to execute query on database {path}: {e}")
    };

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

/// Map raw SQLite result code + message to Python exception (for use from non-Python threads).
pub(crate) fn map_sqlite_error_from_msg(
    path: &str,
    query: &str,
    rc: i32,
    msg: &str,
    include_query: bool,
) -> PyErr {
    let sanitized = sanitize_query(query);
    let error_msg = if include_query {
        format!("Failed to execute query on database {path}: {msg}\nQuery: {sanitized}")
    } else {
        format!("Failed to execute query on database {path}: {msg}")
    };
    let primary = rc & 0xff;
    match primary {
        libsqlite3_sys::SQLITE_CONSTRAINT => IntegrityError::new_err(error_msg),
        libsqlite3_sys::SQLITE_BUSY | libsqlite3_sys::SQLITE_LOCKED => {
            OperationalError::new_err(error_msg)
        }
        libsqlite3_sys::SQLITE_MISUSE | libsqlite3_sys::SQLITE_ERROR => {
            ProgrammingError::new_err(error_msg)
        }
        _ => DatabaseError::new_err(error_msg),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_query;

    #[test]
    fn test_sanitize_query_removes_password() {
        let q = "SELECT * FROM users WHERE password='secret123'";
        let out = sanitize_query(q);
        assert!(out.contains("***"));
        assert!(!out.contains("secret123"));
    }

    #[test]
    fn test_sanitize_query_unchanged_for_safe_query() {
        let q = "SELECT id, name FROM users WHERE id = 1";
        let out = sanitize_query(q);
        assert_eq!(out, q);
    }

    #[test]
    fn test_sanitize_query_token_value() {
        let q = "token=abc123";
        let out = sanitize_query(q);
        assert!(out.contains("***"));
        assert!(!out.contains("abc123"));
    }

    #[test]
    fn test_sanitize_query_keyword_equals_at_end_no_panic() {
        // Edge case: keyword followed by = at end of string (no value).
        // Ensures we don't panic on empty slice in chars().next().
        let q = "password=";
        let out = sanitize_query(q);
        assert_eq!(out, "password=");
    }

    #[test]
    fn test_sanitize_query_token_equals_at_end_no_panic() {
        let q = "token=";
        let out = sanitize_query(q);
        assert_eq!(out, "token=");
    }
}
