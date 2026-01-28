//! `RapRow` implementation (aiosqlite-compatible row type).

use pyo3::prelude::*;

/// Row class for dict-like access to query results (similar to aiosqlite.Row).
#[pyclass]
pub(crate) struct RapRow {
    columns: Vec<String>,
    values: Vec<Py<PyAny>>,
}

#[pymethods]
impl RapRow {
    /// Create a new Row from column names and values.
    #[new]
    fn new(columns: Vec<String>, values: Vec<Py<PyAny>>) -> PyResult<Self> {
        if columns.len() != values.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Columns and values must have the same length",
            ));
        }
        Ok(RapRow { columns, values })
    }

    /// Get item by index or column name.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Try integer index first
        if let Ok(idx) = key.extract::<usize>() {
            if idx >= self.values.len() {
                return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(format!(
                    "Index {idx} out of range"
                )));
            }
            return Ok(self.values[idx].clone_ref(py));
        }

        // Try string column name
        if let Ok(col_name) = key.extract::<String>() {
            if let Some(idx) = self.columns.iter().position(|c| c == &col_name) {
                return Ok(self.values[idx].clone_ref(py));
            }
            return Err(PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!(
                "Column '{col_name}' not found"
            )));
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "Key must be int or str",
        ))
    }

    /// Get number of columns.
    fn __len__(&self) -> usize {
        self.values.len()
    }

    /// Check if row contains a column.
    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        // Try string column name
        if let Ok(col_name) = key.extract::<String>() {
            return Ok(self.columns.contains(&col_name));
        }

        // Try integer index
        if let Ok(idx) = key.extract::<usize>() {
            return Ok(idx < self.values.len());
        }

        Ok(false)
    }

    /// Get column names.
    fn keys(&self) -> PyResult<Vec<String>> {
        Ok(self.columns.clone())
    }

    /// Get values.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        Ok(self.values.iter().map(|v| v.clone_ref(py)).collect())
    }

    /// Get items as (column_name, value) pairs.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(String, Py<PyAny>)>> {
        Ok(self
            .columns
            .iter()
            .zip(self.values.iter())
            .map(|(col, val)| (col.clone(), val.clone_ref(py)))
            .collect())
    }

    /// Iterate over column names.
    fn __iter__(&self) -> PyResult<Vec<String>> {
        Ok(self.columns.clone())
    }

    /// String representation.
    fn __str__(&self, py: Python<'_>) -> PyResult<String> {
        let items: Vec<String> = self
            .columns
            .iter()
            .zip(self.values.iter())
            .map(|(col, val)| {
                let val_str = val
                    .bind(py)
                    .repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "?".to_string());
                format!("{col}={val_str}")
            })
            .collect();
        Ok(format!("Row({})", items.join(", ")))
    }

    /// Repr representation.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        self.__str__(py)
    }
}
