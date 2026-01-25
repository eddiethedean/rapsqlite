# Plan: Option A — Phase 2.7 Advanced SQLite Callbacks

## Goal

Implement the five advanced SQLite callback/hook APIs so that aiosqlite-compat tests pass:

1. **`enable_load_extension(enabled: bool)`** — Enable/disable loading SQLite extensions.
2. **`create_function(name, nargs, func)`** / **`remove_function`** — Register Python callables as SQL UDFs; `func=None` removes.
3. **`set_trace_callback(callback)`** — Invoke a Python callable with each SQL statement executed; `None` to clear.
4. **`set_authorizer(callback)`** — Invoke a Python callable for each authorization check; `None` to clear.
5. **`set_progress_handler(n, callback)`** — Invoke a Python callable every N SQLite VDBE ops; `None` to clear.

These are **per-connection** SQLite C API features. rapsqlite uses **sqlx** (pool + async query execution). Callbacks require access to the raw `sqlite3*` connection and C APIs (`sqlite3_create_function`, `sqlite3_trace_v2`, `sqlite3_set_authorizer`, `sqlite3_progress_handler`, `sqlite3_enable_load_extension`). Today we do not expose these.

## Test Coverage (Compat)

| Feature | Representative tests | Notes |
|--------|----------------------|-------|
| `enable_load_extension` | `test_enable_load_extension`, `test_enable_load_extension_comprehensive` | Enable/disable only; no actual extension load in tests. |
| `create_function` / remove | `test_create_function`, `test_create_function_remove`, `test_create_function_multiple_args`, `test_create_function_type_conversions`, `test_create_function_with_table_data`, `test_create_function_error_handling`, `test_create_function_overwrite`, `test_create_function_multiple_functions`, `test_custom_function_*` | 0–1 args, return int/float/str/bytes/None, overwrite, remove via `create_function(name, n, None)`. |
| `set_trace_callback` | `test_set_trace_callback`, `test_set_trace_callback_comprehensive`, `test_set_trace_callback_transactions`, `test_set_trace_callback_remove`, `test_trace_callback_with_errors`, `test_custom_function_with_trace` | Callback(`sql`); `None` to clear. |
| `set_authorizer` | `test_set_authorizer_deny_drops`, `test_set_authorizer_deny_specific_operation`, `test_set_authorizer_comprehensive`, `test_authorizer_action_codes`, `test_custom_function_with_authorizer` | Callback(`action`, `arg1`, `arg2`, `arg3`, `arg4`) → `0` (OK), `2` (DENY), etc. |
| `set_progress_handler` | `test_set_progress_handler`, `test_set_progress_handler_comprehensive`, `test_set_progress_handler_interrupt`, `test_all_callbacks_together` | `(n, callback)`; callback() → `True` continue, `False` abort; `None` to clear. |

**Combo:** `test_all_callbacks_together` uses trace + authorizer + progress + create_function; then disables all.

Implementing these unblocks **~25+** currently failing compat tests (all `AttributeError: '…Connection' has no attribute 'create_function'` etc.).

## API Summary (Target)

- **`enable_load_extension(enabled: bool) -> Awaitable[None]`**  
  No return value. Wraps `sqlite3_enable_load_extension(db, enabled)`.

- **`create_function(name: str, nargs: int, func: Optional[Callable]) -> Awaitable[None]`**  
  `func is None` → remove. Otherwise register UDF with `nargs` args (-1 = variadic). Callable receives Python values; returns Python value (mapped to SQLite type).

- **`set_trace_callback(callback: Optional[Callable[[str], None]]) -> Awaitable[None]`**  
  Callable receives SQL string. `None` clears.

- **`set_authorizer(callback: Optional[Callable]) -> Awaitable[None]`**  
  Signature `(action, arg1, arg2, arg3, arg4) -> int` (e.g. `SQLITE_OK`, `SQLITE_DENY`). `None` clears.

- **`set_progress_handler(n: int, callback: Optional[Callable[[], bool]]) -> Awaitable[None]`**  
  Called every `n` VDBE ops. Return `True` to continue, `False` to abort. `None` clears.

All must run on the **same connection** used for execute/fetch when we have a transaction or reserved connection. With **pool_size > 1**, we must define semantics: either “callbacks apply only to the connection used for the next op” or “we apply callbacks to all pool connections.” Pool-size 1 (default) avoids ambiguity.

## Architecture Constraints

- **sqlx** provides `SqlitePool`, `PoolConnection`, and `sqlx::query::*`. It does **not** expose `sqlite3*` or C-level hooks.
- **libsqlite3-sys** is already a dependency; we have raw C API access.
- **rusqlite** exposes `create_function`, `trace`, `authorizer`, etc., but is synchronous and separate from sqlx’s async pool.

