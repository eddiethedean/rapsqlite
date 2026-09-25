"""Matched rapsqlite versus redis.asyncio cache benchmark.

This benchmark intentionally measures the real async client APIs, not raw SQLite
or Redis command-line throughput.  It uses the same 100,000-key dataset, 1 KiB
values, expiration-aware reads, existing-key updates, and separately reported
single-operation and batched workloads.

Example::

    python benchmarks/redis_comparison.py --port 6380

The Redis server is expected to be local.  Setup time is excluded from workload
timings.  The default run is deliberately moderate; use ``--keys``, ``--ops``,
``--runs``, and ``--concurrency-levels`` to scale it up or down.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import statistics
import sqlite3
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Awaitable, Callable, Iterable

import redis as redis_package
import redis.asyncio as redis

# When invoked as ``python benchmarks/redis_comparison.py``, Python puts the
# benchmarks directory—not the repository root—first on sys.path.  Prefer the
# checkout so the benchmark measures the code being developed here.
REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

import rapsqlite  # noqa: E402


READ_SQL = """
SELECT value
FROM cache
WHERE key = ? AND expires_at > ?
"""
UPSERT_SQL = """
INSERT INTO cache(key, value, expires_at)
VALUES (?, ?, ?)
ON CONFLICT(key) DO UPDATE SET
    value = excluded.value,
    expires_at = excluded.expires_at
"""
SCHEMA_SQL = """
CREATE TABLE cache(
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    expires_at REAL NOT NULL
)
"""
EXPIRATION_INDEX_SQL = "CREATE INDEX cache_expires_at_idx ON cache(expires_at)"


@dataclass(frozen=True)
class Config:
    host: str
    port: int
    keys: int
    value_size: int
    runs: int
    sequential_ops: int
    concurrent_ops: int
    batch_size: int
    concurrency_levels: tuple[int, ...]
    pool_sizes: tuple[int, ...]
    sqlite_timeout: float
    expiration_seconds: int
    json_out: Path | None


def percentile(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    rank = (len(ordered) - 1) * p
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[lower]
    fraction = rank - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * fraction


def latency_summary(samples_us: list[float]) -> dict[str, float]:
    return {
        "mean_us": statistics.mean(samples_us) if samples_us else 0.0,
        "p50_us": percentile(samples_us, 0.50),
        "p95_us": percentile(samples_us, 0.95),
        "p99_us": percentile(samples_us, 0.99),
    }


def summarize_sequential(runs: list[dict[str, float]]) -> dict[str, Any]:
    """Summarize each run's latency distribution without pretending it is p50/p99."""
    return {
        "runs": runs,
        "median_run_mean_us": statistics.median(r["mean_us"] for r in runs),
        "median_run_p50_us": statistics.median(r["p50_us"] for r in runs),
        "median_run_p95_us": statistics.median(r["p95_us"] for r in runs),
        "median_run_p99_us": statistics.median(r["p99_us"] for r in runs),
        "mean_ops_per_second": statistics.mean(r["ops_per_second"] for r in runs),
    }


def operation_keys(keys: list[str], count: int, *, offset: int = 0) -> list[str]:
    # A deterministic stride avoids timing Python's random-number generator and
    # still visits a broad part of the keyspace rather than one hot key.
    step = 7919
    return [keys[(offset + i * step) % len(keys)] for i in range(count)]


def chunked(items: list[Any], size: int) -> Iterable[list[Any]]:
    for start in range(0, len(items), size):
        yield items[start : start + size]


async def measure_sequential(
    operation: Callable[[str], Awaitable[Any]],
    keys: list[str],
    value: bytes,
    *,
    ops: int,
    runs: int,
    write: bool,
) -> dict[str, Any]:
    del value, write  # Kept in the signature so read/write call sites are explicit.
    scheduled_keys = operation_keys(keys, ops)
    run_results: list[dict[str, float]] = []
    for run in range(runs):
        warmup_keys = operation_keys(keys, min(1000, len(keys)), offset=run * 17)
        for key in warmup_keys:
            await operation(key)
        samples_us: list[float] = []
        started = time.perf_counter()
        for key in scheduled_keys:
            op_started = time.perf_counter_ns()
            await operation(key)
            samples_us.append((time.perf_counter_ns() - op_started) / 1_000.0)
        elapsed = time.perf_counter() - started
        run_results.append(
            {
                **latency_summary(samples_us),
                "elapsed_seconds": elapsed,
                "ops_per_second": ops / elapsed,
            }
        )
    return summarize_sequential(run_results)


