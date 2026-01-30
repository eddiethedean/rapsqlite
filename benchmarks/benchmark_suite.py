"""Performance benchmarks comparing rapsqlite vs aiosqlite vs sqlite3 (Phase 2.15).

Run with: pytest benchmarks/benchmark_suite.py -v -s

Each benchmark is run BENCHMARK_RUNS times; reported values are averages across runs.
All benchmarks use the same structure for both rapsqlite and aiosqlite (same number of
workers/connections, same operations; only API differences). No special-casing per package.
"""

import pytest
import asyncio
import time
import tempfile
import os
import sys
import statistics

# Number of times to run each benchmark; reported metrics are averages across runs.
BENCHMARK_RUNS = 5

# Row scale for benchmarks (×10 from previous scale for stress-test at scale).
BATCH_INSERT_ROWS = 50000
CONCURRENT_BATCH_ROWS_PER_WRITER = 5000
TRANSACTION_INSERTS_PER_TX = 500
SETUP_ROWS = 5000
SIMPLE_QUERY_COUNT = 20000
CONCURRENT_READS_PER_WORKER = 2000
CONCURRENT_READS_WORKERS = 10
HIGH_CONCURRENCY_READS_WORKERS = 30
HIGH_CONCURRENCY_READS_PER_WORKER = 1000
CONCURRENT_BATCH_WRITERS = 10
MIXED_READERS = 20
MIXED_READS_PER_READER = 1000
MIXED_WRITERS = 5
MIXED_WRITES_PER_WRITER = 500
TRANSACTION_COUNT = 100

try:
    import aiosqlite

    AIOSQLITE_AVAILABLE = True
except ImportError:
    AIOSQLITE_AVAILABLE = False

try:
    import sqlite3

    SQLITE3_AVAILABLE = True
except ImportError:
    SQLITE3_AVAILABLE = False

import rapsqlite  # noqa: E402


def cleanup_db(test_db: str) -> None:
    """Helper to clean up database file."""
    if os.path.exists(test_db):
        try:
            os.unlink(test_db)
        except (PermissionError, OSError):
            if sys.platform == "win32":
                pass
            else:
                raise


