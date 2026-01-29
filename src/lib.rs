#![allow(non_local_definitions)] // False positive from pyo3 macros

mod connection;
pub(crate) use connection::Connection;

mod context_managers;
pub(crate) use context_managers::{ExecuteContextManager, TransactionContextManager};

mod cursor;
pub(crate) use cursor::Cursor;

use pyo3::prelude::*;

mod exceptions;
use exceptions::{
    DatabaseError, Error, IntegrityError, OperationalError, ProgrammingError, ValueError, Warning,
};

mod types;

mod utils;

mod conversion;

#[macro_use]
mod parameters;

mod query;

mod pool;

mod errors;
pub(crate) use errors::map_sqlx_error;

mod row;
use row::RapRow;

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Connection>()?;
    m.add_class::<Cursor>()?;
    m.add_class::<ExecuteContextManager>()?;
    m.add_class::<TransactionContextManager>()?;
    m.add_class::<RapRow>()?;

    // Register exception classes (required for create_exception! to be accessible from Python)
    m.add("Error", py.get_type::<Error>())?;
    m.add("Warning", py.get_type::<Warning>())?;
    m.add("DatabaseError", py.get_type::<DatabaseError>())?;
    m.add("OperationalError", py.get_type::<OperationalError>())?;
    m.add("ProgrammingError", py.get_type::<ProgrammingError>())?;
    m.add("IntegrityError", py.get_type::<IntegrityError>())?;
    m.add("ValueError", py.get_type::<ValueError>())?;

    Ok(())
}
