# Phase 0.5 release-build benchmark results

Recorded 2026-09-25 on one machine. This is an exploratory, same-machine
comparison of the released `v0.4.0` wheel with the `0.5.0` candidate. It is not
a performance guarantee, a production FastAPI benchmark, or evidence that
rapsqlite is universally faster than Redis.

## Environment and workload

- Apple M4 Pro, macOS 26.5.2, arm64; CPython 3.10.19; SQLite 3.51.0; Redis
  8.10.2.
- Both rapsqlite versions were release builds. The baseline used the actual
  `v0.4.0` wheel; the candidate reports package version `0.5.0`.
- Each SQLite database was in-memory and held one 1 KiB BLOB in a `WITHOUT
  ROWID` table with a partial expiration index. Reads checked the expiration
  timestamp in SQL; writes updated that existing key. Redis used its native
  one-hour TTL. The rapsqlite benchmark used `pool_size=1`.
- Each sequential scenario ran 3,000 operations, repeated three times, with up
  to 100 warm-up operations before each run. Each concurrent run performed
  1,000 operations at concurrency 1, 8, or 32. Concurrency 1 and 8 scenarios
  were repeated three times. The primary concurrency-32 comparison used ten
  runs; a second independent ten-run candidate batch is also preserved in the
  raw artifact to show run-to-run variation. Concurrent callers were
  independent awaits, not pipelined Redis commands.
- The Redis benchmark used a unique temporary key and deleted it afterward.
  Redis ran on loopback with persistence disabled; the pre-existing server was
  not reconfigured.
- Recorded process peak RSS was 51.4–54.9 MiB for the baseline and 52.3–58.5
  MiB for the candidate. This measures the entire benchmark process, not
  cache-only memory; candidate runs exercise more variants, so these are not a
  controlled memory comparison.

For sequential results, p50/p95/p99 are the medians of the three run-level
percentiles and throughput is the mean run throughput. For concurrent results,
percentiles are the medians of per-run percentiles; throughput is total
operations divided by total elapsed time. These are observed request
percentiles, not confidence intervals. The complete metadata and per-run
measurements are in [`phase5_hotpath_results.json`](phase5_hotpath_results.json).

## Sequential reads

This table uses the run configured for concurrency 1. Session affinity is off
for the matched `fetch_one` and `fetch_scalar` rows. Candidate-only paths are
included to show what the new wrappers did on this workload, not to imply that
they win generally.

| Client/path | p50 | p95 | p99 | Throughput |
|---|---:|---:|---:|---:|
| rapsqlite 0.4 `fetch_one` | 47.92 µs | 67.67 µs | 79.21 µs | 20,355 ops/s |
| rapsqlite 0.5 `fetch_one` | 42.25 µs | 48.21 µs | 67.26 µs | 22,969 ops/s |
| rapsqlite 0.5 `fetch_scalar` | 42.04 µs | 48.21 µs | 73.63 µs | 22,881 ops/s |
| rapsqlite 0.5 `PreparedQuery.fetch_scalar` | 41.54 µs | 48.50 µs | 72.88 µs | 23,195 ops/s |
| rapsqlite 0.5 raw BLOB path, session affinity on | 52.42 µs | 69.79 µs | 104.04 µs | 18,667 ops/s |
| synchronous `sqlite3` | 0.92 µs | 1.00 µs | 1.13 µs | 755,292 ops/s |
| `redis.asyncio` GET, one await per request | 167.21 µs | 200.39 µs | 346.18 µs | 5,599 ops/s |

The synchronous measurement has no async or client/server hand-off and is not
an equivalent client comparison. The prepared wrapper did not materially
improve on `fetch_one` in this run. The raw path was slower than the normal
scalar path here, even with session affinity enabled. Session affinity remains
opt-in because its benefit varies by workload.

No-op versus feature-enabled spot checks (sequential, candidate, affinity off):

