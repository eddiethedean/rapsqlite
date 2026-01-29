//! Error mapping helpers (sqlx -> Python exceptions).

use pyo3::prelude::*;

use crate::exceptions::{DatabaseError, IntegrityError, OperationalError, ProgrammingError};

/// Sanitize a query string to remove potentially sensitive information.
/// Replaces common sensitive patterns with placeholders.
/// 
/// This is a best-effort sanitization. For production use with highly sensitive data,
/// consider setting `include_query_in_errors=False` to exclude queries entirely.
fn sanitize_query(query: &str) -> String {
    let mut sanitized = query.to_string();
    let query_lower = query.to_lowercase();
    
    // Simple pattern matching for common sensitive fields
    // Note: This is basic sanitization - full regex would be better but requires
    // additional dependencies. For production, consider excluding queries entirely.
    
    // Match patterns like "password='value'" or "password=value" or "PASSWORD = value"
    let sensitive_keywords = ["password", "passwd", "secret", "token", "api_key", "api-key", "auth_token", "auth-token"];
    
    for keyword in sensitive_keywords.iter() {
        // Find the keyword in the query (case-insensitive)
        if let Some(pos) = query_lower.find(keyword) {
            // Try to find the value after '=' and replace it
            // This is a simplified approach - full regex would be more accurate
            if let Some(eq_pos) = query[pos..].find('=') {
                let start = pos + eq_pos + 1;
                // Skip whitespace after '='
                let start = query[start..]
                    .find(|c: char| !c.is_whitespace())
                    .map(|i| start + i)
                    .unwrap_or(start);
                
                // Find the end of the value (space, comma, quote, or end of string)
                let end = query[start..]
                    .find(|c: char| c == ' ' || c == ',' || c == '\'' || c == '"' || c == ';' || c == '\n' || c == '\r')
                    .map(|i| start + i)
                    .unwrap_or(query.len());
                
                if end > start {
                    sanitized.replace_range(start..end, "***");
                }
            }
        }
    }
    
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
