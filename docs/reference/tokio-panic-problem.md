# The Tokio Context Panic Problem

## Problem statement

When rapsqlite runs in certain shutdown or garbage-collection scenarios, the process can panic with:

```text
thread '<unnamed>' panicked at .../sqlx-core-0.8.6/src/pool/connection.rs:208:13:
this functionality requires a Tokio context
```

The panic occurs **after** normal work completes (e.g. after all tests pass). It is triggered during process or worker shutdown when something still holds a sqlx pool connection and that value is dropped without an active Tokio runtime.

## When it happens

Typical situations:

1. **Connection dropped without `close()`**  
   A `Connection` is abandoned (no `await conn.close()` and no `async with`). Python’s garbage collector later drops the Rust `Connection`. Any pooled connection still held by that object is then dropped without a Tokio context.

2. **Parallel test workers (e.g. pytest `-n 10`)**  
   Each worker runs tests and then exits. During worker shutdown, finalizers and GC run. Connections and pools are dropped in an order and on threads where Tokio is no longer current, which can trigger the same panic.

3. **Interpreter shutdown**  
   At process exit, statics and remaining Python objects are torn down. Again, drops can happen without a Tokio context.

## Root cause

- **Where**: sqlx-core (e.g. 0.8.6), `src/pool/connection.rs`, in `impl Drop for PoolConnection<DB>`.
- **What**: `PoolConnection::Drop` uses `crate::rt::spawn(...)` (Tokio’s spawn) to return the connection to the pool or close it. That API **requires** a current Tokio runtime.
- **Why it breaks**: The Tokio runtime used by rapsqlite is only “current” while a Rust async future is being polled (e.g. while Python is awaiting a rapsqlite call). During GC, finalization, or worker/process shutdown, no such future is running, so there is no Tokio context and `spawn` panics.

Relevant sqlx code (conceptually):

```rust
impl<DB: Database> Drop for PoolConnection<DB> {
    fn drop(&mut self) {
        if self.close_on_drop {
            crate::rt::spawn(self.take_and_close());  // needs Tokio
            return;
        }
        if self.live.is_some() || self.pool.options.min_connections > 0 {
            crate::rt::spawn(self.return_to_pool());   // needs Tokio
        }
    }
}
```

So: any time a `PoolConnection` (or a type that contains one) is dropped without a current Tokio runtime, this panic can occur.

## Mitigations implemented in rapsqlite

1. **PoolConnectionSlot** (`src/pool.rs`)  
   All stored pool connections (e.g. `session_connection`, `transaction_connection`, `callback_connection`) are kept inside a `PoolConnectionSlot` wrapper. When the slot is dropped **outside** a Tokio runtime, the wrapper **forgets** the inner `PoolConnection` instead of dropping it, so sqlx’s `PoolConnection::Drop` never runs and the panic is avoided. Temporaries used in `backup()` and in the begin/transaction path are also wrapped in `PoolConnectionSlot`.

2. **PoolSlot** (`src/pool.rs`)  
   The connection’s pool reference and the global pool registry store `SqlitePool` inside a `PoolSlot` wrapper. When a slot is dropped without a Tokio context, the pool is forgotten instead of dropped, so pool shutdown (and any dropping of pooled connections) does not run without a runtime.

3. **Python `__del__`** (`rapsqlite/__init__.py`)  
   A best-effort `__del__` schedules `close()` on the running event loop when possible, so connections are more likely to be closed under Tokio when the loop is still active.

4. **Documentation**  
   Users are encouraged to use `async with connect(...) as conn:` or explicit `await conn.close()` so that cleanup runs under Tokio.

## Current status

- **Single-process / no workers**: The minimal repro (`scripts/repro_tokio_panic.py`) and normal single-process usage are generally stable with the above mitigations.
- **Parallel workers**: Running the full test suite with parallel workers (e.g. `pytest tests/ -n 10`) can still show the same panic **after** all tests have passed, during worker or process shutdown. So far, every attempt has been to ensure no `PoolConnection` or pool is dropped without Tokio; the remaining panic suggests at least one code path or drop order still hits sqlx’s `PoolConnection::Drop` without a current runtime (exact source not yet pinned down).

## How to reproduce

- **Minimal (no close, then GC):**
  ```bash
  python scripts/repro_tokio_panic.py
  ```
- **Full suite with workers (panic often at exit):**
  ```bash
  python -m pytest tests/ -n 10
  ```
  Check the very end of the output for the panic message after the test summary.

## References

- sqlx `PoolConnection` Drop: [connection.rs](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-core/src/pool/connection.rs) (around lines 207–220).
- rapsqlite implementation: `src/pool.rs` (`PoolConnectionSlot`, `PoolSlot`), `src/connection/mod.rs` (slots and temporaries).
- Related investigation notes: `docs/reference/tokio-panic-investigation.md`.
- Minimal repro script: `scripts/repro_tokio_panic.py`.
