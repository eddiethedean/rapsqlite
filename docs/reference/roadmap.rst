Roadmap
=======

This roadmap outlines the development plan for ``rapsqlite``.

Current Status
--------------

**Current Version (v0.2.0)** — Phase 1 Complete, Phase 2 Complete:

**Phase 1 Complete:**
* ✅ Connection lifecycle management (async context managers)
* ✅ Transaction support (begin, commit, rollback)
* ✅ Type system improvements (proper Python types: int, float, str, bytes, None)
* ✅ Enhanced error handling (custom exception classes matching aiosqlite)
* ✅ API improvements (fetch_one, fetch_optional, execute_many, last_insert_rowid, changes)
* ✅ Cursor API (execute, executemany, fetchone, fetchall, fetchmany)
* ✅ aiosqlite compatibility (connect function, exception types)
* ✅ Connection pooling (basic implementation with reuse)
* ✅ Input validation and security improvements
* ✅ Type stubs for IDE support

**Phase 2 Complete:**
* ✅ Parameterized queries (named and positional parameters, execute_many with binding)
* ✅ Cursor improvements (fetchmany size-based slicing, result caching, state management)
* ✅ Connection configuration (PRAGMA settings, connection string parsing, constructor parameters)
* ✅ Pool configuration (pool_size and connection_timeout getters/setters)
* ✅ Row factory compatibility (dict/tuple/callable support)
* ✅ Transaction context managers
* ✅ Advanced SQLite callbacks (enable_load_extension, set_progress_handler, create_function, set_trace_callback, set_authorizer)
* ✅ Database dump (iterdump)
* ✅ Database backup (backup)
* ✅ Schema operations and introspection (9 methods)
* ✅ Database initialization hooks (init_hook parameter)
* ✅ Prepared statements & performance optimization
* ✅ Drop-in replacement validation (aiosqlite compatibility features)
* ✅ Documentation & benchmarking

Goal
----

Achieve drop-in replacement compatibility with ``aiosqlite`` to enable seamless migration with true async performance.

Phase 3 (Future)
----------------

Future enhancements may include:

* Advanced connection lifecycle management
* Nested transaction handling (savepoints)
* Enhanced error recovery strategies
* Query optimization utilities
* Ecosystem integration improvements

For the complete roadmap, see the `ROADMAP.md <https://github.com/eddiethedean/rapsqlite/blob/main/docs/ROADMAP.md>`_ file in the repository.
