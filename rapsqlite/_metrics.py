from __future__ import annotations

from typing import Any, TypedDict


class PoolMetricsGauges(TypedDict):
    """Return type for pool_metrics_gauges(); gauge names for Prometheus/custom metrics."""

    rapsqlite_pool_size: int
    rapsqlite_pool_num_idle: int
    rapsqlite_pool_in_use: int


async def pool_metrics_gauges(conn: Any) -> PoolMetricsGauges:
    """Return pool metrics as a dict of gauge names to values for Prometheus or custom metrics."""
    m = await conn.pool_metrics()
    return {
        "rapsqlite_pool_size": m.get("size", 0),
        "rapsqlite_pool_num_idle": m.get("num_idle", 0),
        "rapsqlite_pool_in_use": m.get("in_use", 0),
    }
