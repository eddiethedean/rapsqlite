//! Error mapping helpers (sqlx -> Python exceptions, raw SQLite C API).

use pyo3::prelude::*;

use crate::exceptions::{DatabaseError, IntegrityError, OperationalError, ProgrammingError};

fn is_sql_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_sql_ignored(query: &str, mut i: usize, end: usize) -> usize {
    let bytes = query.as_bytes();
    loop {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < end && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < end && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < end && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(end);
            continue;
        }
        return i;
    }
}

/// Find a SQL keyword outside quoted strings, quoted identifiers, and comments.
fn find_sql_keyword(query_lower: &str, keyword: &str, from: usize) -> Option<usize> {
    let bytes = query_lower.as_bytes();
    let keyword_bytes = keyword.as_bytes();
    let mut i = from;
    let mut quote = None;

    while i < bytes.len() {
        if let Some(quote_byte) = quote {
            if bytes[i] == quote_byte {
                if i + 1 < bytes.len() && bytes[i + 1] == quote_byte {
                    i += 2;
                } else {
                    quote = None;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == b'`' {
            quote = Some(bytes[i]);
            i += 1;
            continue;
        }
        if bytes[i] == b'[' {
            while i < bytes.len() {
                if bytes[i] == b']' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        if bytes[i..].starts_with(keyword_bytes)
            && (i == 0 || !is_sql_identifier_byte(bytes[i - 1]))
            && (i + keyword_bytes.len() == bytes.len()
                || !is_sql_identifier_byte(bytes[i + keyword_bytes.len()]))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_sql_byte(query: &str, from: usize, needle: u8) -> Option<usize> {
    let bytes = query.as_bytes();
    let mut i = from;
    let mut quote = None;

    while i < bytes.len() {
        if let Some(quote_byte) = quote {
            if bytes[i] == quote_byte {
                if i + 1 < bytes.len() && bytes[i + 1] == quote_byte {
                    i += 2;
                } else {
                    quote = None;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == b'`' {
            quote = Some(bytes[i]);
            i += 1;
            continue;
        }
        if bytes[i] == b'[' {
            while i < bytes.len() {
                if bytes[i] == b']' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn matching_sql_paren(query: &str, open: usize) -> Option<usize> {
    let bytes = query.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    let mut quote = None;

    while i < bytes.len() {
        if let Some(quote_byte) = quote {
            if bytes[i] == quote_byte {
                if i + 1 < bytes.len() && bytes[i + 1] == quote_byte {
                    i += 2;
                } else {
                    quote = None;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        match bytes[i] {
            b'\'' | b'"' | b'`' => quote = Some(bytes[i]),
            b'[' => {
                while i < bytes.len() {
                    if bytes[i] == b']' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        i += 1;
    }
    None
}

fn split_sql_list(query: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let bytes = query.as_bytes();
    let mut ranges = Vec::new();
    let mut item_start = start;
    let mut depth = 0usize;
    let mut i = start;
    let mut quote = None;

    while i < end {
        if let Some(quote_byte) = quote {
            if bytes[i] == quote_byte {
                if i + 1 < end && bytes[i + 1] == quote_byte {
                    i += 2;
                } else {
                    quote = None;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        match bytes[i] {
            b'\'' | b'"' | b'`' => quote = Some(bytes[i]),
            b'[' => {
                while i < end {
                    if bytes[i] == b']' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                ranges.push((item_start, i));
                item_start = i + 1;
            }
            _ => {}
        }
        if bytes[i] == b'-' && i + 1 < end && bytes[i + 1] == b'-' {
            i += 2;
            while i < end && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < end && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < end && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(end);
            continue;
        }
        i += 1;
    }
    ranges.push((item_start, end));
    ranges
}

fn trim_sql_range(query: &str, (mut start, mut end): (usize, usize)) -> (usize, usize) {
    let bytes = query.as_bytes();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

fn sql_column_name(query: &str, range: (usize, usize)) -> Option<String> {
    let bytes = query.as_bytes();
    let (_, end) = range;
    let start = skip_sql_ignored(query, range.0, end);
    if start >= end {
        return None;
    }

    let (name_start, name_end) = match bytes[start] {
        b'"' | b'`' => {
            let quote = bytes[start];
            let mut i = start + 1;
            while i < end {
                if bytes[i] == quote {
                    if i + 1 < end && bytes[i + 1] == quote {
                        i += 2;
                    } else {
                        return Some(query[start + 1..i].to_ascii_lowercase());
                    }
                } else {
                    i += 1;
                }
            }
            return None;
        }
        b'[' => {
            let mut i = start + 1;
            while i < end {
                if bytes[i] == b']' {
                    return Some(query[start + 1..i].to_ascii_lowercase());
                }
                i += 1;
            }
            return None;
        }
        _ => {
            let mut i = start;
            while i < end && is_sql_identifier_byte(bytes[i]) {
                i += 1;
            }
            (start, i)
        }
    };

    (name_start < name_end).then(|| query[name_start..name_end].to_ascii_lowercase())
}

/// Find literal ranges belonging to sensitive columns in INSERT ... VALUES statements.
fn insert_sensitive_value_ranges(
    query: &str,
    query_lower: &str,
    sensitive_keywords: &[&str],
) -> Vec<(usize, usize)> {
    let Some(insert_pos) = find_sql_keyword(query_lower, "insert", 0) else {
        return Vec::new();
    };
    let Some(values_pos) = find_sql_keyword(query_lower, "values", insert_pos + "insert".len())
    else {
        return Vec::new();
    };
    let Some(columns_open) = find_sql_byte(query, insert_pos + "insert".len(), b'(') else {
        return Vec::new();
    };
    if columns_open >= values_pos {
        return Vec::new();
    }
    let Some(columns_close) = matching_sql_paren(query, columns_open) else {
        return Vec::new();
    };
    if columns_close > values_pos {
        return Vec::new();
    }

    let sensitive_indices: Vec<usize> = split_sql_list(query, columns_open + 1, columns_close)
        .into_iter()
        .enumerate()
        .filter_map(|(index, range)| {
            let column = sql_column_name(query, range)?;
            sensitive_keywords
                .iter()
                .any(|keyword| *keyword == column)
                .then_some(index)
        })
        .collect();
    if sensitive_indices.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut cursor = values_pos + "values".len();
    while let Some(open) = find_sql_byte(query, cursor, b'(') {
        let Some(close) = matching_sql_paren(query, open) else {
            break;
        };
        let values = split_sql_list(query, open + 1, close);
        for index in &sensitive_indices {
            if let Some(range) = values.get(*index) {
                let trimmed = trim_sql_range(query, *range);
                if trimmed.0 < trimmed.1 {
                    ranges.push(trimmed);
                }
            }
        }
        cursor = close + 1;
        let separator = skip_sql_ignored(query, cursor, query.len());
        if query.as_bytes().get(separator) == Some(&b',') {
            cursor = separator + 1;
        } else {
            break;
        }
    }
    ranges
}

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
    ranges.extend(insert_sensitive_value_ranges(
        query,
        &query_lower,
        &sensitive_keywords,
    ));
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
    fn test_sanitize_query_insert_sensitive_value() {
        let q = "INSERT INTO users (id, password) VALUES (1, 'TOPSECRET')";
        let out = sanitize_query(q);
        assert!(out.contains("VALUES (1, ***)"));
        assert!(!out.contains("TOPSECRET"));
    }

    #[test]
    fn test_sanitize_query_insert_sensitive_value_with_comments() {
        let q = "INSERT INTO users /* (comment) */ (password /* comma, */) VALUES ('TOPSECRET')";
        let out = sanitize_query(q);
        assert!(!out.contains("TOPSECRET"));
        assert!(out.contains("VALUES (***)"));
    }

    #[test]
    fn test_sanitize_query_insert_sensitive_values_after_comment() {
        let q = "INSERT INTO users (password) VALUES ('first') /* comment */ , ('TOPSECRET')";
        let out = sanitize_query(q);
        assert_eq!(out.matches("***").count(), 2);
        assert!(!out.contains("TOPSECRET"));
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
