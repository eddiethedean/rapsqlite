"""Type stubs for _rapsqlite Rust extension module."""

from typing import Any, Callable, Coroutine, List, Optional, Type

class Error(Exception):
    """Base exception class for rapsqlite errors."""
    def __init__(self, message: str) -> None: ...

class Warning(Exception):
    """Warning exception class."""
    def __init__(self, message: str) -> None: ...

class DatabaseError(Error):
    """Base exception class for database-related errors."""
    def __init__(self, message: str) -> None: ...

class OperationalError(DatabaseError):
    """Exception raised for operational errors."""
    def __init__(self, message: str) -> None: ...

class ProgrammingError(DatabaseError):
    """Exception raised for programming errors."""
    def __init__(self, message: str) -> None: ...

class IntegrityError(DatabaseError):
    """Exception raised for integrity constraint violations."""
    def __init__(self, message: str) -> None: ...

class Connection:
    """Async SQLite connection."""

    def __init__(self, path: str) -> None: ...
    def __aenter__(self) -> "Connection": ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_val: Optional[BaseException],
        exc_tb: Optional[Any],
    ) -> Coroutine[Any, Any, Optional[bool]]: ...
    def close(self) -> Coroutine[Any, Any, None]: ...
    def begin(self) -> Coroutine[Any, Any, None]: ...
    def commit(self) -> Coroutine[Any, Any, None]: ...
    def rollback(self) -> Coroutine[Any, Any, None]: ...
    def execute(
        self, query: str, parameters: Optional[Any] = None
    ) -> Coroutine[Any, Any, None]: ...
    def execute_many(
        self, query: str, parameters: List[List[Any]]
    ) -> Coroutine[Any, Any, None]: ...
    def fetch_all(
        self, query: str, parameters: Optional[Any] = None
    ) -> Coroutine[Any, Any, List[Any]]: ...
    def fetch_one(
        self, query: str, parameters: Optional[Any] = None
    ) -> Coroutine[Any, Any, Any]: ...
    def fetch_optional(
        self, query: str, parameters: Optional[Any] = None
    ) -> Coroutine[Any, Any, Optional[Any]]: ...
    def last_insert_rowid(self) -> Coroutine[Any, Any, int]: ...
    def changes(self) -> Coroutine[Any, Any, int]: ...
    def cursor(self) -> "Cursor": ...
    def transaction(self) -> "TransactionContextManager": ...
    @property
    def row_factory(self) -> Any: ...
    @row_factory.setter
    def row_factory(self, value: Optional[Any]) -> None: ...
    @property
    def pool_size(self) -> Optional[int]: ...
    @pool_size.setter
    def pool_size(self, value: Optional[int]) -> None: ...
    @property
    def connection_timeout(self) -> Optional[int]: ...
    @connection_timeout.setter
    def connection_timeout(self, value: Optional[int]) -> None: ...
    def enable_load_extension(self, enabled: bool) -> Coroutine[Any, Any, None]: ...
    def create_function(
        self, name: str, nargs: int, func: Optional[Any]
    ) -> Coroutine[Any, Any, None]: ...
    def set_trace_callback(
        self, callback: Optional[Any]
    ) -> Coroutine[Any, Any, None]: ...
    def set_authorizer(self, callback: Optional[Any]) -> Coroutine[Any, Any, None]: ...
    def set_progress_handler(
        self, n: int, callback: Optional[Any]
    ) -> Coroutine[Any, Any, None]: ...
    def iterdump(self) -> Coroutine[Any, Any, List[str]]: ...
    def backup(
        self,
        target: Any,
        *,
        pages: int = 0,
        progress: Optional[Callable[[int, int, int], None]] = None,
        name: str = "main",
        sleep: float = 0.25,
    ) -> Coroutine[Any, Any, None]:
        """Make a backup of the current database to a target database.
        
        Args:
            target: Target connection for backup. Must be a rapsqlite.Connection.
                Backing up to Python's standard sqlite3.Connection is NOT supported
                and will cause a segmentation fault due to SQLite library instance
                incompatibility.
            pages: Number of pages to copy per step (0 = all pages). Default: 0
            progress: Optional progress callback function receiving (remaining, page_count, pages_copied).
                Default: None
            name: Database name to backup (e.g., "main", "temp"). Default: "main"
            sleep: Sleep duration in seconds between backup steps. Default: 0.25
        
        Raises:
            OperationalError: If backup fails or target connection is invalid
        
        Note:
            The target parameter must be a rapsqlite.Connection. Backing up to
            sqlite3.Connection is not supported due to SQLite library instance
            incompatibility. See README.md for details and workarounds.
        """
        ...

class TransactionContextManager:
    """Async context manager for transactions. Returned by Connection.transaction()."""

    def __aenter__(self) -> Coroutine[Any, Any, "Connection"]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_val: Optional[BaseException],
        exc_tb: Optional[Any],
    ) -> Coroutine[Any, Any, None]: ...

class Cursor:
    """Cursor for executing queries."""

    def __aenter__(self) -> "Cursor": ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_val: Optional[BaseException],
        exc_tb: Optional[Any],
    ) -> Coroutine[Any, Any, Optional[bool]]: ...
    def execute(
        self, query: str, parameters: Optional[Any] = None
    ) -> Coroutine[Any, Any, None]: ...
    def executemany(
        self, query: str, parameters: List[List[Any]]
    ) -> Coroutine[Any, Any, None]: ...
    def fetchone(self) -> Coroutine[Any, Any, Optional[Any]]: ...
    def fetchall(self) -> Coroutine[Any, Any, List[Any]]: ...
    def fetchmany(
        self, size: Optional[int] = None
    ) -> Coroutine[Any, Any, List[Any]]: ...