async def event_loop_ticker(
    stop: asyncio.Event, samples_us: list[float], interval: float = 0.001
) -> None:
    loop = asyncio.get_running_loop()
    next_tick = loop.time() + interval
    while not stop.is_set():
        await asyncio.sleep(max(0.0, next_tick - loop.time()))
        now = loop.time()
        samples_us.append(max(0.0, now - next_tick) * 1_000_000.0)
        next_tick += interval


async def measure_concurrent(
    operation: Callable[[str], Awaitable[Any]] | list[Callable[[str], Awaitable[Any]]],
    keys: list[str],
    *,
    total_ops: int,
    concurrency: int,
) -> dict[str, Any]:
    per_worker = total_ops // concurrency
    remainder = total_ops % concurrency
    latency_samples: list[float] = []
    errors: list[str] = []

    async def worker(worker_id: int, count: int) -> None:
        scheduled = operation_keys(keys, count, offset=worker_id * 13)
        worker_operation = (
            operation[worker_id % len(operation)]
            if isinstance(operation, list)
            else operation
        )
        for key in scheduled:
            started = time.perf_counter_ns()
            try:
                await worker_operation(key)
            except Exception as exc:  # benchmark failures are a measured result
                errors.append(type(exc).__name__)
            finally:
                latency_samples.append((time.perf_counter_ns() - started) / 1_000.0)

    ticker_samples: list[float] = []
    stop_ticker = asyncio.Event()
    ticker = asyncio.create_task(event_loop_ticker(stop_ticker, ticker_samples))
    started = time.perf_counter()
    await asyncio.gather(
        *[
            worker(worker_id, per_worker + (worker_id < remainder))
            for worker_id in range(concurrency)
        ]
    )
    elapsed = time.perf_counter() - started
    stop_ticker.set()
    await ticker

    summary = latency_summary(latency_samples)
    return {
        **summary,
        "total_ops": total_ops,
        "completed_ops": total_ops - len(errors),
        "errors": len(errors),
        "error_types": dict(sorted((name, errors.count(name)) for name in set(errors))),
        "elapsed_seconds": elapsed,
        "ops_per_second": total_ops / elapsed,
        "event_loop_delay_p95_us": percentile(ticker_samples, 0.95),
        "event_loop_delay_max_us": max(ticker_samples, default=0.0),
    }


async def setup_sqlite(
    config: Config,
    keys: list[str],
    value: bytes,
    expiry: float,
    pool_size: int,
    *,
    name: str | None = None,
    initialize: bool = True,
):
    name = name or f"redis-comparison-{uuid.uuid4().hex}"
    db = rapsqlite.connect_memory(
        name=name,
        pool_size=pool_size,
        timeout=config.sqlite_timeout,
    )
    await db.__aenter__()
    if initialize:
        await db.execute(SCHEMA_SQL)
        await db.execute(EXPIRATION_INDEX_SQL)
        rows = [[key, value, expiry] for key in keys]
        await db.execute_many(
            "INSERT INTO cache(key, value, expires_at) VALUES (?, ?, ?)", rows
        )
    return db


async def setup_redis(
    config: Config, keys: list[str], value: bytes, ttl: int, pool_size: int
):
    # Use Redis-py's async blocking pool so pool exhaustion queues work, as the
    # rapsqlite pool does, instead of turning ordinary concurrency into
    # immediate "Too many connections" errors.
    connection_pool = redis.BlockingConnectionPool(
        host=config.host,
        port=config.port,
        max_connections=pool_size,
        timeout=10,
        socket_timeout=10,
        socket_connect_timeout=5,
    )
    client = redis.Redis(
        connection_pool=connection_pool,
        host=config.host,
        port=config.port,
        decode_responses=False,
    )
    await client.ping()
    await client.flushdb()
    pipe = client.pipeline(transaction=False)
    for index, key in enumerate(keys, 1):
        pipe.set(key, value, ex=ttl)
        if index % 1000 == 0:
            await pipe.execute()
            pipe = client.pipeline(transaction=False)
    if pipe.command_stack:
        await pipe.execute()
    return client


