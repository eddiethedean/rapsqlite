//! SQL parameter parsing and binding helpers.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::types::SqliteParam;

/// Parse named parameters from SQL query and convert to positional.
/// Returns the processed query with ? placeholders and ordered parameter values.
pub(crate) fn process_named_parameters(
    query: &str,
    dict: &Bound<'_, PyDict>,
) -> PyResult<(String, Vec<SqliteParam>)> {
    let mut processed_query = query.to_string();
    let mut param_values = Vec::new();

    // Find all named parameter placeholders in order of appearance
    let mut param_placeholders: Vec<(usize, usize, String)> = Vec::new();
    let query_chars: Vec<char> = query.chars().collect();
    let mut i = 0;

    while i < query_chars.len() {
        let ch = query_chars[i];

        // Check for :name, @name, or $name patterns
        if (ch == ':' || ch == '@')
            && i + 1 < query_chars.len()
            && (query_chars[i + 1].is_alphabetic() || query_chars[i + 1] == '_')
        {
            let start = i;
            i += 1; // Skip the prefix
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
            i += 1; // Skip the $
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

    // Replace named parameters with ? and collect values in order
    // Process from end to start to avoid index shifting issues
    for (start, end, name) in param_placeholders.into_iter().rev() {
        if let Ok(Some(value)) = dict.get_item(name.as_str()) {
            let sqlx_param = SqliteParam::from_py(&value)?;
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
pub(crate) fn process_positional_parameters(
    list: &Bound<'_, PyList>,
) -> PyResult<Vec<SqliteParam>> {
    let mut param_values = Vec::new();
    for item in list.iter() {
        let param = SqliteParam::from_py(&item)?;
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
