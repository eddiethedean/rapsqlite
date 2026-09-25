from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from contextlib import suppress
from typing import Any, TypeVar

T = TypeVar("T")


async def transaction_retry(
    conn: Any,
    work: Callable[[], Awaitable[T]] | Awaitable[T],
    max_retries: int = 5,
    initial_delay: float = 0.01,
    max_delay: float = 1.0,
) -> T:
    """Run a transaction with retry on transient errors (e.g. SQLITE_BUSY, SQLITE_LOCKED).

    ``work`` is a callable that returns an awaitable (e.g. an async function); it is
    invoked once per attempt so each retry runs fresh. Retries with exponential backoff.

    Example::

        async with connect("app.db") as conn:
            async def do_work():
                await conn.execute("INSERT INTO t (x) VALUES (?)", ["a"])
            await transaction_retry(conn, do_work, max_retries=3)
    """
    last_err: Exception | None = None
    delay = initial_delay
    for attempt in range(max_retries):
        began = False
        try:
            await conn.begin()
            began = True
            coro: Awaitable[T] = work() if callable(work) else work
            result = await coro
            await conn.commit()
            began = False
            return result
        except Exception as e:
            last_err = e
            if began:
                with suppress(Exception):
                    await conn.rollback()
            msg = str(e).lower()
            if ("busy" in msg or "locked" in msg) and attempt < max_retries - 1:
                await asyncio.sleep(min(delay, max_delay))
                delay = min(delay * 2, max_delay)
                continue
            raise
    # If max_retries is 0, we never enter the loop, so raise the last error or a helpful message.
    if last_err is not None:
        raise last_err
    raise RuntimeError("transaction_retry: max_retries must be at least 1")


async def transaction_with_timeout(
    conn: Any,
    work: Callable[[], Awaitable[T]] | Awaitable[T],
    timeout_secs: float = 30.0,
) -> T:
    """Run a transaction with a timeout.

    Wraps the transaction body in asyncio.wait_for. Raises asyncio.TimeoutError
    if the transaction (including work) exceeds timeout_secs.
    """

    async def _run() -> Any:
        async with conn.transaction():
            coro: Awaitable[T] = work() if callable(work) else work
            return await coro

    return await asyncio.wait_for(_run(), timeout=timeout_secs)