async def close_sqlite(db: Any) -> None:
    await db.__aexit__(None, None, None)


async def close_redis(client: Any) -> None:
    await client.aclose()


async def run_sequential_scenarios(
    config: Config, keys: list[str], value: bytes, expiry: float
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for backend in ("rapsqlite", "redis.asyncio"):
        if backend == "rapsqlite":
            handle = await setup_sqlite(config, keys, value, expiry, pool_size=1)

            async def read(key: str) -> Any:
                row = await handle.fetch_one(READ_SQL, [key, time.time()])
                return row[0] if row else None

            async def write(key: str) -> Any:
                async with handle.transaction():
                    await handle.execute(UPSERT_SQL, [key, value, expiry])

            close = close_sqlite
        else:
            handle = await setup_redis(
                config, keys, value, config.expiration_seconds, pool_size=1
            )

            async def read(key: str) -> Any:
                return await handle.get(key)

            async def write(key: str) -> Any:
                return await handle.set(key, value, ex=config.expiration_seconds)

            close = close_redis

        try:
            for name, operation, is_write in (
                ("sequential_read", read, False),
                ("sequential_write", write, True),
            ):
                measured = await measure_sequential(
                    operation,
                    keys,
                    value,
                    ops=config.sequential_ops,
                    runs=config.runs,
                    write=is_write,
                )
                results.append(
                    {"backend": backend, "pool_size": 1, "scenario": name, **measured}
                )
        finally:
            await close(handle)
    return results


async def run_batch_scenarios(
    config: Config, keys: list[str], value: bytes, expiry: float
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    batch_keys = operation_keys(keys, config.sequential_ops)
    for backend in ("rapsqlite", "redis.asyncio"):
        if backend == "rapsqlite":
            handle = await setup_sqlite(config, keys, value, expiry, pool_size=1)
        else:
            handle = await setup_redis(
                config, keys, value, config.expiration_seconds, pool_size=1
            )

        run_results: list[dict[str, float]] = []
        try:
            for _run in range(config.runs):
                started = time.perf_counter()
                items = 0
                for key_chunk in chunked(batch_keys, config.batch_size):
                    if backend == "rapsqlite":
                        params = [[key, value, expiry] for key in key_chunk]
                        async with handle.transaction():
                            await handle.execute_many(UPSERT_SQL, params)
                    else:
                        pipe = handle.pipeline(transaction=False)
                        for key in key_chunk:
                            pipe.set(key, value, ex=config.expiration_seconds)
                        await pipe.execute()
                    items += len(key_chunk)
                elapsed = time.perf_counter() - started
                run_results.append(
                    {
                        "elapsed_seconds": elapsed,
                        "ops_per_second": items / elapsed,
                        "mean_us": elapsed * 1_000_000.0 / items,
                    }
                )
        finally:
            await (
                close_sqlite(handle) if backend == "rapsqlite" else close_redis(handle)
            )
        results.append(
            {
                "backend": backend,
                "pool_size": 1,
                "scenario": "batched_write",
                "batch_size": config.batch_size,
                "items": len(batch_keys),
                "runs": run_results,
                "median_run_mean_us": statistics.median(
                    r["mean_us"] for r in run_results
                ),
                "mean_ops_per_second": statistics.mean(
                    r["ops_per_second"] for r in run_results
                ),
            }
        )
    return results


async def run_concurrent_scenarios(
    config: Config, keys: list[str], value: bytes, expiry: float
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for backend in ("rapsqlite", "redis.asyncio"):
        for pool_size in config.pool_sizes:
            if backend == "rapsqlite":
                sqlite_name = f"redis-comparison-{uuid.uuid4().hex}"
                primary_handle = await setup_sqlite(
                    config,
                    keys,
                    value,
                    expiry,
                    pool_size=pool_size,
                    name=sqlite_name,
                )
                # Transactions are connection-scoped in rapsqlite. Use one
                # logical connection per worker so this measures shared-cache
                # contention rather than concurrent transaction calls on one
                # connection object.
                worker_handles = [primary_handle]
                for _ in range(max(config.concurrency_levels) - 1):
                    worker_handles.append(
                        await setup_sqlite(
                            config,
                            keys,
                            value,
                            expiry,
                            pool_size=pool_size,
                            name=sqlite_name,
                            initialize=False,
                        )
                    )

                def sqlite_read(handle: Any) -> Callable[[str], Awaitable[Any]]:
                    async def read(key: str) -> Any:
                        row = await handle.fetch_one(READ_SQL, [key, time.time()])
                        return row[0] if row else None

                    return read

                def sqlite_write(handle: Any) -> Callable[[str], Awaitable[Any]]:
                    async def write(key: str) -> Any:
                        async with handle.transaction():
                            await handle.execute(UPSERT_SQL, [key, value, expiry])

                    return write

                read = [sqlite_read(handle) for handle in worker_handles]
                write = [sqlite_write(handle) for handle in worker_handles]
            else:
                handle = await setup_redis(
                    config, keys, value, config.expiration_seconds, pool_size=pool_size
                )

                async def read(key: str) -> Any:
                    return await handle.get(key)

                async def write(key: str) -> Any:
                    return await handle.set(key, value, ex=config.expiration_seconds)

            try:
                for concurrency in config.concurrency_levels:
                    read_result = await measure_concurrent(
                        read,
                        keys,
                        total_ops=config.concurrent_ops,
                        concurrency=concurrency,
                    )
                    results.append(
                        {
                            "backend": backend,
                            "pool_size": pool_size,
                            "concurrency": concurrency,
                            "scenario": "concurrent_read",
                            **read_result,
                        }
                    )
                    write_result = await measure_concurrent(
                        write,
                        keys,
                        total_ops=max(1000, config.concurrent_ops // 4),
                        concurrency=concurrency,
                    )
                    results.append(
                        {
                            "backend": backend,
                            "pool_size": pool_size,
                            "concurrency": concurrency,
                            "scenario": "concurrent_write",
                            **write_result,
                        }
                    )
            finally:
                if backend == "rapsqlite":
                    for worker_handle in worker_handles:
                        await close_sqlite(worker_handle)
                else:
                    await close_redis(handle)
    return results


def print_results(results: list[dict[str, Any]], metadata: dict[str, Any]) -> None:
    print("\n=== Matched rapsqlite vs redis.asyncio cache benchmark ===")
    print(
        f"{metadata['keys']:,} keys × {metadata['value_size']:,} bytes; "
        f"{metadata['runs']} sequential runs; Redis {metadata['redis_version']}"
    )
    print(
        "Setup time is excluded. Latencies are measured around the actual async client call."
    )

    print("\nSequential scenarios (median of run averages; lower latency is better)")
    print(
        "backend         scenario          mean µs    p50 µs    p95 µs    p99 µs     ops/s"
    )
    for row in results:
        if not row["scenario"].startswith("sequential_"):
            continue
        print(
            f"{row['backend']:<15} {row['scenario']:<17} "
            f"{row['median_run_mean_us']:>9.2f} {row['median_run_p50_us']:>9.2f} "
            f"{row['median_run_p95_us']:>9.2f} {row['median_run_p99_us']:>9.2f} "
            f"{row['mean_ops_per_second']:>10,.0f}"
        )

    print("\nBatched writes (pipeline/transaction; per-item amortized time)")
    print("backend         batch size  mean µs/item       ops/s")
    for row in results:
        if row["scenario"] != "batched_write":
            continue
        print(
            f"{row['backend']:<15} {row['batch_size']:>10} "
            f"{row['median_run_mean_us']:>15.2f} {row['mean_ops_per_second']:>13,.0f}"
        )

    print(
        "\nConcurrent reads (p95/p99 are per-operation; event-loop delay is separate)"
    )
    print(
        "backend         pool  conc   p50 µs    p95 µs    p99 µs     ops/s  errors  loop p95 µs"
    )
    for row in results:
        if row["scenario"] != "concurrent_read":
            continue
        print(
            f"{row['backend']:<15} {row['pool_size']:>4} {row['concurrency']:>5} "
            f"{row['p50_us']:>9.2f} {row['p95_us']:>9.2f} {row['p99_us']:>9.2f} "
            f"{row['ops_per_second']:>10,.0f} {row['errors']:>7} {row['event_loop_delay_p95_us']:>12.2f}"
        )

    print(
        "\nConcurrent writes (SQLite lock/contention failures are reported, not hidden)"
    )
    print(
        "backend         pool  conc   p50 µs    p95 µs     ops/s  errors  loop p95 µs"
    )
    for row in results:
        if row["scenario"] != "concurrent_write":
            continue
        print(
            f"{row['backend']:<15} {row['pool_size']:>4} {row['concurrency']:>5} "
            f"{row['p50_us']:>9.2f} {row['p95_us']:>9.2f} {row['ops_per_second']:>10,.0f} "
            f"{row['errors']:>7} {row['event_loop_delay_p95_us']:>12.2f}"
        )


async def main(config: Config) -> dict[str, Any]:
    keys = [f"key{i:08d}" for i in range(config.keys)]
    value = b"x" * config.value_size
    expiry = time.time() + config.expiration_seconds

    redis_probe = redis.Redis(
        host=config.host, port=config.port, decode_responses=False
    )
    info = await redis_probe.info("server")
    await redis_probe.aclose()

    results = []
    results.extend(await run_sequential_scenarios(config, keys, value, expiry))
    results.extend(await run_batch_scenarios(config, keys, value, expiry))
    results.extend(await run_concurrent_scenarios(config, keys, value, expiry))

    metadata = {
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": sys.platform,
        "python": sys.version,
        "sqlite": sqlite3.sqlite_version,
        "rapsqlite": rapsqlite.__version__,
        "redis_client": redis_package.__version__,
        "redis_version": info.get("redis_version", "unknown"),
        "redis_host": config.host,
        "redis_port": config.port,
        "keys": config.keys,
        "value_size": config.value_size,
        "runs": config.runs,
        "sequential_ops": config.sequential_ops,
        "concurrent_ops": config.concurrent_ops,
        "batch_size": config.batch_size,
        "concurrency_levels": config.concurrency_levels,
        "pool_sizes": config.pool_sizes,
        "sqlite_timeout": config.sqlite_timeout,
        "expiration_seconds": config.expiration_seconds,
    }
    payload = {"metadata": metadata, "results": results}
    print_results(results, metadata)
    if config.json_out:
        config.json_out.parent.mkdir(parents=True, exist_ok=True)
        config.json_out.write_text(json.dumps(payload, indent=2) + "\n")
        print(f"\nRaw results: {config.json_out}")
    return payload


def parse_args() -> Config:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=6380)
    parser.add_argument("--keys", type=int, default=100_000)
    parser.add_argument("--value-size", type=int, default=1024)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--sequential-ops", type=int, default=20_000)
    parser.add_argument("--concurrent-ops", type=int, default=20_000)
    parser.add_argument("--batch-size", type=int, default=100)
    parser.add_argument("--concurrency-levels", default="1,8,32")
    parser.add_argument("--pool-sizes", default="1,4,16")
    parser.add_argument("--sqlite-timeout", type=float, default=0.25)
    parser.add_argument("--expiration-seconds", type=int, default=3600)
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()

    def parse_ints(raw: str) -> tuple[int, ...]:
        values = tuple(int(piece) for piece in raw.split(",") if piece.strip())
        if not values or any(value < 1 for value in values):
            raise ValueError("integer lists must contain positive values")
        return values

    return Config(
        host=args.host,
        port=args.port,
        keys=args.keys,
        value_size=args.value_size,
        runs=args.runs,
        sequential_ops=args.sequential_ops,
        concurrent_ops=args.concurrent_ops,
        batch_size=args.batch_size,
        concurrency_levels=parse_ints(args.concurrency_levels),
        pool_sizes=parse_ints(args.pool_sizes),
        sqlite_timeout=args.sqlite_timeout,
        expiration_seconds=args.expiration_seconds,
        json_out=args.json_out,
    )


if __name__ == "__main__":
    asyncio.run(main(parse_args()))
