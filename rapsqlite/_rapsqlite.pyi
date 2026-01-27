"""Type stubs for _rapsqlite Rust extension module."""

from typing import Any, Callable, Coroutine, Dict, List, Optional, Type

# Type alias for init_hook callback
InitHook = Callable[["Connection"], Coroutine[Any, Any, None]]

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

    def __new__(
        cls,
        path: str,
        *,
        pragmas: Optional[Dict[str, Any]] = None,
        init_hook: Optional[InitHook] = None,
    ) -> "Connection":
        """Create a new async SQLite connection.
        
        Args:
            path: Path to SQLite database file
            pragmas: Optional dict of PRAGMA settings
            init_hook: Optional async callable that receives Connection and runs initialization code.
                The hook is called once when the connection pool is first used.
                Example: async def init_hook(conn): await conn.execute("CREATE TABLE ...")
                
        Note:
            init_hook is a rapsqlite-specific enhancement and is not available in aiosqlite.
            This feature provides automatic database initialization capabilities beyond
            standard aiosqlite functionality.
        """
        ...
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
    ) -> Coroutine[Any, Any, "Cursor"]: ...
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
    
    def get_tables(
        self, name: Optional[str] = None
    ) -> Coroutine[Any, Any, List[str]]:
        """Get list of table names in the database.
        
        Args:
            name: Optional table name filter. If provided, returns only that table if it exists.
        
        Returns:
            List of table names (strings), excluding system tables (sqlite_*).
        """
        ...
    
    def get_table_info(
        self, table_name: str
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get table information (columns) for a specific table.
        
        Args:
            table_name: Name of the table to get information for.
        
        Returns:
            List of dictionaries with column metadata:
            - cid: Column ID
            - name: Column name
            - type: Column type
            - notnull: Not null constraint (0 or 1)
            - dflt_value: Default value (can be None)
            - pk: Primary key (0 or 1)
        
        Raises:
            OperationalError: If table does not exist.
        """
        ...
    
    def get_indexes(
        self, table_name: Optional[str] = None
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get list of indexes in the database.
        
        Args:
            table_name: Optional table name filter. If provided, returns only indexes for that table.
        
        Returns:
            List of dictionaries with index information:
            - name: Index name
            - table: Table name
            - unique: Whether index is unique (0 or 1)
            - sql: CREATE INDEX SQL statement (can be None)
        """
        ...
    
    def get_foreign_keys(
        self, table_name: str
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get foreign key constraints for a specific table.
        
        Args:
            table_name: Name of the table to get foreign keys for.
        
        Returns:
            List of dictionaries with foreign key information:
            - id: Foreign key ID
            - seq: Sequence number
            - table: Referenced table name
            - from: Column in current table
            - to: Column in referenced table
            - on_update: ON UPDATE action
            - on_delete: ON DELETE action
            - match: MATCH clause
        """
        ...
    
    def get_schema(
        self, table_name: Optional[str] = None
    ) -> Coroutine[Any, Any, Dict[str, Any]]:
        """Get comprehensive schema information for a table or all tables.
        
        Args:
            table_name: Optional table name. If provided, returns detailed info for that table.
                If None, returns list of all tables.
        
        Returns:
            Dictionary with schema information:
            - If table_name provided: columns, indexes, foreign_keys, table_name
            - If table_name is None: tables (list of table names)
        """
        ...
    
    def get_views(
        self, name: Optional[str] = None
    ) -> Coroutine[Any, Any, List[str]]:
        """Get list of view names in the database.
        
        Args:
            name: Optional view name filter. If provided, returns only that view if it exists.
        
        Returns:
            List of view names (strings).
        """
        ...
    
    def get_index_list(
        self, table_name: str
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get list of indexes for a specific table using PRAGMA index_list.
        
        Args:
            table_name: Name of the table to get indexes for.
        
        Returns:
            List of dictionaries with index list information:
            - seq: Sequence number
            - name: Index name
            - unique: Whether index is unique (0 or 1)
            - origin: Origin of index (c=CREATE, u=UNIQUE, pk=PRIMARY KEY, or None)
            - partial: Whether index is partial (0 or 1)
        """
        ...
    
    def get_index_info(
        self, index_name: str
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get information about columns in an index using PRAGMA index_info.
        
        Args:
            index_name: Name of the index to get information for.
        
        Returns:
            List of dictionaries with index column information:
            - seqno: Sequence number in index
            - cid: Column ID in table
            - name: Column name
        """
        ...
    
    def get_table_xinfo(
        self, table_name: str
    ) -> Coroutine[Any, Any, List[Dict[str, Any]]]:
        """Get extended table information using PRAGMA table_xinfo (SQLite 3.26.0+).
        
        Returns additional information beyond table_info, including hidden columns.
        
        Args:
            table_name: Name of the table to get extended information for.
        
        Returns:
            List of dictionaries with extended column metadata:
            - cid: Column ID
            - name: Column name
            - type: Column type
            - notnull: Not null constraint (0 or 1)
            - dflt_value: Default value (can be None)
            - pk: Primary key (0 or 1)
            - hidden: Hidden column flag (0=normal, 1=hidden, 2=virtual, 3=stored)
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
