"""Performance benchmarks comparing rapsqlite vs aiosqlite vs sqlite3 (Phase 2.15).

Run with: pytest benchmarks/benchmark_suite.py -v
"""

import pytest
import asyncio
import time
import tempfile
import os
import sys
import statistics

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

import rapsqlite


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
    """Benchmark: Simple SELECT queries throughput."""
    results = {}
    
    # rapsqlite
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name
    
    try:
        async with rapsqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
            for i in range(100):
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])
        
        async with rapsqlite.connect(test_db) as conn:
            times = []
            for _ in range(1000):
                start = time.perf_counter()
                await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])
                times.append(time.perf_counter() - start)
            results["rapsqlite"] = {
                "mean": statistics.mean(times) * 1000,  # ms
                "median": statistics.median(times) * 1000,
                "p95": statistics.quantiles(times, n=20)[18] * 1000,
                "p99": statistics.quantiles(times, n=100)[98] * 1000,
            }
    finally:
        cleanup_db(test_db)
    
    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            async with aiosqlite.connect(test_db) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
                for i in range(100):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))
            
            async with aiosqlite.connect(test_db) as conn:
                times = []
                for _ in range(1000):
                    start = time.perf_counter()
                    async with conn.execute("SELECT * FROM test WHERE value = ?", (50,)) as cursor:
                        await cursor.fetchall()
                    times.append(time.perf_counter() - start)
                results["aiosqlite"] = {
                    "mean": statistics.mean(times) * 1000,
                    "median": statistics.median(times) * 1000,
                    "p95": statistics.quantiles(times, n=20)[18] * 1000,
                    "p99": statistics.quantiles(times, n=100)[98] * 1000,
                }
        finally:
            cleanup_db(test_db)
    
    # sqlite3 (synchronous)
    if SQLITE3_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            conn = sqlite3.connect(test_db)
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
            for i in range(100):
                conn.execute("INSERT INTO test (value) VALUES (?)", (i,))
            conn.commit()
            
            times = []
            for _ in range(1000):
                start = time.perf_counter()
                conn.execute("SELECT * FROM test WHERE value = ?", (50,)).fetchall()
                times.append(time.perf_counter() - start)
            results["sqlite3"] = {
                "mean": statistics.mean(times) * 1000,
                "median": statistics.median(times) * 1000,
                "p95": statistics.quantiles(times, n=20)[18] * 1000,
                "p99": statistics.quantiles(times, n=100)[98] * 1000,
            }
            conn.close()
        finally:
            cleanup_db(test_db)
    
    print("\n=== Simple Query Throughput (1000 queries) ===")
    for lib, metrics in results.items():
        print(f"{lib:12} - Mean: {metrics['mean']:.3f}ms, Median: {metrics['median']:.3f}ms, "
              f"P95: {metrics['p95']:.3f}ms, P99: {metrics['p99']:.3f}ms")


@pytest.mark.asyncio
async def test_batch_insert_performance():
    """Benchmark: Batch insert performance with execute_many."""
    results = {}
    
    # rapsqlite
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name
    
    try:
        async with rapsqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            params = [[f"value_{i}"] for i in range(1000)]
            
            start = time.perf_counter()
            await conn.execute_many("INSERT INTO test (value) VALUES (?)", params)
            results["rapsqlite"] = (time.perf_counter() - start) * 1000
    finally:
        cleanup_db(test_db)
    
    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            async with aiosqlite.connect(test_db) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
                params = [(f"value_{i}",) for i in range(1000)]
                
                start = time.perf_counter()
                await conn.executemany("INSERT INTO test (value) VALUES (?)", params)
                await conn.commit()
                results["aiosqlite"] = (time.perf_counter() - start) * 1000
        finally:
            cleanup_db(test_db)
    
    # sqlite3
    if SQLITE3_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            conn = sqlite3.connect(test_db)
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            params = [(f"value_{i}",) for i in range(1000)]
            
            start = time.perf_counter()
            conn.executemany("INSERT INTO test (value) VALUES (?)", params)
            conn.commit()
            results["sqlite3"] = (time.perf_counter() - start) * 1000
            conn.close()
        finally:
            cleanup_db(test_db)
    
    print("\n=== Batch Insert Performance (1000 rows) ===")
    for lib, elapsed in results.items():
        print(f"{lib:12} - {elapsed:.3f}ms")


@pytest.mark.asyncio
async def test_concurrent_reads():
    """Benchmark: Concurrent read operations."""
    results = {}
    
    # rapsqlite
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name
    
    try:
        async with rapsqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
            for i in range(100):
                await conn.execute("INSERT INTO test (value) VALUES (?)", [i])
        
        async def read_worker():
            async with rapsqlite.connect(test_db) as conn:
                for _ in range(100):
                    await conn.fetch_all("SELECT * FROM test WHERE value = ?", [50])
        
        start = time.perf_counter()
        await asyncio.gather(*[read_worker() for _ in range(10)])
        results["rapsqlite"] = (time.perf_counter() - start) * 1000
    finally:
        cleanup_db(test_db)
    
    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            async with aiosqlite.connect(test_db) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
                for i in range(100):
                    await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))
            
            async def read_worker():
                async with aiosqlite.connect(test_db) as conn:
                    for _ in range(100):
                        async with conn.execute("SELECT * FROM test WHERE value = ?", (50,)) as cursor:
                            await cursor.fetchall()
            
            start = time.perf_counter()
            await asyncio.gather(*[read_worker() for _ in range(10)])
            results["aiosqlite"] = (time.perf_counter() - start) * 1000
        finally:
            cleanup_db(test_db)
    
    print("\n=== Concurrent Reads (10 workers × 100 queries) ===")
    for lib, elapsed in results.items():
        print(f"{lib:12} - {elapsed:.3f}ms")


@pytest.mark.asyncio
async def test_transaction_performance():
    """Benchmark: Transaction performance."""
    results = {}
    
    # rapsqlite
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        test_db = f.name
    
    try:
        async with rapsqlite.connect(test_db) as conn:
            await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
        
        async with rapsqlite.connect(test_db) as conn:
            start = time.perf_counter()
            for _ in range(100):
                async with conn.transaction():
                    for i in range(10):
                        await conn.execute("INSERT INTO test (value) VALUES (?)", [i])
            results["rapsqlite"] = (time.perf_counter() - start) * 1000
    finally:
        cleanup_db(test_db)
    
    # aiosqlite
    if AIOSQLITE_AVAILABLE:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
            test_db = f.name
        
        try:
            async with aiosqlite.connect(test_db) as conn:
                await conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
            
            async with aiosqlite.connect(test_db) as conn:
                start = time.perf_counter()
                for _ in range(100):
                    await conn.execute("BEGIN")
                    try:
                        for i in range(10):
                            await conn.execute("INSERT INTO test (value) VALUES (?)", (i,))
                        await conn.commit()
                    except Exception:
                        await conn.rollback()
                results["aiosqlite"] = (time.perf_counter() - start) * 1000
        finally:
            cleanup_db(test_db)
    
    print("\n=== Transaction Performance (100 transactions × 10 inserts) ===")
    for lib, elapsed in results.items():
        print(f"{lib:12} - {elapsed:.3f}ms")