| Path | p50 / p95 / p99 | Throughput |
|---|---:|---:|
| `fetch_scalar`, no optional feature | 42.04 / 48.21 / 73.63 µs | 22,881 ops/s |
| Query-usage tracking enabled | 41.58 / 49.13 / 79.30 µs | 22,991 ops/s |
| Trace callback configured | 42.25 / 51.25 / 92.21 µs | 22,647 ops/s |
| Explicit transaction active | 41.50 / 49.00 / 81.92 µs | 22,939 ops/s |

These short runs show the enabled paths remain functional, with differences
small enough to be dominated by run variance; they are not a claim that enabled
callbacks have zero cost. Init-hook execution and feature transitions are
covered by correctness tests.

Query-usage tracking spot checks (candidate, 3,000 calls per run, three runs,
session affinity off; each tracked variant was measured after its untracked
counterpart in the same harness invocation):

| Operation | Tracking off p50 / p95 / p99 | Tracking on p50 / p95 / p99 | Throughput off / on |
|---|---:|---:|---:|
| `fetch_scalar` | 51.58 / 57.79 / 72.25 µs | 50.40 / 58.33 / 75.10 µs | 19,010 / 19,948 ops/s |
| `execute` upsert | 86.63 / 96.79 / 116.65 µs | 86.67 / 99.63 / 123.25 µs | 11,347 / 11,218 ops/s |
| `execute_many` (16 upserts per call) | 177.79 / 217.10 / 299.92 µs | 199.19 / 244.69 / 327.30 µs | 5,383 / 4,877 batches/s |
| `SQLiteCache.get` | 55.67 / 64.29 / 94.84 µs | 57.13 / 75.88 / 118.67 µs | 17,591 / 16,554 ops/s |

These sequential spot checks are short, order-fixed samples, not a paired
microbenchmark or confidence interval. They indicate that tracking is not free,
especially for batch calls, and support keeping it disabled by default. The
complete per-run data and environment metadata are in
[`phase5_query_usage_results.json`](phase5_query_usage_results.json). Batch
latency is per call containing 16 SQL statements; its throughput is batches per
second, not individual statements per second.

## Concurrent reads

Each cell is p50 / p95 / p99. Throughput, p95 event-loop delay, and errors are
shown separately. Redis uses one awaited GET per operation, without pipelining.

| Callers | rapsqlite 0.4 | rapsqlite 0.5 | Redis GET |
|---:|---:|---:|---:|
| 1 | 51.04 / 68.13 / 78.60 µs | 43.04 / 52.05 / 70.82 µs | 177.50 / 309.94 / 626.03 µs |
| 8 | 311.25 / 425.56 / 480.27 µs | 247.85 / 365.01 / 437.93 µs | 361.98 / 439.63 / 546.15 µs |
| 32 | 1,022.90 / 1,175.70 / 1,233.94 µs | 932.74 / 1,111.30 / 1,260.59 µs | 1,210.66 / 1,694.41 / 1,930.76 µs |

| Callers | Throughput: 0.4 / 0.5 / Redis | Event-loop p95: 0.4 / 0.5 / Redis | Errors |
|---:|---:|---:|---:|
| 1 | 18,895 / 22,377 / 4,369 ops/s | 47.0 / 42.9 / 194.2 µs | 0 / 0 / 0 |
| 8 | 24,932 / 30,480 / 21,616 ops/s | 139.8 / 82.6 / 273.7 µs | 0 / 0 / 0 |
| 32 | 30,846 / 33,276 / 23,928 ops/s | 105.3 / 73.0 / 1,087.8 µs | 0 / 0 / 0 |

The candidate improved the measured read percentiles and throughput at
concurrency 1 and 8. The primary ten-run c32 batch improved p50/p95 and
throughput, while p99 was close to the v0.4.0 sample. A second independent
ten-run candidate batch measured 977 / 1,104 / 1,181 µs at c32 and 31,759
ops/s, illustrating run-to-run spread. Both batches had zero errors. This is
not a controlled CI performance guarantee. Redis throughput was similar at
c32, with a much larger event-loop delay in this particular setup. Different
machines, pool sizes, and pipelining can change the result.

