"""Compat for Connection conveniences: execute_fetchall, explain_query_plan, pool_health, executemany alias."""

from __future__ import annotations

from typing import Any, cast


async def _execute_fetchall(
    self: Any, sql: str, parameters: Any | None = None
) -> list[list[Any]]:
    return cast(list[list[Any]], await self.fetch_all(sql, parameters))


async def _explain_query_plan(
    self: Any, sql: str, parameters: Any | None = None
) -> list[list[Any]]:
    return cast(
        list[list[Any]],
        await self.fetch_all(f"EXPLAIN QUERY PLAN {sql}", parameters),
    )


async def _pool_health(self: Any) -> bool:
    await self.fetch_all("SELECT 1")
    return True


def apply_connection_conveniences_compat(Connection: type) -> None:
    """Compat for aiosqlite-style Connection conveniences."""
    Connection.execute_fetchall = _execute_fetchall  # type: ignore[attr-defined]
    Connection.explain_query_plan = _explain_query_plan  # type: ignore[attr-defined]
    Connection.pool_health = _pool_health  # type: ignore[attr-defined]
    Connection.executemany = Connection.execute_many  # type: ignore[attr-defined]
