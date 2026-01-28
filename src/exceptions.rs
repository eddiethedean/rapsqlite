//! Python exception types exposed by the extension module.

use pyo3::create_exception;
use pyo3::exceptions::PyException;

// Exception classes matching aiosqlite API (ABI3 compatible)
create_exception!(_rapsqlite, Error, PyException);
create_exception!(_rapsqlite, Warning, PyException);
create_exception!(_rapsqlite, DatabaseError, PyException);
create_exception!(_rapsqlite, OperationalError, PyException);
create_exception!(_rapsqlite, ProgrammingError, PyException);
create_exception!(_rapsqlite, IntegrityError, PyException);


