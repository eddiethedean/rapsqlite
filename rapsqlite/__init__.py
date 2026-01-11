"""True async SQLite — no fake async, no GIL stalls."""

from typing import List

try:
    from _rapsqlite import Connection  # type: ignore[import-not-found]
except ImportError:
    try:
        from rapsqlite._rapsqlite import Connection
    except ImportError:
        raise ImportError(
            "Could not import _rapsqlite. Make sure rapsqlite is built with maturin."
        )

__version__: str = "0.0.2"
__all__: List[str] = ["Connection"]
