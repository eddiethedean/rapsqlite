Type Conversion Strategy
========================

This document describes how rapsqlite maps Python and SQLite types, and how to
work with custom types today. Full sqlite3-style ``register_adapter`` and
``register_converter`` are planned but not yet implemented.

Built-in type mapping
---------------------

**Parameters (Python → SQLite)**  
Supported Python types when binding parameters: ``int``, ``float``, ``str``,
``bytes``, ``None``. These map to SQLite INTEGER, REAL, TEXT, BLOB, and NULL.
Other types raise ``TypeError`` unless you convert them before passing (see below).

**Results (SQLite → Python)**  
Result columns are decoded by declared type when available: INTEGER/INT → ``int``,
REAL/FLOAT/DOUBLE → ``float``, TEXT/VARCHAR/CHAR → ``str``, BLOB → ``bytes``,
NULL → ``None``. For dynamic or unknown types, rapsqlite probes the value and
returns the appropriate Python type. ``text_factory`` (e.g. ``str`` or a callable)
applies to TEXT columns and can change how text is produced (e.g. return bytes).

Custom types today
------------------

Without ``register_adapter`` / ``register_converter``, use one of these approaches:

1. **Application-layer conversion (recommended)**  
   Convert custom Python objects to a supported type before ``execute``, and
   convert result values after ``fetch_*``. Example: serialize a dataclass to
   JSON (str) when inserting, and parse JSON back when reading.

2. **``create_function``**  
   For custom SQL functions that accept or return values, use
   ``conn.create_function(...)``. The callback receives Python values and
   returns Python values; rapsqlite converts to/from SQLite inside the callback
   layer. This does not change how normal parameters or result columns are
   converted.

3. **``row_factory``**  
   Set ``conn.row_factory`` to a callable that receives the raw row (list of
   values) and returns a transformed object (e.g. a named tuple or dataclass).
   You can convert individual column values inside that callable.

4. **``text_factory``**  
   Affects only TEXT columns: a callable ``(bytes) -> Any`` or ``str``. Use it
   to return something other than ``str`` for text (e.g. ``bytes``).

Future: register_adapter and register_converter
-----------------------------------------------

The sqlite3 module supports:

- ``sqlite3.register_adapter(type, adapter)`` — when a Python object of that
  type is used as a parameter, call ``adapter(obj)`` and bind the result.
- ``sqlite3.register_converter(typename, converter)`` — when a result column
  has that declared type name, call ``converter(bytes)`` and return the result.

rapsqlite does **not** yet provide these. Binding and decoding are implemented
in Rust (via ``SqliteParam::from_py`` and ``sqlite_value_to_py``); adding
adapter/converter support would require a registry (global or per-connection)
and hooking it into the parameter and row conversion paths. This is planned for
a future release (see :doc:`../ROADMAP` Phase 3.10).

Until then, use application-layer conversion, ``create_function``, ``row_factory``,
or ``text_factory`` as above.
