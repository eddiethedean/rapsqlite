"""Helper module for extracting sqlite3* handle from sqlite3.Connection objects.

This module provides a safe way to access the internal database handle
from Python's sqlite3.Connection objects for use in backup operations.
"""

import ctypes
import sys
from typing import Optional


def get_sqlite3_handle(conn) -> Optional[int]:
    """
    Extract sqlite3* handle from sqlite3.Connection object.

    This function uses ctypes to safely access the internal pysqlite_Connection
    struct and extract the sqlite3* database handle pointer.

    Args:
        conn: A sqlite3.Connection object

    Returns:
        The sqlite3* pointer as an integer (usize), or None if extraction fails.
    """
    try:
        # Verify it's a sqlite3.Connection
        if not hasattr(conn, "__class__"):
            return None

        class_name = conn.__class__.__name__
        module_name = conn.__class__.__module__

        # Should be _sqlite3.Connection or sqlite3.Connection
        if "sqlite3" not in module_name or "Connection" not in class_name:
            return None

        # Verify connection is not closed
        # Closed connections may have None or invalid handles
        try:
            # Try to access a property that requires an open connection
            # This will raise an error if connection is closed
            _ = conn.total_changes
        except (AttributeError, ValueError, Exception):
            # Connection might be closed or invalid
            # Catch all exceptions to handle ProgrammingError for closed connections
            return None

        # Determine pointer size based on platform
        if sys.maxsize > 2**32:  # 64-bit
            Py_ssize_t = ctypes.c_int64
        else:  # 32-bit
            Py_ssize_t = ctypes.c_int32  # type: ignore[assignment]

        # Define PyObject_HEAD structure
        class PyObject_HEAD(ctypes.Structure):
            _fields_ = [
                ("ob_refcnt", Py_ssize_t),  # type: ignore[misc]
                ("ob_type", ctypes.c_void_p),  # PyTypeObject*
            ]

        # Define pysqlite_Connection structure (only what we need)
        class pysqlite_Connection(ctypes.Structure):
            _fields_ = [
                ("ob_base", PyObject_HEAD),
                ("db", ctypes.c_void_p),  # sqlite3* - this is what we need!
            ]

        # In CPython, id(obj) returns the memory address of the PyObject
        # We can cast this to our struct
        obj_addr = id(conn)

        # Cast the address to our struct pointer
        # This is unsafe but necessary to access internal fields
        conn_struct = ctypes.cast(
            obj_addr, ctypes.POINTER(pysqlite_Connection)
        ).contents

        # Extract the db field
        handle_ptr = conn_struct.db

        # Verify the pointer is not null
        if handle_ptr is None or handle_ptr == 0:
            return None

        # Return as integer (usize)
        # handle_ptr is c_void_p, convert to int
        if handle_ptr is None:
            return None
        return int(ctypes.cast(handle_ptr, ctypes.c_void_p).value)  # type: ignore[arg-type]

    except (AttributeError, TypeError, ValueError, OSError, ctypes.ArgumentError):
        # Silently fail - we'll handle the error in Rust
        return None


def is_sqlite3_connection(obj) -> bool:
    """
    Check if an object is a sqlite3.Connection.

    Args:
        obj: Object to check

    Returns:
        True if the object is a sqlite3.Connection, False otherwise
    """
    try:
        class_name = obj.__class__.__name__
        module_name = obj.__class__.__module__
        return "sqlite3" in module_name and "Connection" in class_name
    except AttributeError:
        return False
