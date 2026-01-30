# Performance Benchmarks

This directory contains performance benchmarks comparing `rapsqlite` with `aiosqlite` and `sqlite3`.

## Running Benchmarks

```bash
# Install test deps (includes aiosqlite for comparison)
pip install -r requirements-test.txt

# Run benchmarks (use same Python as your rapsqlite build)
python -m pytest benchmarks/benchmark_suite.py -v -s
```

## Benchmark Suite

Each benchmark is run **multiple times** (see `BENCHMARK_RUNS` in `benchmark_suite.py`, default 5); reported values are **averages** across runs.

1. **Simple Query Throughput** - Repeated SELECTs (20000 queries, 5000-row table; mean, median, P95, P99)
2. **Batch Insert Performance** - `execute_many()` with 50000 rows (avg ms)
3. **Concurrent Reads** - 10 workers × 2000 queries on 5000-row table (avg ms)
4. **Transaction Performance** - 100 transactions × 500 inserts each (avg ms)
5. **High Concurrency Reads** - 30 workers × 1000 queries on 5000-row table; showcases rapsqlite scaling
6. **Concurrent Batch Inserts** - 10 writers × 5000 rows each in parallel; showcases pool + true async
7. **Mixed Concurrent Workload** - 20 readers × 1000 + 5 writers × 500 on 5000-row table (fair comparison: same structure for both—25 separate connections)

All benchmarks use the **same structure for both packages** (no special-casing); each coroutine does its own `async with connect(db) as conn` and then work, so results are comparable.

## Expected Results

### Key Advantages of rapsqlite

- **True async**: All operations execute outside the Python GIL
- **Better concurrency**: No event loop stalls under load
- **Connection pooling**: Efficient connection reuse
- **Prepared statement caching**: Automatic query optimization

### Performance Characteristics

- **Latency**: rapsqlite typically shows similar or better latency than aiosqlite
- **Throughput**: Better throughput under concurrent load due to GIL independence
- **Scalability**: Better performance scaling with concurrent operations

## Benchmark Results

**Test Date**: 2026-01-29  
**System**: macOS (Darwin arm64)  
**Python Version**: 3.9.6  
**SQLite Version**: 3.51.0  
**rapsqlite Version**: 0.3.0-dev

*Note: Actual benchmark results will vary based on system configuration, load, and SQLite version. Run the benchmarks on your system for accurate measurements.*

### Actual Results (macOS arm64, Python 3.9.6, with aiosqlite, avg of 5 runs, ×10 row scale)

```
=== Simple Query Throughput (20000 queries, avg of 5 runs) ===
rapsqlite    - Mean: 0.317ms, Median: 0.285ms, P95: 0.406ms, P99: 0.965ms
aiosqlite    - Mean: 0.097ms, Median: 0.093ms, P95: 0.113ms, P99: 0.189ms
sqlite3      - Mean: 0.071ms, Median: 0.062ms, P95: 0.104ms, P99: 0.240ms

=== Batch Insert Performance (50000 rows, avg of 5 runs) ===
rapsqlite    - 84.573ms
aiosqlite    - 29.349ms
sqlite3      - 20.435ms

=== Concurrent Reads (10 workers × 2000 queries, avg of 5 runs) ===
rapsqlite    - 1206.418ms
aiosqlite    - 1439.381ms

=== Transaction Performance (100 transactions × 500 inserts, avg of 5 runs) ===
rapsqlite    - 4652.946ms
aiosqlite    - 1597.390ms

=== Benchmarks that showcase rapsqlite (same structure for both; avg of 5 runs) ===
High Concurrency Reads (30 workers × 1000 queries):
rapsqlite    - 1968.270ms
aiosqlite    - 2036.071ms

Concurrent Batch Inserts (10 writers × 5000 rows each):
rapsqlite    - 80.779ms
aiosqlite    - 127.367ms

=== Fair comparison: Mixed Concurrent Workload (25 separate connections for both; avg of 5 runs) ===
Mixed Concurrent Workload (20 readers × 1000 + 5 writers × 500):
rapsqlite    - 1480.186ms
aiosqlite    - 1737.003ms
```

### Performance Analysis

**Key Observations:**

1. **Simple Query Throughput** (20000 queries): rapsqlite ~0.32ms mean, aiosqlite ~0.10ms; sqlite3 ~0.07ms. At 10× scale rapsqlite shows higher mean latency; aiosqlite and sqlite3 remain lower due to less async/pool overhead per query.

