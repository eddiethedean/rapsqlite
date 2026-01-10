"""True async SQLite — no fake async, no GIL stalls."""

from rapsqlite._rapsqlite import Connection

__version__ = "0.0.1"
__all__ = ["Connection"]

