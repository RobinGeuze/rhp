class SQLite3Exception extends Exception
{
}

class SQLite3
{
    public const int OK = 0;
    public const int DENY = 1;
    public const int IGNORE = 2;
    public const int CREATE_INDEX = 1;
    public const int CREATE_TABLE = 2;
    public const int CREATE_TEMP_INDEX = 3;
    public const int CREATE_TEMP_TABLE = 4;
    public const int CREATE_TEMP_TRIGGER = 5;
    public const int CREATE_TEMP_VIEW = 6;
    public const int CREATE_TRIGGER = 7;
    public const int CREATE_VIEW = 8;
    public const int DELETE = 9;
    public const int DROP_INDEX = 10;
    public const int DROP_TABLE = 11;
    public const int DROP_TEMP_INDEX = 12;
    public const int DROP_TEMP_TABLE = 13;
    public const int DROP_TEMP_TRIGGER = 14;
    public const int DROP_TEMP_VIEW = 15;
    public const int DROP_TRIGGER = 16;
    public const int DROP_VIEW = 17;
    public const int INSERT = 18;
    public const int PRAGMA = 19;
    public const int READ = 20;
    public const int SELECT = 21;
    public const int TRANSACTION = 22;
    public const int UPDATE = 23;
    public const int ATTACH = 24;
    public const int DETACH = 25;
    public const int ALTER_TABLE = 26;
    public const int REINDEX = 27;
    public const int ANALYZE = 28;
    public const int CREATE_VTABLE = 29;
    public const int DROP_VTABLE = 30;
    public const int FUNCTION = 31;
    public const int SAVEPOINT = 32;
    public const int COPY = 0;
    public const int RECURSIVE = 33;

    public function __construct(string $filename, int $flags = SQLITE3_OPEN_READWRITE | SQLITE3_OPEN_CREATE, string $encryptionKey = "") { __sqlite3('__construct', $this, $filename, $flags, $encryptionKey); }
    public function open(string $filename, int $flags = SQLITE3_OPEN_READWRITE | SQLITE3_OPEN_CREATE, string $encryptionKey = ""): void { __sqlite3('open', $this, $filename, $flags, $encryptionKey); }
    public function close(): bool { return __sqlite3('close', $this); }
    public static function version(): array { return __sqlite3('version', null); }
    public function lastInsertRowID(): int { return __sqlite3('lastInsertRowID', $this); }
    public function lastErrorCode(): int { return __sqlite3('lastErrorCode', $this); }
    public function lastExtendedErrorCode(): int { return __sqlite3('lastExtendedErrorCode', $this); }
    public function lastErrorMsg(): string { return __sqlite3('lastErrorMsg', $this); }
    public function changes(): int { return __sqlite3('changes', $this); }
    public function busyTimeout(int $milliseconds): bool { return __sqlite3('busyTimeout', $this, $milliseconds); }
    public function loadExtension(string $name): bool { return __sqlite3('loadExtension', $this, $name); }
    public function backup(SQLite3 $destination, string $sourceDatabase = "main", string $destinationDatabase = "main"): bool { return __sqlite3('backup', $this, $destination, $sourceDatabase, $destinationDatabase); }
    public static function escapeString(string $string): string { return __sqlite3('escapeString', null, $string); }
    public function prepare(string $query): SQLite3Stmt|false { return __sqlite3('prepare', $this, $query); }
    public function exec(string $query): bool { return __sqlite3('exec', $this, $query); }
    public function query(string $query): SQLite3Result|false { return __sqlite3('query', $this, $query); }
    public function querySingle(string $query, bool $entireRow = false): mixed { return __sqlite3('querySingle', $this, $query, $entireRow); }
    public function createFunction(string $name, $callback, int $argCount = -1, int $flags = 0): bool { return __sqlite3('createFunction', $this, $name, $callback, $argCount, $flags); }
    public function createAggregate(string $name, $stepCallback, $finalCallback, int $argCount = -1): bool { return __sqlite3('createAggregate', $this, $name, $stepCallback, $finalCallback, $argCount); }
    public function createCollation(string $name, $callback): bool { return __sqlite3('createCollation', $this, $name, $callback); }
    public function openBlob(string $table, string $column, int $rowid, string $database = "main", int $flags = SQLITE3_OPEN_READONLY) { return __sqlite3('openBlob', $this, $table, $column, $rowid, $database, $flags); }
    public function enableExceptions(bool $enable = false): bool { return __sqlite3('enableExceptions', $this, $enable); }
    public function enableExtendedResultCodes(bool $enable = true): bool { return __sqlite3('enableExtendedResultCodes', $this, $enable); }
    public function setAuthorizer($callback): bool { return __sqlite3('setAuthorizer', $this, $callback); }
}

class SQLite3Stmt
{
    public const int EXPLAIN_MODE_PREPARED = 0;
    public const int EXPLAIN_MODE_EXPLAIN = 1;
    public const int EXPLAIN_MODE_EXPLAIN_QUERY_PLAN = 2;

    private function __construct(SQLite3 $sqlite3, string $query) { __sqlite3('Stmt::__construct', $this, $sqlite3, $query); }
    public function bindParam(string|int $param, mixed &$var, int $type = SQLITE3_TEXT): bool { return __sqlite3('Stmt::bindParam', $this, func_num_args(), $param, [&$var], $type); }
    public function bindValue(string|int $param, mixed $value, int $type = SQLITE3_TEXT): bool { return __sqlite3('Stmt::bindValue', $this, func_num_args(), $param, $value, $type); }
    public function clear(): bool { return __sqlite3('Stmt::clear', $this); }
    public function close(): true { return __sqlite3('Stmt::close', $this); }
    public function execute(): SQLite3Result|false { return __sqlite3('Stmt::execute', $this); }
    public function getSQL(bool $expand = false): string|false { return __sqlite3('Stmt::getSQL', $this, $expand); }
    public function paramCount(): int { return __sqlite3('Stmt::paramCount', $this); }
    public function readOnly(): bool { return __sqlite3('Stmt::readOnly', $this); }
    public function reset(): bool { return __sqlite3('Stmt::reset', $this); }
    public function busy(): bool { return __sqlite3('Stmt::busy', $this); }
    public function explain(): int { return __sqlite3('Stmt::explain', $this); }
    public function setExplain(int $mode): bool { return __sqlite3('Stmt::setExplain', $this, $mode); }
}

class SQLite3Result
{
    private function __construct() { __sqlite3('Result::__construct', $this); }
    public function numColumns(): int { return __sqlite3('Result::numColumns', $this); }
    public function columnName(int $column): string|false { return __sqlite3('Result::columnName', $this, $column); }
    public function columnType(int $column): int|false { return __sqlite3('Result::columnType', $this, $column); }
    public function fetchArray(int $mode = SQLITE3_BOTH): array|false { return __sqlite3('Result::fetchArray', $this, $mode); }
    public function fetchAll(int $mode = SQLITE3_BOTH): array|false { return __sqlite3('Result::fetchAll', $this, $mode); }
    public function reset(): bool { return __sqlite3('Result::reset', $this); }
    public function finalize(): true { return __sqlite3('Result::finalize', $this); }
}
