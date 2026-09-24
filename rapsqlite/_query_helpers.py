from __future__ import annotations

import builtins as _builtins
import re
import time
from collections.abc import Callable
from typing import Any, cast

try:  # pragma: no cover - mirrors __init__ fallback logic
    import _rapsqlite as _ext
except ImportError:  # pragma: no cover
    try:
        from rapsqlite import _rapsqlite as _ext
    except ImportError:  # pragma: no cover
        _ext = None


if getattr(_ext, "ValueError", None) is not None:
    ValueError = _ext.ValueError
else:
    ValueError = _builtins.ValueError


def _normalize_parameters(parameters: Any | None) -> list[Any]:
    """Treat only lists and tuples as positional parameter sequences."""
    if parameters is None:
        return []
    if isinstance(parameters, (list, tuple)):
        return list(parameters)
    return [parameters]


_IN_PLACEHOLDER_RE = re.compile(r"\bIN\s*\(\s*\?\s*\)", re.IGNORECASE)


def _sql_code_mask(sql: str) -> list[bool]:
    """Mark SQL code characters, excluding quoted strings/identifiers and comments."""
    code = [True] * len(sql)
    i = 0
    while i < len(sql):
        char = sql[i]
        if char in ("'", '"', "`", "["):
            closing = "]" if char == "[" else char
            code[i] = False
            i += 1
            while i < len(sql):
                code[i] = False
                if sql[i] == closing:
                    if closing != "]" and i + 1 < len(sql) and sql[i + 1] == closing:
                        code[i + 1] = False
                        i += 2
                        continue
                    i += 1
                    break
                i += 1
            continue
        if sql.startswith("--", i):
            end = sql.find("\n", i + 2)
            end = len(sql) if end < 0 else end
            code[i:end] = [False] * (end - i)
            i = end
            continue
        if sql.startswith("/*", i):
            end = sql.find("*/", i + 2)
            end = len(sql) if end < 0 else end + 2
            code[i:end] = [False] * (end - i)
            i = end
            continue
        i += 1
    return code


def _full_scan_targets(details: list[str]) -> list[str]:
    """Extract table names from SQLite's full-scan EXPLAIN QUERY PLAN details."""
    targets: list[str] = []
    for detail in details:
        match = re.match(r"^\s*SCAN\s+(?:TABLE\s+)?(.+?)\s*$", detail, re.IGNORECASE)
        if not match:
            continue
        target = match.group(1).split(None, 1)[0]
        if target.upper() in {"CONSTANT", "SUBQUERY", "MATERIALIZE", "CO-ROUTINE"}:
            continue
        targets.append(target)
    return targets


async def timed_fetch_all(
    conn: Any,
    sql: str,
    parameters: Any | None = None,
    on_timing: Callable[[float, str], None] | None = None,
) -> list[list[Any]] | tuple[list[list[Any]], float]:
    """Run fetch_all and record duration; optionally call on_timing(duration_secs, sql).

    If on_timing is None, returns (rows, duration_secs). If on_timing is provided,
    calls on_timing(duration_secs, sql) and returns rows only.
    """
    t0 = time.perf_counter()
    rows = await conn.fetch_all(sql, parameters)
    duration = time.perf_counter() - t0
    if on_timing is not None:
        on_timing(duration, sql)
        return cast(list[list[Any]], rows)
    return cast(tuple[list[list[Any]], float], (rows, duration))


def execute_iter(
    conn: Any,
    sql: str,
    parameters: Any | None = None,
    chunk_size: int | None = None,
) -> _StreamChunksIterator:
    """Return an async iterator that yields rows in chunks (streaming / memory-efficient).

    Uses LIMIT/OFFSET under the hood so memory stays bounded by chunk_size.
    Single connection is used for the duration of iteration; closing the
    connection or cancelling the task stops iteration.
    """
    return _StreamChunksIterator(conn, sql, parameters, chunk_size)


async def paginate(
    conn: Any,
    sql: str,
    parameters: Any | None = None,
    page_size: int = 64,
    offset: int = 0,
) -> list[list[Any]]:
    """Fetch one page of rows from a SELECT query.

    Uses LIMIT/OFFSET under the hood. For multiple pages, call with
    incrementing offset: paginate(conn, sql, params, 100, 0), then
    paginate(conn, sql, params, 100, 100), etc.
    """
    sql_clean = sql.strip().rstrip(";")
    wrapped = f"SELECT * FROM ({sql_clean}) LIMIT ? OFFSET ?"
    if page_size <= 0:
        raise ValueError("page_size must be greater than zero")
    params = _normalize_parameters(parameters)
    rows = await conn.fetch_all(wrapped, params + [page_size, offset])
    return cast(list[list[Any]], rows)


