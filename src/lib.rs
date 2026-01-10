use pyo3::prelude::*;

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Connection>()?;
    Ok(())
}

/// Async SQLite connection.
#[pyclass]
struct Connection {
    // TODO: Implement async SQLite connection
}

#[pymethods]
impl Connection {
    #[new]
    fn new(path: String) -> PyResult<Self> {
        // TODO: Implement async SQLite connection initialization
        Ok(Connection {})
    }

    fn execute(&self, query: String) -> PyResult<()> {
        // TODO: Implement async SQLite execute
        Ok(())
    }
}