@pytest.mark.asyncio
async def test_simple_query_throughput():
    """Benchmark: Simple SELECT queries throughput (avg over BENCHMARK_RUNS)."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    run_means = []
    run_medians = []
    run_p95 = []
    run_p99 = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )
                for i in range(SETUP_ROWS):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            async with rapsqlite.connect(test_db) as conn:
                times = []
                for _ in range(SIMPLE_QUERY_COUNT):
                    start = time.perf_counter()
                    await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])
                    times.append(time.perf_counter() - start)
                run_means.append(statistics.mean(times) * 1000)
                run_medians.append(statistics.median(times) * 1000)
                run_p95.append(statistics.quantiles(times, n=20)[18] * 1000)
                run_p99.append(statistics.quantiles(times, n=100)[98] * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = {
        "mean": statistics.mean(run_means),
        "median": statistics.mean(run_medians),
        "p95": statistics.mean(run_p95),
        "p99": statistics.mean(run_p99),
    }

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        run_means = []
        run_medians = []
        run_p95 = []
        run_p99 = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )
                    for i in range(SETUP_ROWS):
                        await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))

                async with aiosqlite.connect(test_db) as conn:
                    times = []
                    for _ in range(SIMPLE_QUERY_COUNT):
                        start = time.perf_counter()
                        async with conn.execute(
                            "SELECT * FROM test WHERE value = ?", (50,)
                        ) as cursor:
                            await cursor.fetchall()
                        times.append(time.perf_counter() - start)
                    run_means.append(statistics.mean(times) * 1000)
                    run_medians.append(statistics.median(times) * 1000)
                    run_p95.append(statistics.quantiles(times, n=20)[18] * 1000)
                    run_p99.append(statistics.quantiles(times, n=100)[98] * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = {
            "mean": statistics.mean(run_means),
            "median": statistics.mean(run_medians),
            "p95": statistics.mean(run_p95),
            "p99": statistics.mean(run_p99),
        }

    # sqlite3 (synchronous)
    if SQLITE3_AVAILABLE:
        run_means = []
        run_medians = []
        run_p95 = []
        run_p99 = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                conn = sqlite3.connect(test_db)
                conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )
                for i in range(SETUP_ROWS):
                    conn.execute("INSERT INTO test (value) VALUES (?)", (i,))
                conn.commit()

                times = []
                for _ in range(SIMPLE_QUERY_COUNT):
                    start = time.perf_counter()
                    conn.execute("SELECT * FROM test WHERE value = ?", (50,)).fetchall()
                    times.append(time.perf_counter() - start)
                run_means.append(statistics.mean(times) * 1000)
                run_medians.append(statistics.median(times) * 1000)
                run_p95.append(statistics.quantiles(times, n=20)[18] * 1000)
                run_p99.append(statistics.quantiles(times, n=100)[98] * 1000)
                conn.close()
            finally:
                cleanup_db(test_db)
        results["sqlite3"] = {
            "mean": statistics.mean(run_means),
            "median": statistics.mean(run_medians),
            "p95": statistics.mean(run_p95),
            "p99": statistics.mean(run_p99),
        }

    print(
        f"\n=== Simple Query Throughput ({SIMPLE_QUERY_COUNT} queries, avg of {runs} runs) ==="
    )
    for lib, metrics in results.items():
        print(
            f"{lib:12} - Mean: {metrics['mean']:.3f}ms, Median: {metrics['median']:.3f}ms, "
            f"P95: {metrics['p95']:.3f}ms, P99: {metrics['p99']:.3f}ms"
        )


@pytest.mark.asyncio
async def test_batch_insert_performance():
    """Benchmark: Batch insert performance with execute_many (avg over BENCHMARK_RUNS)."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
                )
                params = [[f"value_{i}"] for i in range(BATCH_INSERT_ROWS)]

                start = time.perf_counter()
                await conn.execute_many("INSERT INTO test (value) VALUES (?)", params)
                elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
                    )
                    params = [(f"value_{i}",) for i in range(BATCH_INSERT_ROWS)]

                    start = time.perf_counter()
                    await conn.executemany(
                        "INSERT INTO test (value) VALUES (?)", params
                    )
                    await conn.commit()
                    elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    # sqlite3
    if SQLITE3_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                conn = sqlite3.connect(test_db)
                conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                params = [(f"value_{i}",) for i in range(BATCH_INSERT_ROWS)]

                start = time.perf_counter()
                conn.executemany("INSERT INTO test (value) VALUES (?)", params)
                conn.commit()
                elapsed_ms.append((time.perf_counter() - start) * 1000)
                conn.close()
            finally:
                cleanup_db(test_db)
        results["sqlite3"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== Batch Insert Performance ({BATCH_INSERT_ROWS} rows, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(f"{lib:12} - {avg_ms:.3f}ms")


@pytest.mark.asyncio
async def test_concurrent_reads():
    """Benchmark: Concurrent read operations (avg over BENCHMARK_RUNS)."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )
                for i in range(SETUP_ROWS):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            async def read_worker():
                async with rapsqlite.connect(test_db) as conn:
                    for _ in range(CONCURRENT_READS_PER_WORKER):
                        await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])

            start = time.perf_counter()
            await asyncio.gather(
                *[read_worker() for _ in range(CONCURRENT_READS_WORKERS)]
            )
            elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )
                    for i in range(SETUP_ROWS):
                        await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))

                async def read_worker():
                    async with aiosqlite.connect(test_db) as conn:
                        for _ in range(CONCURRENT_READS_PER_WORKER):
                            async with conn.execute(
                                "SELECT * FROM test WHERE value = ?", (50,)
                            ) as cursor:
                                await cursor.fetchall()

                start = time.perf_counter()
                await asyncio.gather(
                    *[read_worker() for _ in range(CONCURRENT_READS_WORKERS)]
                )
                elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== Concurrent Reads ({CONCURRENT_READS_WORKERS} workers × {CONCURRENT_READS_PER_WORKER} queries, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(f"{lib:12} - {avg_ms:.3f}ms")


@pytest.mark.asyncio
async def test_transaction_performance():
    """Benchmark: Transaction performance (avg over BENCHMARK_RUNS)."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )

            async with rapsqlite.connect(test_db) as conn:
                start = time.perf_counter()
                for _ in range(TRANSACTION_COUNT):
                    async with conn.transaction():
                        for i in range(TRANSACTION_INSERTS_PER_TX):
                            await conn.execute(
                                "INSERT INTO test (value) VALUES (?)", [i]
                            )
                elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )

                async with aiosqlite.connect(test_db) as conn:
                    start = time.perf_counter()
                    for _ in range(TRANSACTION_COUNT):
                        await conn.execute("BEGIN")
                        try:
                            for i in range(TRANSACTION_INSERTS_PER_TX):
                                await conn.execute(
                                    "INSERT INTO test (value) VALUES (?)", (i,)
                                )
                            await conn.commit()
                        except Exception:
                            await conn.rollback()
                    elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== Transaction Performance ({TRANSACTION_COUNT} transactions × {TRANSACTION_INSERTS_PER_TX} inserts, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(f"{lib:12} - {avg_ms:.3f}ms")


# --- Benchmarks that showcase rapsqlite's strengths (true async, pool, concurrency) ---


@pytest.mark.asyncio
async def test_high_concurrency_reads():
    """Benchmark: Many concurrent readers. Shows rapsqlite scaling."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )
                for i in range(SETUP_ROWS):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            async def read_worker():
                async with rapsqlite.connect(test_db) as conn:
                    for _ in range(HIGH_CONCURRENCY_READS_PER_WORKER):
                        await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])

            start = time.perf_counter()
            await asyncio.gather(
                *[read_worker() for _ in range(HIGH_CONCURRENCY_READS_WORKERS)]
            )
            elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )
                    for i in range(SETUP_ROWS):
                        await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))

                async def read_worker():
                    async with aiosqlite.connect(test_db) as conn:
                        for _ in range(HIGH_CONCURRENCY_READS_PER_WORKER):
                            async with conn.execute(
                                "SELECT * FROM test WHERE value = ?", (50,)
                            ) as cursor:
                                await cursor.fetchall()

                start = time.perf_counter()
                await asyncio.gather(
                    *[read_worker() for _ in range(HIGH_CONCURRENCY_READS_WORKERS)]
                )
                elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== High Concurrency Reads ({HIGH_CONCURRENCY_READS_WORKERS} workers × {HIGH_CONCURRENCY_READS_PER_WORKER} queries, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(
            f"{lib:12} - {avg_ms:.3f}ms (lower is better; rapsqlite true async scales)"
        )


@pytest.mark.asyncio
async def test_concurrent_batch_inserts():
    """Benchmark: Many coroutines each doing one batch insert in parallel."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )

            async def batch_writer(worker_id: int):
                async with rapsqlite.connect(test_db) as conn:
                    params = [
                        [worker_id * 10000 + i]
                        for i in range(CONCURRENT_BATCH_ROWS_PER_WRITER)
                    ]
                    await conn.execute_many(
                        "INSERT INTO test (value) VALUES (?)", params
                    )

            start = time.perf_counter()
            await asyncio.gather(
                *[batch_writer(i) for i in range(CONCURRENT_BATCH_WRITERS)]
            )
            elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )

                async def batch_writer(worker_id: int):
                    async with aiosqlite.connect(test_db) as conn:
                        params = [
                            (worker_id * 10000 + i,)
                            for i in range(CONCURRENT_BATCH_ROWS_PER_WRITER)
                        ]
                        await conn.executemany(
                            "INSERT INTO test (value) VALUES (?)", params
                        )
                        await conn.commit()

                start = time.perf_counter()
                await asyncio.gather(
                    *[batch_writer(i) for i in range(CONCURRENT_BATCH_WRITERS)]
                )
                elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== Concurrent Batch Inserts ({CONCURRENT_BATCH_WRITERS} writers × {CONCURRENT_BATCH_ROWS_PER_WRITER} rows each, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(
            f"{lib:12} - {avg_ms:.3f}ms (lower is better; rapsqlite pool overlaps I/O)"
        )


@pytest.mark.asyncio
async def test_mixed_concurrent_workload():
    """Benchmark: Mixed workload — many readers + writers concurrently."""
    results = {}
    runs = BENCHMARK_RUNS

    # rapsqlite
    elapsed_ms = []
    for _ in range(runs):
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        try:
            async with rapsqlite.connect(test_db) as conn:
                await conn.execute(
                    "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                )
                for i in range(SETUP_ROWS):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", [i])

            async def reader():
                async with rapsqlite.connect(test_db) as conn:
                    for _ in range(MIXED_READS_PER_READER):
                        await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])

            async def writer():
                async with rapsqlite.connect(test_db) as conn:
                    for i in range(MIXED_WRITES_PER_WRITER):
                        await conn.execute(
                            "INSERT INTO test (value) VALUES (?)", [9999 + i]
                        )

            start = time.perf_counter()
            await asyncio.gather(
                *[reader() for _ in range(MIXED_READERS)],
                *[writer() for _ in range(MIXED_WRITERS)],
            )
            elapsed_ms.append((time.perf_counter() - start) * 1000)
        finally:
            cleanup_db(test_db)
    results["rapsqlite"] = statistics.mean(elapsed_ms)

    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        elapsed_ms = []
        for _ in range(runs):
            with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
                test_db = f.name
            try:
                async with aiosqlite.connect(test_db) as conn:
                    await conn.execute(
                        "CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)"
                    )
                    for i in range(SETUP_ROWS):
                        await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))

                async def reader():
                    async with aiosqlite.connect(test_db) as conn:
                        for _ in range(MIXED_READS_PER_READER):
                            async with conn.execute(
                                "SELECT * FROM test WHERE value = ?", (50,)
                            ) as cursor:
                                await cursor.fetchall()

                async def writer():
                    async with aiosqlite.connect(test_db) as conn:
                        for i in range(MIXED_WRITES_PER_WRITER):
                            await conn.execute(
                                "INSERT INTO test (value) VALUES (?)", (9999 + i,)
                            )
                        await conn.commit()

                start = time.perf_counter()
                await asyncio.gather(
                    *[reader() for _ in range(MIXED_READERS)],
                    *[writer() for _ in range(MIXED_WRITERS)],
                )
                elapsed_ms.append((time.perf_counter() - start) * 1000)
            finally:
                cleanup_db(test_db)
        results["aiosqlite"] = statistics.mean(elapsed_ms)

    print(
        f"\n=== Mixed Concurrent Workload ({MIXED_READERS} readers × {MIXED_READS_PER_READER} + "
        f"{MIXED_WRITERS} writers × {MIXED_WRITES_PER_WRITER}, avg of {runs} runs) ==="
    )
    for lib, avg_ms in results.items():
        print(
            f"{lib:12} - {avg_ms:.3f}ms (fair comparison: same structure for both, 25 separate connections)"
        )
