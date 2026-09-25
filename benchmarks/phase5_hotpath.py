"""Measure the Phase 0.5 scalar and raw-cache paths through real client APIs.

This is intentionally a diagnostic benchmark, not a claim that the backends are
matched across machines.  It reports sequential latency and a small concurrent
workload separately.  Redis is skipped when the optional client or local server
is unavailable.

Example::

    python benchmarks/phase5_hotpath.py --ops 20000 --json-out phase5.json
"""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import sqlite3
import statistics
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Awaitable, Callable, cast

try:
    import resource
except ImportError:  # pragma: no cover - Windows does not provide resource
    resource = None

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

import rapsqlite  # noqa: E402

try:  # Redis is optional for local development and offline CI.
    import redis.asyncio as redis  # type: ignore[import-not-found]
except ImportError:  # pragma: no cover - depends on the environment
    redis = None


READ_SQL = (
    "SELECT value FROM cache WHERE key = ? AND (expires_at IS NULL OR expires_at > ?)"
)
WRITE_SQL = (
    "INSERT INTO cache (key, value, expires_at) VALUES (?, ?, ?) "
    "ON CONFLICT(key) DO UPDATE SET "
    "value = excluded.value, expires_at = excluded.expires_at"
)
SCHEMA_SQL = (
    "CREATE TABLE cache ("
    "key TEXT PRIMARY KEY NOT NULL, "
    "value BLOB NOT NULL, "
    "expires_at REAL"
    ") WITHOUT ROWID"
)
EXPIRATION_INDEX_SQL = (
    "CREATE INDEX cache_expires_idx ON cache (expires_at) WHERE expires_at IS NOT NULL"
)
CACHE_KEY = "hot-key"
CACHE_VALUE = b"x" * 1024


def percentile(samples: list[float], p: float) -> float:
    if not samples:
        return 0.0
    values = sorted(samples)
    rank = (len(values) - 1) * p
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return values[lower]
    return values[lower] + (values[upper] - values[lower]) * (rank - lower)


def summarize(samples: list[float], elapsed: float) -> dict[str, float]:
    return {
        "mean_us": statistics.mean(samples) if samples else 0.0,
        "p50_us": percentile(samples, 0.50),
        "p95_us": percentile(samples, 0.95),
        "p99_us": percentile(samples, 0.99),
        "ops_per_second": len(samples) / elapsed if elapsed else 0.0,
        "elapsed_seconds": elapsed,
    }


async def measure_sequential(
    operation: Callable[[], Awaitable[Any]], ops: int, runs: int
) -> dict[str, Any]:
    run_results: list[dict[str, float]] = []
    for _ in range(runs):
        for _ in range(min(100, ops)):
            await operation()
        samples: list[float] = []
        started = time.perf_counter()
        for _ in range(ops):
            op_started = time.perf_counter_ns()
            await operation()
            samples.append((time.perf_counter_ns() - op_started) / 1_000.0)
        run_results.append(summarize(samples, time.perf_counter() - started))
    return {
        "runs": run_results,
        "median_run_mean_us": statistics.median(r["mean_us"] for r in run_results),
        "median_run_p50_us": statistics.median(r["p50_us"] for r in run_results),
        "median_run_p95_us": statistics.median(r["p95_us"] for r in run_results),
        "median_run_p99_us": statistics.median(r["p99_us"] for r in run_results),
        "mean_ops_per_second": statistics.mean(
            r["ops_per_second"] for r in run_results
        ),
    }


async def measure_concurrent(
    operation: Callable[[], Awaitable[Any]], ops: int, concurrency: int
) -> dict[str, Any]:
    per_worker = ops // concurrency
    remainder = ops % concurrency
    latencies: list[float] = []
    errors: list[str] = []

    async def worker(count: int) -> None:
        for _ in range(count):
            started = time.perf_counter_ns()
            try:
                await operation()
            except Exception as exc:  # measured as part of the result
                errors.append(type(exc).__name__)
            latencies.append((time.perf_counter_ns() - started) / 1_000.0)

    async def ticker(stop: asyncio.Event, delays: list[float]) -> None:
        loop = asyncio.get_running_loop()
        next_tick = loop.time() + 0.001
        while not stop.is_set():
            await asyncio.sleep(max(0.0, next_tick - loop.time()))
            delays.append(max(0.0, loop.time() - next_tick) * 1_000_000.0)
            next_tick += 0.001

    delays: list[float] = []
    stop = asyncio.Event()
    ticker_task = asyncio.create_task(ticker(stop, delays))
    started = time.perf_counter()
    await asyncio.gather(
        *[
            worker(per_worker + int(worker_id < remainder))
            for worker_id in range(concurrency)
        ]
    )
    elapsed = time.perf_counter() - started
    stop.set()
    await ticker_task
    return {
        **summarize(latencies, elapsed),
        "total_ops": ops,
        "errors": len(errors),
        "error_types": {name: errors.count(name) for name in sorted(set(errors))},
        "concurrency": concurrency,
        "event_loop_delay_p95_us": percentile(delays, 0.95),
        "event_loop_delay_max_us": max(delays, default=0.0),
    }


