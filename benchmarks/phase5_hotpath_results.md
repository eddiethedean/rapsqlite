# Phase 0.5 hot-path benchmark results

Exploratory same-machine comparison recorded 2026-09-24 with the release-built
rapsqlite extension. These results document one local environment; they are not
a universal performance claim or a comparison with Redis pipelining.

## Environment and workload

- Apple M4 Pro, macOS 26.5.2, arm64
- CPython 3.10.19, SQLite 3.51.0, Redis 8.10.2
- 1 KiB BLOB value; SQLite query checks `expires_at`, while Redis uses a 1-hour
  key expiration
- `rapsqlite.fetch_scalar()` used its normal pool route with session affinity
  disabled; Redis was accessed with one awaited `redis.asyncio` GET per request
- 10,000 sequential operations per run, 3 runs, 100 warm-up operations per run;
  concurrent runs used 3,000 total operations at concurrency 1, 8, and 32
- Redis ran locally on loopback with persistence disabled. Redis pipelining was
  not enabled. The existing Redis service on port 6380 was left untouched.
- The benchmark process peak RSS ranged from 53.3 to 57.0 MiB across the three
  invocations; this includes Python and benchmark overhead, not cache-only use.

## Sequential request latency

Values are medians across the three benchmark invocations configured for
concurrency 1, 8, and 32; each invocation reports the median of three run-level
statistics.

| Client operation | p50 | p95 | p99 | Throughput |
|---|---:|---:|---:|---:|
| rapsqlite `fetch_scalar` | 49.42 µs | 55.00 µs | 59.63 µs | 20,482 ops/s |
| `redis.asyncio` GET | 79.13 µs | 98.54 µs | 107.83 µs | 12,154 ops/s |

## Concurrent request latency

Each cell reports p50 / p95 / p99 latency, aggregate throughput, and p95 event
loop delay. Errors were zero in every workload.

| Concurrent callers | Client operation | p50 / p95 / p99 | Throughput | Event-loop delay p95 |
|---:|---|---:|---:|---:|
| 1 | rapsqlite `fetch_scalar` | 52.54 / 59.00 / 66.29 µs | 18,760 ops/s | 44.49 µs |
| 1 | `redis.asyncio` GET | 78.46 / 91.92 / 101.34 µs | 12,488 ops/s | 77.21 µs |
| 8 | rapsqlite `fetch_scalar` | 321.04 / 415.43 / 455.18 µs | 24,556 ops/s | 102.59 µs |
| 8 | `redis.asyncio` GET | 316.25 / 360.98 / 390.24 µs | 24,163 ops/s | 339.22 µs |
| 32 | rapsqlite `fetch_scalar` | 1,090.65 / 1,275.34 / 1,335.51 µs | 29,986 ops/s | 110.57 µs |
| 32 | `redis.asyncio` GET | 1,152.71 / 1,369.15 / 7,000.84 µs | 25,344 ops/s | 1,158.40 µs |

The concurrent p99 for Redis at concurrency 32 includes a large outlier; do not
read the throughput result as a latency guarantee. Concurrent callers were
issued as independent awaits, not pipelined Redis commands. The benchmark does
not cover cancellations, multi-process sharing, write workloads,
cleanup/eviction, or a production FastAPI endpoint; cancellation correctness is
covered by the test suite.

## Interpretation

In this local run, sequential `rapsqlite.fetch_scalar()` latency was lower than
one-at-a-time `redis.asyncio` GET latency. Under concurrent load, aggregate
throughput was similar at concurrency 8 and higher for rapsqlite at concurrency
32, while per-request tail latency rose substantially for both backends. These
results support the process-local low-latency use case; they do not establish
that rapsqlite is faster than pipelined Redis or better for shared-cache
deployments. Re-run the harness on deployment hardware before using these
numbers for capacity planning.

Reproduce with `benchmarks/phase5_hotpath.py`, a release build, and a local
Redis server; the recorded runs used `--ops 10000 --runs 3
--concurrent-ops 3000` with `--concurrency 1`, `8`, and `32`.
