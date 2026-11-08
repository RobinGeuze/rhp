class PDOException extends RuntimeException
{
    protected $code = 0;
    public ?array $errorInfo = null;
}

class PDO
{
    public const int PARAM_NULL = 0;
    public const int PARAM_BOOL = 5;
    public const int PARAM_INT = 1;
    public const int PARAM_STR = 2;
    public const int PARAM_LOB = 3;
    public const int PARAM_STMT = 4;
    public const int PARAM_INPUT_OUTPUT = 2147483648;
    public const int PARAM_STR_NATL = 1073741824;
    public const int PARAM_STR_CHAR = 536870912;
    public const int PARAM_EVT_ALLOC = 0;
    public const int PARAM_EVT_FREE = 1;
    public const int PARAM_EVT_EXEC_PRE = 2;
    public const int PARAM_EVT_EXEC_POST = 3;
    public const int PARAM_EVT_FETCH_PRE = 4;
    public const int PARAM_EVT_FETCH_POST = 5;
    public const int PARAM_EVT_NORMALIZE = 6;
    public const int FETCH_DEFAULT = 0;
    public const int FETCH_LAZY = 1;
    public const int FETCH_ASSOC = 2;
    public const int FETCH_NUM = 3;
    public const int FETCH_BOTH = 4;
    public const int FETCH_OBJ = 5;
    public const int FETCH_BOUND = 6;
    public const int FETCH_COLUMN = 7;
    public const int FETCH_CLASS = 8;
    public const int FETCH_INTO = 9;
    public const int FETCH_FUNC = 10;
    public const int FETCH_GROUP = 32;
    public const int FETCH_UNIQUE = 64;
    public const int FETCH_KEY_PAIR = 12;
    public const int FETCH_CLASSTYPE = 128;
    public const int FETCH_SERIALIZE = 512;
    public const int FETCH_PROPS_LATE = 256;
    public const int FETCH_NAMED = 11;
    public const int ATTR_AUTOCOMMIT = 0;
    public const int ATTR_PREFETCH = 1;
    public const int ATTR_TIMEOUT = 2;
    public const int ATTR_ERRMODE = 3;
    public const int ATTR_SERVER_VERSION = 4;
    public const int ATTR_CLIENT_VERSION = 5;
    public const int ATTR_SERVER_INFO = 6;
    public const int ATTR_CONNECTION_STATUS = 7;
    public const int ATTR_CASE = 8;
    public const int ATTR_CURSOR_NAME = 9;
    public const int ATTR_CURSOR = 10;
    public const int ATTR_ORACLE_NULLS = 11;
    public const int ATTR_PERSISTENT = 12;
    public const int ATTR_STATEMENT_CLASS = 13;
    public const int ATTR_FETCH_TABLE_NAMES = 14;
    public const int ATTR_FETCH_CATALOG_NAMES = 15;
    public const int ATTR_DRIVER_NAME = 16;
    public const int ATTR_STRINGIFY_FETCHES = 17;
    public const int ATTR_MAX_COLUMN_LEN = 18;
    public const int ATTR_EMULATE_PREPARES = 20;
    public const int ATTR_DEFAULT_FETCH_MODE = 19;
    public const int ATTR_DEFAULT_STR_PARAM = 21;
    public const int ERRMODE_SILENT = 0;
    public const int ERRMODE_WARNING = 1;
    public const int ERRMODE_EXCEPTION = 2;
    public const int CASE_NATURAL = 0;
    public const int CASE_LOWER = 2;
    public const int CASE_UPPER = 1;
    public const int NULL_NATURAL = 0;
    public const int NULL_EMPTY_STRING = 1;
    public const int NULL_TO_STRING = 2;
    public const string ERR_NONE = '00000';
    public const int FETCH_ORI_NEXT = 0;
    public const int FETCH_ORI_PRIOR = 1;
    public const int FETCH_ORI_FIRST = 2;
    public const int FETCH_ORI_LAST = 3;
    public const int FETCH_ORI_ABS = 4;
    public const int FETCH_ORI_REL = 5;
    public const int CURSOR_FWDONLY = 0;
    public const int CURSOR_SCROLL = 1;
    public const int SQLITE_DETERMINISTIC = 2048;
    public const int SQLITE_ATTR_OPEN_FLAGS = 1000;
    public const int SQLITE_OPEN_READONLY = 1;
    public const int SQLITE_OPEN_READWRITE = 2;
    public const int SQLITE_OPEN_CREATE = 4;
    public const int SQLITE_ATTR_READONLY_STATEMENT = 1001;
    public const int SQLITE_ATTR_EXTENDED_RESULT_CODES = 1002;
    public function __construct(string $dsn, ?string $username = null, #[\SensitiveParameter] ?string $password = null, ?array $options = null) { __pdo('__construct', $this, $dsn, $username, $password, $options); }
    public static function connect(string $dsn, ?string $username = null, #[\SensitiveParameter] ?string $password = null, ?array $options = null): static { return __pdo('connect', static::class, $dsn, $username, $password, $options); }
    public function beginTransaction() { return __pdo('beginTransaction', $this); }
    public function commit() { return __pdo('commit', $this); }
    public function errorCode() { return __pdo('errorCode', $this); }
    public function errorInfo() { return __pdo('errorInfo', $this); }
    public function exec(string $statement) { return __pdo('exec', $this, $statement); }
    public function getAttribute(int $attribute) { return __pdo('getAttribute', $this, $attribute); }
    public static function getAvailableDrivers() { return __pdo('getAvailableDrivers', static::class); }
    public function inTransaction() { return __pdo('inTransaction', $this); }
    public function lastInsertId(?string $name = null) { return __pdo('lastInsertId', $this, $name); }
    public function prepare(string $query, array $options = []) { return __pdo('prepare', $this, $query, $options); }
    public function query(string $query, ?int $fetchMode = null, mixed ...$fetchModeArgs) { return __pdo('query', $this, func_num_args(), $query, $fetchMode, ...$fetchModeArgs); }
    public function quote(string $string, int $type = PDO::PARAM_STR) { return __pdo('quote', $this, $string, $type); }
    public function rollBack() { return __pdo('rollBack', $this); }
    public function setAttribute(int $attribute, mixed $value) { return __pdo('setAttribute', $this, $attribute, $value); }
    // pdo_sqlite's driver methods, reached through the get_method handler.
    public function __sqliteCreateFunction(...$args) { return __pdo_sqlite('sqliteCreateFunction', $this, ...$args); }
    public function __sqliteCreateAggregate(...$args) { return __pdo_sqlite('sqliteCreateAggregate', $this, ...$args); }
    public function __sqliteCreateCollation(...$args) { return __pdo_sqlite('sqliteCreateCollation', $this, ...$args); }
}

class PDOStatement implements IteratorAggregate
{
    public string $queryString;

    public function bindColumn(string|int $column, mixed &$var, int $type = PDO::PARAM_STR, int $maxLength = 0, mixed $driverOptions = null) { return __pdo_stmt('bindColumn', $this, $column, [&$var], $type, $maxLength, $driverOptions); }
    public function bindParam(string|int $param, mixed &$var, int $type = PDO::PARAM_STR, int $maxLength = 0, mixed $driverOptions = null) { return __pdo_stmt('bindParam', $this, $param, [&$var], $type, $maxLength, $driverOptions); }
    public function bindValue(string|int $param, mixed $value, int $type = PDO::PARAM_STR) { return __pdo_stmt('bindValue', $this, $param, $value, $type); }
    public function closeCursor() { return __pdo_stmt('closeCursor', $this); }
    public function columnCount() { return __pdo_stmt('columnCount', $this); }
    public function debugDumpParams() { return __pdo_stmt('debugDumpParams', $this); }
    public function errorCode() { return __pdo_stmt('errorCode', $this); }
    public function errorInfo() { return __pdo_stmt('errorInfo', $this); }
    public function execute(?array $params = null) { return __pdo_stmt('execute', $this, $params); }
    public function fetch(int $mode = PDO::FETCH_DEFAULT, int $cursorOrientation = PDO::FETCH_ORI_NEXT, int $cursorOffset = 0) { return __pdo_stmt('fetch', $this, $mode, $cursorOrientation, $cursorOffset); }
    public function fetchAll(int $mode = PDO::FETCH_DEFAULT, mixed ...$args) { return __pdo_stmt('fetchAll', $this, func_num_args(), $mode, ...$args); }
    public function fetchColumn(int $column = 0) { return __pdo_stmt('fetchColumn', $this, $column); }
    public function fetchObject(?string $class = "stdClass", array $constructorArgs = []) { return __pdo_stmt('fetchObject', $this, $class, $constructorArgs); }
    public function getAttribute(int $name) { return __pdo_stmt('getAttribute', $this, $name); }
    public function getColumnMeta(int $column) { return __pdo_stmt('getColumnMeta', $this, $column); }
    public function nextRowset() { return __pdo_stmt('nextRowset', $this); }
    public function rowCount() { return __pdo_stmt('rowCount', $this); }
    public function setAttribute(int $attribute, mixed $value) { return __pdo_stmt('setAttribute', $this, $attribute, $value); }
    public function setFetchMode(int $mode, mixed ...$args) { return __pdo_stmt('setFetchMode', $this, func_num_args(), $mode, ...$args); }
    public function getIterator(): Iterator { return __pdo_stmt('getIterator', $this); }
}

final class PDORow
{
    public function __construct() { __pdo_row(4, $this, null); }
    public function __get($name) { return __pdo_row(0, $this, $name); }
    public function __set($name, $value) { __pdo_row(1, $this, $name); }
    public function __isset($name) { return __pdo_row(2, $this, $name); }
    public function __unset($name) { __pdo_row(3, $this, $name); }
}
