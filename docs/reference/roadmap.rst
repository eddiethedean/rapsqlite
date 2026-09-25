Roadmap
=======

This roadmap outlines the development plan for ``rapsqlite``.

Current Status
--------------

**Latest tag:** ``v0.3.3``
**Current phase:** ``0.4`` — Compatibility, security, and release stabilization
**Next phases:** ``0.5`` low-latency execution; ``0.6`` cache APIs and batching

The completed 0.1, 0.2, and 0.3 phases delivered the async core, aiosqlite-compatible API, callbacks, pooling, True Async DBAPI, SQLAlchemy/Alembic integration, query helpers, type adapters/converters, aggregates, collations, and operational metrics.

Phase 0.4 consolidates all work completed after ``v0.3.3``: parameter parsing and redaction fixes, shared-pool lifecycle fixes, cursor/row/SQLAlchemy compatibility, callback interrupt cleanup, CI/PyO3 hardening, Ruff tooling updates, SQLAlchemy 2.1 support, and the matched Redis comparison baseline.

Goal
----

Achieve drop-in replacement compatibility with ``aiosqlite`` to enable seamless migration with true async performance.

Release Phases
--------------

* **0.4** — Compatibility, security, and release stabilization; release all post-``v0.3.3`` work.
* **0.5** — Low-latency execution, session affinity, prepared queries, scalar/BLOB paths, and hot-path reductions. Issues #35–#39, #41, #45, and #46.
* **0.6** — Cache-specific APIs, bulk operations, and multiplexed concurrent reads. Issues #42–#44.
* **0.7** — Pooling, observability, reliability, stress testing, and platform validation.
* **0.8** — Type utilities, framework integrations, database tooling, and advanced SQLite helpers.
* **0.9** — Final stabilization toward a future ``1.0.0`` release.

For the complete roadmap, see the `ROADMAP.md <https://github.com/eddiethedean/rapsqlite/blob/main/docs/ROADMAP.md>`_ file in the repository (canonical source).
