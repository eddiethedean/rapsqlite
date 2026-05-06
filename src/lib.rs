#![allow(non_local_definitions)] // False positive from pyo3 macros

mod connection;
pub(crate) use connection::Connection;

mod context_managers;
pub(crate) use context_managers::{
    ExecuteContextManager, SavepointContextManager, TransactionContextManager,
};

mod cursor;
pub(crate) use cursor::Cursor;

use pyo3::prelude::*;

mod exceptions;
use exceptions::{
    DataError, DatabaseError, Error, IntegrityError, InterfaceError, InternalError,
    NotSupportedError, OperationalError, ProgrammingError, ValueError, Warning,
};

mod types;

mod utils;

mod conversion;

#[macro_use]
mod parameters;

mod query;

mod pool;

mod batch;

mod errors;
pub(crate) use errors::map_sqlx_error;

mod row;
use row::RapRow;

/// Python bindings for rapsqlite - True async SQLite.
#[pymodule]
fn _rapsqlite(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Callbacks/UDFs can be invoked by SQLite on threads which didn't originate in Python.
    // Preparing the interpreter for multi-threaded access ensures PyO3 can safely acquire the GIL
    // from those threads (e.g. in sqlite callback destructors).
    #[allow(deprecated)]
    pyo3::prepare_freethreaded_python();

    m.add_class::<Connection>()?;
    m.add_class::<Cursor>()?;
    m.add_class::<ExecuteContextManager>()?;
    m.add_class::<TransactionContextManager>()?;
    m.add_class::<SavepointContextManager>()?;
    m.add_class::<RapRow>()?;

    // Register exception classes (required for create_exception! to be accessible from Python)
    m.add("Error", py.get_type::<Error>())?;
    m.add("Warning", py.get_type::<Warning>())?;
    m.add("InterfaceError", py.get_type::<InterfaceError>())?;
    m.add("DatabaseError", py.get_type::<DatabaseError>())?;
    m.add("DataError", py.get_type::<DataError>())?;
    m.add("OperationalError", py.get_type::<OperationalError>())?;
    m.add("IntegrityError", py.get_type::<IntegrityError>())?;
    m.add("InternalError", py.get_type::<InternalError>())?;
    m.add("ProgrammingError", py.get_type::<ProgrammingError>())?;
    m.add("NotSupportedError", py.get_type::<NotSupportedError>())?;
    m.add("ValueError", py.get_type::<ValueError>())?;

    Ok(())
}
