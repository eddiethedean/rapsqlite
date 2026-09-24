//! SQL parameter parsing and binding helpers.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::types::{Adapters, SqliteParam};

/// Find all named parameter placeholders in order of appearance.
/// Returns (start_byte, end_byte, name) for each :name, @name, or $name.
pub(crate) fn find_named_parameter_placeholders(query: &str) -> Vec<(usize, usize, String)> {
    let mut param_placeholders: Vec<(usize, usize, String)> = Vec::new();
    let query_chars: Vec<(usize, char)> = query.char_indices().collect();
    let mut i = 0;

    while i < query_chars.len() {
        let (byte_index, ch) = query_chars[i];

        // Ignore text that SQLite treats as quoted content or a comment. Named
        // parameter-looking text there is literal SQL, not a bind parameter.
        if ch == '\'' || ch == '"' || ch == '`' {
            let quote = ch;
            i += 1;
            while i < query_chars.len() {
                if query_chars[i].1 == quote {
                    // SQL escapes quote characters by doubling them.
                    if i + 1 < query_chars.len() && query_chars[i + 1].1 == quote {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == '[' {
            i += 1;
            while i < query_chars.len() {
                if query_chars[i].1 == ']' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == '-' && i + 1 < query_chars.len() && query_chars[i + 1].1 == '-' {
            i += 2;
            while i < query_chars.len() && query_chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < query_chars.len() && query_chars[i + 1].1 == '*' {
            i += 2;
            while i + 1 < query_chars.len()
                && !(query_chars[i].1 == '*' && query_chars[i + 1].1 == '/')
            {
                i += 1;
            }
            i = (i + 2).min(query_chars.len());
            continue;
        }

        let first_name_char = |candidate: char| {
            candidate.is_alphabetic() || candidate == '_' || (ch == '$' && candidate.is_numeric())
        };
        let is_named_prefix = (ch == ':' || ch == '@' || ch == '$')
            && i + 1 < query_chars.len()
            && first_name_char(query_chars[i + 1].1);

        if is_named_prefix {
            let start = byte_index;
            i += 1;
            let mut name = String::new();
            while i < query_chars.len() {
                let c = query_chars[i].1;
                if c.is_alphanumeric() || c == '_' {
                    name.push(c);
                    i += 1;
                } else {
                    break;
                }
            }

            // SQLite permits `$` parameter names to contain `::` components and
            // an optional parenthesized suffix, e.g. `$value::suffix(extra)`.
            // Keep the complete token together so the dictionary key matches
            // SQLite's parameter name (`value::suffix(extra)`).
            if ch == '$' {
                loop {
                    if i + 1 >= query_chars.len()
                        || query_chars[i].1 != ':'
                        || query_chars[i + 1].1 != ':'
                    {
                        break;
                    }
                    let component_start = i;
                    let name_len_before_component = name.len();
                    name.push(':');
                    name.push(':');
                    i += 2;
                    let component_name_start = i;
                    while i < query_chars.len() {
                        let c = query_chars[i].1;
                        if c.is_alphanumeric() || c == '_' {
                            name.push(c);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if i == component_name_start {
                        // Do not consume an incomplete `::` component.
                        name.truncate(name_len_before_component);
                        i = component_start;
                        break;
                    }
                }

                if i < query_chars.len() && query_chars[i].1 == '(' {
                    let suffix_start = i;
                    let name_len_before_suffix = name.len();
                    name.push('(');
                    i += 1;
                    let mut closed = false;
                    while i < query_chars.len() {
                        let c = query_chars[i].1;
                        name.push(c);
                        i += 1;
                        if c == ')' {
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        // SQLite only recognizes a complete parenthesized suffix.
                        name.truncate(name_len_before_suffix);
                        i = suffix_start;
                    }
                }
            }
            if !name.is_empty() {
                let end = query_chars
                    .get(i)
                    .map(|(byte_index, _)| *byte_index)
                    .unwrap_or(query.len());
                param_placeholders.push((start, end, name));
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

/// Process positional parameters from a list.
/// If adapters is Some, apply registered adapters before converting each value to SqliteParam.
pub(crate) fn process_positional_parameters(
    py: Python<'_>,
    list: &Bound<'_, PyList>,
    adapters: Option<&Adapters>,
) -> PyResult<Vec<SqliteParam>> {
    process_positional_parameters_iter(py, list.iter(), adapters)
}

/// Process positional parameters from a tuple (e.g. SQLAlchemy passes tuples for qmark).
pub(crate) fn process_positional_parameters_tuple(
    py: Python<'_>,
    tup: &Bound<'_, PyTuple>,
    adapters: Option<&Adapters>,
) -> PyResult<Vec<SqliteParam>> {
    process_positional_parameters_iter(py, tup.iter(), adapters)
}

/// Parse query parameters from optional Python value (dict, list, tuple, or single value).
/// Returns (processed_query, param_values). Use this to avoid duplicating the same
/// branch logic in Connection and Cursor.
pub(crate) fn process_parameters(
    py: Python<'_>,
    query: &str,
    params: Option<&Bound<'_, PyAny>>,
    adapters: Option<&Adapters>,
) -> PyResult<(String, Vec<SqliteParam>)> {
    let params = match params {
        None => return Ok((query.to_string(), Vec::new())),
        Some(p) => p.as_borrowed(),
    };
    if let Ok(dict) = params.cast::<PyDict>() {
        return process_named_parameters(py, query, &dict, adapters);
    }
    if let Ok(list) = params.cast::<PyList>() {
        let v = process_positional_parameters(py, &list, adapters)?;
        return Ok((query.to_string(), v));
    }
    if let Ok(tup) = params.cast::<PyTuple>() {
        let v = process_positional_parameters_tuple(py, &tup, adapters)?;
        return Ok((query.to_string(), v));
    }
    let param = SqliteParam::apply_adapters_then_from_py(py, &params, adapters)?;
    Ok((query.to_string(), vec![param]))
}

fn process_positional_parameters_iter<'a>(
    py: Python<'_>,
    iter: impl Iterator<Item = Bound<'a, PyAny>>,
    adapters: Option<&Adapters>,
) -> PyResult<Vec<SqliteParam>> {
    let mut param_values = Vec::new();
    for item in iter {
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
    fn test_find_named_placeholders_dollar_components() {
        let query = "SELECT $value::suffix, $other::part::detail FROM t";
        let out = find_named_parameter_placeholders(query);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].2, "value::suffix");
        assert_eq!(&query[out[0].0..out[0].1], "$value::suffix");
        assert_eq!(out[1].2, "other::part::detail");
        assert_eq!(&query[out[1].0..out[1].1], "$other::part::detail");
    }

    #[test]
    fn test_find_named_placeholders_dollar_parenthesized_suffix() {
        let query = "SELECT $value::suffix(extra) FROM t";
        let out = find_named_parameter_placeholders(query);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].2, "value::suffix(extra)");
        assert_eq!(&query[out[0].0..out[0].1], "$value::suffix(extra)");
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
