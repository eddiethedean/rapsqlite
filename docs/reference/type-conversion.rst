Type Conversion Strategy
========================

This document describes how rapsqlite maps Python and SQLite types, and how to
work with custom types. rapsqlite supports per-connection ``register_adapter``
and ``register_converter`` (sqlite3-style).

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

register_adapter and register_converter (supported)
---------------------------------------------------

rapsqlite supports per-connection type adapters and converters (sqlite3-style):

- **``conn.register_adapter(type, adapter)``** — When a Python object of that
  type is used as a parameter, call ``adapter(obj)`` and bind the result. Pass
  ``adapter=None`` to remove the adapter for that type.

- **``conn.register_converter(typename, converter)``** — When a result column's
  declared type (as reported by the driver) matches ``typename`` (case-insensitive),
  call ``converter(bytes)`` and return the result. Pass ``converter=None`` to
  remove. The converter receives the column value as bytes; for TEXT columns the
  value is UTF-8 encoded.

Example: custom type for parameters and results:

.. code-block:: python

   conn.register_adapter(Point, lambda p: f"{p.x},{p.y}")
   conn.register_converter("DATE", lambda b: b.decode("utf-8") if b else None)

Other options for custom types
------------------------------

You can also use these alternatives (or in combination with adapters/converters):

1. **Application-layer conversion**  
   Convert custom Python objects to a supported type before ``execute``, and
   convert result values after ``fetch_*``.

2. **``create_function``**  
   For custom SQL functions that accept or return values, use
   ``conn.create_function(...)``. This does not change how normal parameters
   or result columns are converted.

3. **``row_factory``**  
   Set ``conn.row_factory`` to a callable that receives the raw row and returns
   a transformed object (e.g. a named tuple or dataclass).

4. **``text_factory``**  
   Affects only TEXT columns: a callable ``(bytes) -> Any`` or ``str``.