Session-affinity toggle on the candidate (p50 / p95 / p99):

| Callers | Affinity off | Affinity on |
|---:|---:|---:|
| 1 | 43.04 / 52.05 / 70.82 µs | 42.50 / 51.38 / 78.31 µs |
| 8 | 247.85 / 365.01 / 437.93 µs | 307.50 / 468.55 / 719.16 µs |
| 32 | 932.74 / 1,111.30 / 1,260.59 µs | 923.20 / 1,133.46 / 1,275.32 µs |

Affinity was similar at 1 and 32 callers but slower at 8 callers in this run.
Since it retains a physical connection and consumes pool capacity,
applications should enable it only after testing their own workload.

## Writes and interpretation

The matched sequential write updates one existing key. It reports p50 / p95 /
p99 and mean run throughput:

| Client/path | p50 / p95 / p99 | Throughput |
|---|---:|---:|
| rapsqlite 0.4 `execute` upsert | 80.54 / 117.88 / 139.76 µs | 11,552 ops/s |
| rapsqlite 0.5 `execute` upsert | 76.73 / 87.00 / 134.24 µs | 12,578 ops/s |
| rapsqlite 0.5 `PreparedQuery.execute` | 77.00 / 88.29 / 148.88 µs | 12,519 ops/s |
| synchronous `sqlite3` upsert | 1.79 / 1.92 / 2.13 µs | 511,946 ops/s |
| `redis.asyncio` SET with TTL | 179.25 / 237.35 / 317.09 µs | 5,391 ops/s |

Concurrent writes (p50 / p95 / p99; aggregate throughput):

| Callers | rapsqlite 0.4 | rapsqlite 0.5 | Redis SET with TTL |
|---:|---:|---:|---:|
| 1 | 81.08 / 108.71 / 130.32 µs; 11,746 ops/s | 79.10 / 122.89 / 166.97 µs; 11,799 ops/s | 176.94 / 222.50 / 285.66 µs; 5,197 ops/s |
| 8 | 528.63 / 730.77 / 825.45 µs; 14,908 ops/s | 430.92 / 598.45 / 706.89 µs; 18,064 ops/s | 413.21 / 505.90 / 589.04 µs; 19,269 ops/s |
| 32 | 1,764.28 / 1,963.84 / 2,067.99 µs; 17,954 ops/s | 1,836.27 / 2,132.11 / 2,282.11 µs; 17,445 ops/s | 1,419.06 / 1,936.29 / 2,117.13 µs; 21,739 ops/s |

The 0.5 default write path was similar to 0.4 sequentially. Under concurrent
writes, 0.5 improved the c8 sample but the primary c32 batch was slightly
slower than v0.4; a second candidate c32 batch measured 1,716 / 1,949 / 2,044
µs and 18,558 ops/s, near the baseline. Treat c32 writes as comparable within
the observed run variance, not a demonstrated win. Redis had higher aggregate
write throughput at c8 and c32. The benchmark does not include expiration cleanup,
eviction, multi-process sharing, durable databases, or HTTP/serialization
overhead.

The data supports a process-local SQLite cache as a credible option, not a
general “faster than Redis” claim. It also shows that specialized raw and
prepared APIs are workload-dependent; keep them opt-in and measure the actual
application call pattern. Redis batching and shared-server deployment are
different workloads from the one-await-at-a-time local comparison here.

Reproduce with a release build and local Redis using `benchmarks/phase5_hotpath.py`;
the recorded commands used `--ops 3000 --runs 3 --concurrent-ops 1000`, with
`--concurrent-runs 3` at concurrency 1 and 8 and `--concurrent-runs 10` at
concurrency 32.
