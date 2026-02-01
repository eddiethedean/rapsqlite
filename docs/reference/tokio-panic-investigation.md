# Tokio Panic Investigation

## Summary

If a `Connection` is dropped without calling `close()` (e.g. abandoned or GC'd after a test timeout), Python's garbage collector can drop the Rust `Connection` and its sqlx pool/connection handles. sqlx's `PoolConnection::Drop` calls `crate::rt::spawn()`, which **requires a current Tokio runtime**. That runtime is only active while a Rust future is being polled (e.g. when Python awaits a rapsqlite method). During GC there is typically no Tokio context, so the spawn panics: **"this functionality requires a Tokio context"**.

## Reproducing

Run the minimal repro (no close, then GC):

```bash
python scripts/repro_tokio_panic.py
```

You should see the panic (or a `PytestUnraisableExceptionWarning` wrapping it) during `gc.collect()`.

## Root Cause (Step 2)

- **Location**: sqlx-core 0.8.6, `src/pool/connection.rs`
- **Call site**: `impl Drop for PoolConnection<DB>` (lines 207–220). The panic is reported at line 208 (start of the `Drop` impl); the actual failing call is inside it.
- **Tokio API**: `crate::rt::spawn(...)`. The Drop implementation:
  - If `close_on_drop` is true: spawns `take_and_close()` (which uses `crate::rt::timeout`).
  - Otherwise, if there is a live connection or `min_connections > 0`: spawns `return_to_pool()`.
- **Why it panics**: `tokio::spawn` (exposed as `crate::rt::spawn` in sqlx) requires a current Tokio runtime. When the `Connection` is dropped from Python GC, Drop runs in a context where no Tokio runtime is current (see below).

Relevant snippet from sqlx:

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

## When Tokio Is Available (Step 3)

- **pyo3-async-runtimes (tokio-runtime)**: The Tokio runtime is active when Rust async code is being driven by the bridge (e.g. when Python awaits a future produced by `future_into_py`). So during `await conn.execute(...)` or `await conn.close()`, Tokio is current.
- **GC**: Python's garbage collector can run at various times (e.g. when the interpreter collects cycles or during shutdown). GC may run on the same thread as the asyncio loop or on another thread; in either case, **no Rust future is being polled**, so the Tokio runtime is not necessarily current. Dropping the PyO3 `Connection` during GC therefore runs `PoolConnection::Drop` without a Tokio context.

## Mitigation (Step 4)

1. **Always close under Tokio (Option A)**  
   Use `async with connect(...) as conn:` or explicitly `await conn.close()` so that the pool is closed and connections are released from within async code, where Tokio is active. Do not rely on GC to clean up connections.

2. **Document (Option D)**  
   Document that abandoning a connection without `close()` can cause a panic during GC. See the "Resource cleanup" section in the advanced usage guide.

3. **Best-effort `__del__` (Option B)**  
   The Python wrapper provides a `__del__` that schedules `close()` on the running event loop if one exists. This is best-effort only (no guarantees about loop lifetime or finalizer order) but can prevent the panic when GC runs while the asyncio loop is still running (e.g. `repro_tokio_panic.py` no longer panics). Implemented in `rapsqlite/__init__.py`.

## References

- sqlx `PoolConnection` Drop: [connection.rs](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-core/src/pool/connection.rs) (impl Drop around lines 207–220).
- rapsqlite comment on GC vs Tokio: `src/connection.rs` lines 100–113.
- Minimal repro: `scripts/repro_tokio_panic.py`.
