//! Error mapping helpers (sqlx -> Python exceptions).

use pyo3::prelude::*;

use crate::exceptions::{DatabaseError, IntegrityError, OperationalError, ProgrammingError};

/// Map sqlx error to appropriate Python exception.
pub(crate) fn map_sqlx_error(e: sqlx::Error, path: &str, query: &str) -> PyErr {
    use sqlx::Error as SqlxError;

    let error_msg = format!("Failed to execute query on database {path}: {e}\nQuery: {query}");

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
