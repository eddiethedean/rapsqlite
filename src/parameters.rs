//! SQL parameter parsing and binding helpers.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::types::{Adapters, SqliteParam};

/// Find all named parameter placeholders in order of appearance.
/// Returns (start_byte, end_byte, name) for each :name, @name, or $name.
pub(crate) fn find_named_parameter_placeholders(query: &str) -> Vec<(usize, usize, String)> {
    let mut param_placeholders: Vec<(usize, usize, String)> = Vec::new();
    let query_chars: Vec<char> = query.chars().collect();
    let mut i = 0;

    while i < query_chars.len() {
        let ch = query_chars[i];

        if (ch == ':' || ch == '@')
            && i + 1 < query_chars.len()
            && (query_chars[i + 1].is_alphabetic() || query_chars[i + 1] == '_')
        {
            let start = i;
            i += 1;
            let mut name = String::new();
            while i < query_chars.len() {
                let c = query_chars[i];
                if c.is_alphanumeric() || c == '_' {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            if !name.is_empty() {
                param_placeholders.push((start, i, name));
            }
        } else if ch == '$'
            && i + 1 < query_chars.len()
            && (query_chars[i + 1].is_alphabetic() || query_chars[i + 1] == '_')
        {
            let start = i;
            i += 1;
            let mut name = String::new();
            while i < query_chars.len() {
                let c = query_chars[i];
                if c.is_alphanumeric() || c == '_' {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }
            if !name.is_empty() {
                param_placeholders.push((start, i, name));
            }
        } else {
            i += 1;
        }
    }

    param_placeholders
}

/// Parse named parameters from SQL query and convert to positional.
/// Returns the processed query with ? placeholders and ordered parameter values.
/// If adapters is Some, apply registered adapters before converting each value to SqliteParam.
pub(crate) fn process_named_parameters(
    py: Python<'_>,
    query: &str,
    dict: &Bound<'_, PyDict>,
    adapters: Option<&Adapters>,
) -> PyResult<(String, Vec<SqliteParam>)> {
    let mut processed_query = query.to_string();
    let mut param_values = Vec::new();

    let param_placeholders = find_named_parameter_placeholders(query);

    // Replace named parameters with ? and collect values in order
    // Process from end to start to avoid index shifting issues
    for (start, end, name) in param_placeholders.into_iter().rev() {
        if let Ok(Some(value)) = dict.get_item(name.as_str()) {
            let sqlx_param = SqliteParam::apply_adapters_then_from_py(py, &value, adapters)?;
            param_values.push(sqlx_param);

            // Replace the named parameter with ?
            processed_query.replace_range(start..end, "?");
        } else {
            return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!(
                "Missing parameter: {name}"
            )));
        }
    }

    // Reverse to get correct order (we processed backwards)
    param_values.reverse();

    Ok((processed_query, param_values))
}

/// Process positional parameters from a list/tuple.
/// If adapters is Some, apply registered adapters before converting each value to SqliteParam.
pub(crate) fn process_positional_parameters(
    py: Python<'_>,
    list: &Bound<'_, PyList>,
    adapters: Option<&Adapters>,
) -> PyResult<Vec<SqliteParam>> {
    let mut param_values = Vec::new();
    for item in list.iter() {
        let param = SqliteParam::apply_adapters_then_from_py(py, &item, adapters)?;
        param_values.push(param);
    }
    Ok(param_values)
}

/// Macro to bind a chain of parameters to a query builder.
///
/// Kept as a macro because sqlx binding is expressed via method-chaining; this macro
/// generates the necessary bind chain for a fixed set of indices.
macro_rules! bind_chain {
    ($query:expr, $params:expr, $($idx:expr),*) => {
        {
            let q = sqlx::query($query);
            $(
                let q = match &$params[$idx] {
                    SqliteParam::Null => q.bind(Option::<i64>::None),
                    SqliteParam::Int(v) => q.bind(*v),
                    SqliteParam::Real(v) => q.bind(*v),
                    SqliteParam::Text(v) => q.bind(v.as_str()),
                    SqliteParam::Blob(v) => q.bind(v.as_slice()),
                };
            )*
            q
        }
    };
}

#[cfg(test)]
mod tests {
    use super::find_named_parameter_placeholders;

    #[test]
    fn test_find_named_placeholders_colon() {
        let out = find_named_parameter_placeholders("SELECT * FROM t WHERE id = :id");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2, "id");
    }

    #[test]
    fn test_find_named_placeholders_at() {
        let out = find_named_parameter_placeholders("INSERT INTO t (a) VALUES (@val)");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2, "val");
    }

    #[test]
    fn test_find_named_placeholders_dollar() {
        let out = find_named_parameter_placeholders("SELECT $name FROM t");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2, "name");
    }

    #[test]
    fn test_find_named_placeholders_multiple() {
        let out = find_named_parameter_placeholders("SELECT :a, @b, $c");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].2, "a");
        assert_eq!(out[1].2, "b");
        assert_eq!(out[2].2, "c");
    }

    #[test]
    fn test_find_named_placeholders_underscore_and_numbers() {
        let out = find_named_parameter_placeholders("WHERE col = :_ab12");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2, "_ab12");
    }

    #[test]
    fn test_find_named_placeholders_none() {
        assert!(find_named_parameter_placeholders("SELECT 1").is_empty());
        assert!(find_named_parameter_placeholders("").is_empty());
    }

    #[test]
    fn test_find_named_placeholders_colon_not_param() {
        // ":1" style positional is not a named param (must start with letter or _)
        let out = find_named_parameter_placeholders("SELECT :1");
        assert!(out.is_empty());
    }
}
