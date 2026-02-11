from __future__ import annotations

import asyncio
from typing import Any


async def transaction_retry(
    conn: Any,
    work: Any,
    max_retries: int = 5,
    initial_delay: float = 0.01,
    max_delay: float = 1.0,
) -> Any:
    """Run a transaction with retry on transient errors (e.g. SQLITE_BUSY, SQLITE_LOCKED).

    ``work`` is a callable that returns an awaitable (e.g. an async function); it is
    invoked once per attempt so each retry runs fresh. Retries with exponential backoff.

    Example:
        async with connect("app.db") as conn:
            async def do_work():
                await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
            await transaction_retry(conn, do_work, max_retries=3)
    """
    last_err: Exception | None = None
    delay = initial_delay
    for attempt in range(max_retries):
        try:
            await conn.begin()
            try:
                coro = work() if callable(work) else work
                result = await coro
                await conn.commit()
                return result
            except Exception as e:  # noqa: PERF203 - explicit rollback path
                await conn.rollback()
                last_err = e
                msg = str(e).lower()
                if "busy" in msg or "locked" in msg:
                    if attempt < max_retries - 1:
                        await asyncio.sleep(min(delay, max_delay))
                        delay = min(delay * 2, max_delay)
                        continue
                raise
        except Exception as e:
            last_err = e
            raise
    # If max_retries is 0, we never enter the loop, so raise the last error or a helpful message.
    if last_err is not None:
        raise last_err
    raise RuntimeError("transaction_retry: max_retries must be at least 1")


async def transaction_with_timeout(
    conn: Any,
    work: Any,
    timeout_secs: float = 30.0,
) -> Any:
    """Run a transaction with a timeout.

    Wraps the transaction body in asyncio.wait_for. Raises asyncio.TimeoutError
    if the transaction (including work) exceeds timeout_secs.
    """

    async def _run() -> Any:
        async with conn.transaction():
            coro = work() if callable(work) else work
            return await coro

    return await asyncio.wait_for(_run(), timeout=timeout_secs)