**Implications:** We cannot implement these callbacks by “configuring sqlx” alone. We need at least one of:

1. **Raw connection path**  
   Use `libsqlite3-sys` to open/manage our own `sqlite3*` connections, run SQL through them, and install callbacks. Async could be handled via `spawn_blocking` or a dedicated async wrapper. This path would exist **alongside** sqlx (e.g. when callbacks are in use we use raw connections; otherwise keep using the pool). Significant design and refactor.

2. **Extract `sqlite3*` from sqlx**  
   If sqlx or its dependencies expose the underlying `sqlite3*` handle for a connection, we could install callbacks there and keep using sqlx for execution. Needs investigation (sqlx version, backend, etc.).

3. **Rusqlite + `spawn_blocking`**  
   Use rusqlite for a “callback-capable” connection, run all DB work for that connection on a thread pool via `spawn_blocking`, and bridge to async Python. Differs from current “pure sqlx + Tokio” design but would reuse rusqlite’s callback support.

4. **Hybrid**  
   Keep pool for “no-callback” usage; when user sets any callback, use a dedicated raw/rusqlite connection for that logical “connection” and route execute/fetch to it. Increases branching and complexity.

## Recommended Implementation Order

1. **Research**  
   - Confirm whether sqlx (or underlying driver) exposes `sqlite3*` for a connection.  
   - Decide between raw `libsqlite3-sys` vs rusqlite vs hybrid.

2. **`enable_load_extension`**  
   Simplest: single C call, no Python callbacks. Good first step to validate “we can touch connection-level settings.”

3. **`create_function` / `remove_function`**  
   Most compat value. Requires Python ↔ SQLite value marshalling and UDF trampolines. rusqlite has patterns we can mirror.

4. **`set_trace_callback`**  
   Single callback, simple signature. Good next step after create_function.

5. **`set_authorizer`**  
   More complex (action codes, multiple args). Implement after trace.

6. **`set_progress_handler`**  
   Can abort long-running work. Implement last among the five.

## Key Code Locations

- **Connection:** `src/lib.rs` — `Connection` struct, `Connection::new`, existing async methods. Callback state (e.g. `Arc<Mutex<...>>` for each hook) would live here or in a shared “connection config” used by the execution path.
- **Pool vs raw:** `get_or_create_pool`, `bind_and_execute`, `bind_and_fetch_*`, execute/fetch in transaction. These currently use sqlx only; any callback-aware path must route to the raw/rusqlite connection when appropriate.
- **Type stubs:** `rapsqlite/_rapsqlite.pyi` — add signatures for the five APIs.

## Testing Strategy

- **Unit:** For each API, minimal Rust-side tests (or Python tests that invoke the API and then run a single execute/fetch) to verify enable/disable, set/clear, and basic behavior.
- **Compat:** Run the relevant `test_aiosqlite_compat` tests listed above. Fix any assertion or behavioral gaps.
- **Regression:** Full `test_rapsqlite` and `test_pool_config` / `test_row_factory` suites whenever we touch shared paths.

## Out of Scope (This Plan)

- **iterdump** (Phase 2.8+): Separate feature; see ROADMAP.
- **backup**: Not in scope.
- **Nested transactions / savepoints**: Unrelated to callbacks.

## Success Criteria

- All five APIs implemented and wired to the correct execution path.
- `test_enable_load_extension*`, `test_create_function*`, `test_set_trace_callback*`, `test_set_authorizer*`, `test_set_progress_handler*`, `test_custom_function_*`, `test_authorizer_action_codes`, `test_all_callbacks_together`, and `test_trace_callback_with_errors` pass.
- No regressions in `test_rapsqlite`, `test_pool_config`, `test_row_factory`, or other compat tests.

## References

- [SQLite C API: create_function](https://www.sqlite.org/c3ref/create_function.html), [trace](https://www.sqlite.org/c3ref/trace_v2.html), [authorizer](https://www.sqlite.org/c3ref/set_authorizer.html), [progress_handler](https://www.sqlite.org/c3ref/progress_handler.html), [enable_load_extension](https://www.sqlite.org/c3ref/load_extension.html)
- [aiosqlite API](https://aiosqlite.omnilib.dev/en/stable/api.html) (create_function, enable_load_extension)
- Python `sqlite3`: `create_function`, `set_trace_callback`, `set_authorizer`, `set_progress_handler`
- `docs/ROADMAP.md` — Phase 2.7
- `tests/test_aiosqlite_compat.py` — all callback-related tests
