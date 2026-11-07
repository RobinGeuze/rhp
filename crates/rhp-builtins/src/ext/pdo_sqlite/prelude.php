namespace Pdo {
    class Sqlite extends \PDO
    {
        public const int DETERMINISTIC = 2048;
        public const int OPEN_READONLY = 1;
        public const int OPEN_READWRITE = 2;
        public const int OPEN_CREATE = 4;
        public const int ATTR_OPEN_FLAGS = 1000;
        public const int ATTR_READONLY_STATEMENT = 1001;
        public const int ATTR_EXTENDED_RESULT_CODES = 1002;
        public const int ATTR_BUSY_STATEMENT = 1003;
        public const int ATTR_EXPLAIN_STATEMENT = 1004;
        public const int ATTR_TRANSACTION_MODE = 1005;
        public const int TRANSACTION_MODE_DEFERRED = 0;
        public const int TRANSACTION_MODE_IMMEDIATE = 1;
        public const int TRANSACTION_MODE_EXCLUSIVE = 2;
        public const int EXPLAIN_MODE_PREPARED = 0;
        public const int EXPLAIN_MODE_EXPLAIN = 1;
        public const int EXPLAIN_MODE_EXPLAIN_QUERY_PLAN = 2;
        public const int OK = 0;
        public const int DENY = 1;
        public const int IGNORE = 2;

        public function createAggregate(string $name, $step, $finalize, int $numArgs = -1): bool { return __pdo_sqlite('createAggregate', $this, $name, $step, $finalize, $numArgs); }
        public function createCollation(string $name, $callback): bool { return __pdo_sqlite('createCollation', $this, $name, $callback); }
        public function createFunction(string $function_name, $callback, int $num_args = -1, int $flags = 0): bool { return __pdo_sqlite('createFunction', $this, $function_name, $callback, $num_args, $flags); }
        public function loadExtension(string $name): void { __pdo_sqlite('loadExtension', $this, $name); }
        public function openBlob(string $table, string $column, int $rowid, ?string $dbname = "main", int $flags = \Pdo\Sqlite::OPEN_READONLY) { return __pdo_sqlite('openBlob', $this, $table, $column, $rowid, $dbname, $flags); }
        public function setAuthorizer($callback): void { __pdo_sqlite('setAuthorizer', $this, $callback); }
    }
}