async def analyze_query_plan(
    conn: Any,
    sql: str,
    parameters: Any | None = None,
) -> dict[str, Any]:
    """Run EXPLAIN QUERY PLAN and return structured analysis."""
    rows = await conn.explain_query_plan(sql, parameters)
    details: list[str] = []
    for row in rows:
        if isinstance(row, (list, tuple)) and len(row) >= 4:
            details.append(str(row[3]))
        elif isinstance(row, dict) and "detail" in row:
            details.append(str(row["detail"]))
        else:
            details.append(str(row))
    detail_str = " ".join(details).upper()
    full_scan_targets = _full_scan_targets(details)
    return {
        "rows": rows,
        "details": details,
        "uses_index": "USING INDEX" in detail_str or "INDEX" in detail_str,
        "table_scan": bool(full_scan_targets) or "TABLE SCAN" in detail_str,
    }


async def suggest_indexes(
    conn: Any,
    sql: str,
    parameters: Any | None = None,
) -> list[dict[str, Any]]:
    """Suggest indexes when query plan indicates a full table scan."""
    analysis = await analyze_query_plan(conn, sql, parameters)
    if not analysis.get("table_scan") or analysis.get("uses_index"):
        return []

    suggestions: list[dict[str, Any]] = []
    seen_tables: set[str] = set()

    for table in _full_scan_targets(
        [str(detail) for detail in analysis.get("details", [])]
    ):
        if table.casefold() in seen_tables:
            continue
        seen_tables.add(table.casefold())
        suggestions.append(
            {
                "table": table,
                "column": "",
                "suggestion": (
                    f"CREATE INDEX idx_{table}_<columns> ON {table}(<columns>) "
                    "-- add columns used in WHERE, ORDER BY, or JOIN"
                ),
            }
        )

    return suggestions


def in_clause_query(
    sql: str,
    values: list[Any] | tuple[Any, ...],
) -> tuple[str, list[Any]]:
    """Expand IN (?) to IN (?,?,...) for use with fetch_all."""
    if len(values) == 0:
        raise ValueError(
            "in_clause_query requires at least one value; IN () is invalid in SQLite"
        )
    placeholders = ",".join("?" * len(values))
    code = _sql_code_mask(sql)
    match = next(
        (m for m in _IN_PLACEHOLDER_RE.finditer(sql) if all(code[m.start() : m.end()])),
        None,
    )
    if match is None:
        raise ValueError(
            "in_clause_query: sql must contain 'IN (?)' placeholder; found no match"
        )
    new_sql = sql[: match.start()] + f"IN ({placeholders})" + sql[match.end() :]
    return (new_sql, list(values))


def rows_to_dicts(
    rows: list[Any],
    columns: list[str] | tuple[str, ...] | None = None,
) -> list[dict[str, Any]]:
    """Convert rows (list of list/tuple) to list of dicts using column names."""
    if columns is None or len(columns) == 0:
        return []
    col_list = list(columns)
    result: list[dict[str, Any]] = []
    for row in rows:
        if hasattr(row, "keys") and callable(row.keys):
            result.append(dict(row))
        else:
            row_iter = row if isinstance(row, (list, tuple)) else list(row)
            result.append(dict(zip(col_list, row_iter)))
    return result


class _StreamChunksIterator:
    """Async iterator yielding chunks of rows from a SELECT (LIMIT/OFFSET under the hood)."""

    def __init__(
        self,
        conn: Any,
        sql: str,
        parameters: Any | None = None,
        chunk_size: int | None = None,
    ) -> None:
        self._conn = conn
        self._sql = sql.strip().rstrip(";")
        self._params = _normalize_parameters(parameters)
        try:
            default_chunk = getattr(conn, "iter_chunk_size", 64)
            default_chunk = int(default_chunk) if default_chunk is not None else 64
        except (TypeError, ValueError):
            default_chunk = 64
        self._chunk_size = int(chunk_size) if chunk_size is not None else default_chunk
        if self._chunk_size <= 0:
            raise ValueError("chunk_size must be greater than zero")
        self._offset = 0

    def __aiter__(self) -> _StreamChunksIterator:
        return self

    async def __anext__(self) -> list[list[Any]]:
        # Wrap query so we can paginate: SELECT * FROM (user_query) LIMIT ? OFFSET ?
        wrapped = f"SELECT * FROM ({self._sql}) LIMIT ? OFFSET ?"
        params = self._params + [self._chunk_size, self._offset]
        rows = await self._conn.fetch_all(wrapped, params)
        if not rows:
            raise StopAsyncIteration
        self._offset += len(rows)
        return cast(list[list[Any]], rows)