2. **Batch Insert Performance** (50000 rows): rapsqlite ~85ms, aiosqlite ~29ms, sqlite3 ~20ms. rapsqlite runs the whole batch in a **single transaction** (BEGIN … COMMIT) on one pool connection via the raw SQLite C API; aiosqlite faster on single-batch due to dedicated worker + no pool/lock overhead.

3. **Concurrent Reads** (10 workers × 2000 queries): rapsqlite wins (~1206ms vs ~1439ms)—session-connection reuse lets each worker hold one connection for 2000 queries, matching aiosqlite-style usage and avoiding per-query pool acquire/release.

4. **Transaction Performance** (100 transactions × 500 inserts): rapsqlite ~4653ms, aiosqlite ~1597ms; aiosqlite's worker thread excels at many small transactions.

5. **High Concurrency Reads** (30 workers × 1000 queries): rapsqlite wins (~1968ms vs ~2036ms)—true async and connection pooling scale better when many coroutines hit the DB.

6. **Concurrent Batch Inserts** (10 writers × 5000 rows each): rapsqlite wins (~81ms vs ~127ms)—pool overlaps I/O; aiosqlite’s per-connection worker queue serializes more.

7. **Mixed Concurrent Workload** (20 readers × 1000 + 5 writers × 500): rapsqlite wins (~1480ms vs ~1737ms)—fair comparison with 25 separate connections; global pool registry and session-connection reuse help rapsqlite. rapsqlite wins on Concurrent Reads, High Concurrency Reads, Concurrent Batch Inserts, and Mixed Workload at this scale.

**Performance Characteristics:**

- **True Async**: All operations execute outside the Python GIL, providing better concurrency under load
- **Connection Pooling**: Efficient connection reuse reduces overhead
- **execute_many**: Uses one connection for the entire batch (avoids N pool acquire/release cycles)
- **Prepared Statement Caching**: sqlx automatically caches prepared statements per connection, improving repeated query performance
- **Scalability**: Better performance scaling with concurrent operations compared to fake async libraries
- **Bulk in a transaction**: Prefer one transaction + `execute_many` over many small transactions with single-row inserts

**Note on sqlite3 Comparison:**

The synchronous `sqlite3` module shows lower latency for single-threaded operations because it doesn't have async overhead. However, rapsqlite's advantage becomes clear under concurrent load where it can handle multiple operations simultaneously without blocking the event loop.

### Batch insert: single transaction (pool)

For the **pool path** (no transaction, no callbacks), rapsqlite runs `execute_many` in a **single transaction**: it acquires one pool connection, locks the raw SQLite handle, runs BEGIN then the entire bind/step/reset loop synchronously in `block_in_place`, then COMMIT. No per-row await; one transaction for the whole batch (matches aiosqlite/sqlite3 executemany semantics). Transaction and callback paths still use the per-row async loop.

**Why aiosqlite can be faster on batch insert:** aiosqlite uses a **dedicated worker thread** per connection. For `executemany` it enqueues one job (the whole batch) and the worker runs `sqlite3.executemany` on the **already-open** connection—no pool acquire, no handle lock. rapsqlite acquires a connection from the pool and locks the raw handle before running the batch, so it pays for pool + lock_handle + block_in_place each time. The raw insert loop itself is comparable (single transaction, prepare once, bind/step/reset); the gap is mainly this extra async/pool overhead. rapsqlite still uses a single transaction and avoids per-row await, so batch insert is in the same order of magnitude as aiosqlite.

## Interpreting Results

- **Lower is better** for all metrics (latency, elapsed time)
- **P95/P99 percentiles** show tail latency under load
- **Concurrent benchmarks** demonstrate scalability
- **Transaction benchmarks** show overhead of transaction management

## Contributing Benchmarks

To add new benchmarks:

1. Create a new test function in `benchmark_suite.py`
2. Follow the pattern of existing benchmarks
3. Include results in this README
4. Document any assumptions or system-specific considerations

## System Requirements

Benchmarks require:
- Python 3.8+
- rapsqlite (installed)
- aiosqlite (optional, for comparison)
- sqlite3 (standard library)

## Notes

- **Fair comparison**: All benchmarks use the same structure for rapsqlite and aiosqlite (same number of workers/connections, same operations; only API differences such as `execute_many` vs `executemany`). No special-casing per package, so results are comparable.
- **execute_many**: rapsqlite acquires one pool connection, runs BEGIN, then the batch synchronously on the raw sqlite3 handle (prepare once, bind/step/reset per row, no await in loop), then COMMIT. One transaction for the whole batch. Transaction and callback paths use the per-row async loop.
- Benchmarks use temporary databases that are cleaned up after each test
- Results may vary significantly based on:
  - System load
  - Disk I/O performance
  - SQLite version
  - Python version
  - Operating system