async def setup_sqlite(session_affinity: bool) -> Any:
    conn = rapsqlite.connect_memory(
        name=f"phase5-{uuid.uuid4().hex}",
        pool_size=1,
        session_affinity=session_affinity,
    )
    await conn.__aenter__()
    await conn.execute(SCHEMA_SQL)
    await conn.execute(EXPIRATION_INDEX_SQL)
    await conn.execute(
        "INSERT INTO cache VALUES (?, ?, ?)",
        ["hot-key", b"x" * 1024, time.time() + 3600],
    )
    return conn


async def setup_redis(host: str, port: int) -> Any | None:
    if redis is None:
        return None
    client: Any = redis.Redis(host=host, port=port, decode_responses=False)
    try:
        await cast(Awaitable[Any], client.ping())
        await client.set("phase5-hot-key", b"x" * 1024, ex=3600)
    except Exception:
        await client.aclose()
        return None
    return client


async def close_handle(handle: Any) -> None:
    if hasattr(handle, "aclose"):
        await handle.aclose()
    else:
        await handle.__aexit__(None, None, None)


def sqlite3_baseline(ops: int, runs: int) -> dict[str, Any]:
    conn = sqlite3.connect(":memory:")
    conn.execute(SCHEMA_SQL)
    conn.execute(EXPIRATION_INDEX_SQL)
    conn.execute(
        "INSERT INTO cache VALUES (?, ?, ?)",
        (CACHE_KEY, CACHE_VALUE, time.time() + 3600),
    )
    run_results: list[dict[str, float]] = []
    for _ in range(runs):
        samples: list[float] = []
        started = time.perf_counter()
        for _ in range(ops):
            op_started = time.perf_counter_ns()
            conn.execute(READ_SQL, (CACHE_KEY, time.time())).fetchone()
            samples.append((time.perf_counter_ns() - op_started) / 1_000.0)
        run_results.append(summarize(samples, time.perf_counter() - started))
    conn.close()
    return {
        "runs": run_results,
        "median_run_mean_us": statistics.median(r["mean_us"] for r in run_results),
        "median_run_p50_us": statistics.median(r["p50_us"] for r in run_results),
        "median_run_p95_us": statistics.median(r["p95_us"] for r in run_results),
        "median_run_p99_us": statistics.median(r["p99_us"] for r in run_results),
        "mean_ops_per_second": statistics.mean(
            r["ops_per_second"] for r in run_results
        ),
    }


