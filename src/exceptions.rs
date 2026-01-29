//! Python exception types exposed by the extension module.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::exceptions::PyValueError;

// Exception classes matching aiosqlite API (ABI3 compatible)
create_exception!(_rapsqlite, Error, PyException);
create_exception!(_rapsqlite, Warning, PyException);
create_exception!(_rapsqlite, DatabaseError, Error);
create_exception!(_rapsqlite, OperationalError, DatabaseError);
create_exception!(_rapsqlite, ProgrammingError, DatabaseError);
create_exception!(_rapsqlite, IntegrityError, DatabaseError);
create_exception!(_rapsqlite, ValueError, PyValueError);
