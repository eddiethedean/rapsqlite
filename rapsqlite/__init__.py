"""True async SQLite — no fake async, no GIL stalls."""

try:
    from _rapsqlite import Connection
except ImportError:
    try:
        from rapsqlite._rapsqlite import Connection
    except ImportError:
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        )

__version__ = "0.0.1"
__all__ = ["Connection"]