async def main(args: argparse.Namespace) -> dict[str, Any]:
    results: list[dict[str, Any]] = []
    for affinity in (False, True):
        conn = await setup_sqlite(affinity)
        cache = rapsqlite.SQLiteCache(conn, table_name="cache")
        await cache.set(CACHE_KEY, CACHE_VALUE, ttl=3600)
        prepared = conn.prepare(READ_SQL)
        raw_prepared = conn.prepare(READ_SQL, raw=True, blob=True)
        operations = {
            "fetch_one": lambda: conn.fetch_one(READ_SQL, [CACHE_KEY, time.time()]),
            "fetch_scalar": lambda: conn.fetch_scalar(
                READ_SQL, [CACHE_KEY, time.time()]
            ),
            "fetch_blob": lambda: conn.fetch_blob(READ_SQL, [CACHE_KEY, time.time()]),
            "raw_fetch_scalar": lambda: conn.raw_fetch_scalar(
                READ_SQL, [CACHE_KEY, time.time()], True
            ),
            "prepared_scalar": lambda: prepared.fetch_scalar([CACHE_KEY, time.time()]),
            "prepared_raw_blob": lambda: raw_prepared.fetch_blob(
                [CACHE_KEY, time.time()]
            ),
            "sqlite_cache_get": lambda: cache.get(CACHE_KEY),
        }

        async def generic_set() -> None:
            cursor = await conn.execute(
                WRITE_SQL, [CACHE_KEY, CACHE_VALUE, time.time() + 3600]
            )
            await cursor.close()

        write_operations = {
            "execute_upsert": generic_set,
            "sqlite_cache_set": lambda: cache.set(CACHE_KEY, CACHE_VALUE, ttl=3600),
        }
        try:
            for name, operation in operations.items():
                row = await measure_sequential(operation, args.ops, args.runs)
                results.append(
                    {
                        "backend": "rapsqlite",
                        "variant": name,
                        "session_affinity": affinity,
                        "workload": "sequential",
                        **row,
                    }
                )
                concurrent = await measure_concurrent(
                    operation, args.concurrent_ops, args.concurrency
                )
                results.append(
                    {
                        "backend": "rapsqlite",
                        "variant": name,
                        "session_affinity": affinity,
                        "workload": "concurrent",
                        **concurrent,
                    }
                )
            for name, operation in write_operations.items():
                row = await measure_sequential(operation, args.ops, args.runs)
                results.append(
                    {
                        "backend": "rapsqlite",
                        "variant": name,
                        "session_affinity": affinity,
                        "workload": "sequential_write",
                        **row,
                    }
                )
                concurrent = await measure_concurrent(
                    operation, args.concurrent_ops, args.concurrency
                )
                results.append(
                    {
                        "backend": "rapsqlite",
                        "variant": name,
                        "session_affinity": affinity,
                        "workload": "concurrent_write",
                        **concurrent,
                    }
                )
        finally:
            await close_handle(conn)

    results.append(
        {
            "backend": "sqlite3",
            "variant": "synchronous_fetch_one",
            "workload": "sequential",
            **sqlite3_baseline(args.ops, args.runs),
        }
    )

    redis_client = await setup_redis(args.host, args.port)
    redis_version = None
    if redis_client is not None:
        try:
            redis_version = (await redis_client.info("server")).get("redis_version")
            redis_result = await measure_sequential(
                lambda: redis_client.get("phase5-hot-key"), args.ops, args.runs
            )
            results.append(
                {
                    "backend": "redis.asyncio",
                    "variant": "get",
                    "workload": "sequential",
                    **redis_result,
                }
            )
            redis_set_result = await measure_sequential(
                lambda: redis_client.set("phase5-hot-key", CACHE_VALUE, ex=3600),
                args.ops,
                args.runs,
            )
            results.append(
                {
                    "backend": "redis.asyncio",
                    "variant": "set_with_ttl",
                    "workload": "sequential_write",
                    **redis_set_result,
                }
            )
            redis_concurrent = await measure_concurrent(
                lambda: redis_client.get("phase5-hot-key"),
                args.concurrent_ops,
                args.concurrency,
            )
            results.append(
                {
                    "backend": "redis.asyncio",
                    "variant": "get",
                    "workload": "concurrent",
                    **redis_concurrent,
                }
            )
            redis_concurrent_set = await measure_concurrent(
                lambda: redis_client.set("phase5-hot-key", CACHE_VALUE, ex=3600),
                args.concurrent_ops,
                args.concurrency,
            )
            results.append(
                {
                    "backend": "redis.asyncio",
                    "variant": "set_with_ttl",
                    "workload": "concurrent_write",
                    **redis_concurrent_set,
                }
            )
        finally:
            await redis_client.aclose()

    metadata = {
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "python": sys.version,
        "platform": sys.platform,
        "rapsqlite": rapsqlite.__version__,
        "sqlite3": sqlite3.sqlite_version,
        "redis_version": redis_version,
        "ops": args.ops,
        "runs": args.runs,
        "concurrent_ops": args.concurrent_ops,
        "concurrency": args.concurrency,
        # ru_maxrss is bytes on macOS and KiB on Linux.
        "max_rss_kib": (
            None
            if resource is None
            else resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
            / (1024 if sys.platform == "darwin" else 1)
        ),
    }
    payload = {"metadata": metadata, "results": results}
    print(
        "backend                 variant              workload      mean/p50/p95/p99 µs      ops/s"
    )
    for row in results:
        if row["workload"] in {"sequential", "sequential_write"}:
            print(
                f"{row['backend']:<23} {row['variant']:<20} {row['workload']:<12} "
                f"{row['median_run_mean_us']:>7.2f}/{row['median_run_p50_us']:>7.2f}/"
                f"{row['median_run_p95_us']:>7.2f}/{row['median_run_p99_us']:>7.2f} "
                f"{row['mean_ops_per_second']:>10,.0f}"
            )
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(payload, indent=2) + "\n")
    return payload


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=6380)
    parser.add_argument("--ops", type=int, default=10_000)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--concurrent-ops", type=int, default=5_000)
    parser.add_argument("--concurrency", type=int, default=16)
    parser.add_argument("--json-out", type=Path)
    return parser.parse_args()


if __name__ == "__main__":
    asyncio.run(main(parse_args()))
