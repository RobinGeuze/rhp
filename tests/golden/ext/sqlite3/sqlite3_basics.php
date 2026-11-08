<?php
// SQLite3, SQLite3Stmt and SQLite3Result on an in-memory database.
class Store
{
    public SQLite3 $db;

    public function __construct()
    {
        $this->db = new SQLite3(':memory:');
        // A private method as callback: resolved in this scope when
        // registered, called later from sqlite.
        $this->db->createFunction('twice', [$this, 'twice'], 1, SQLITE3_DETERMINISTIC);
        $this->db->createCollation('rev', [$this, 'rev']);
    }

    private function twice($v) { return $v * 2; }
    private function rev($a, $b) { return strcmp($b, $a); }
}

$s = new Store();
$db = $s->db;
var_dump($db->exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, n INTEGER, f REAL, b BLOB)'));
$st = $db->prepare('INSERT INTO t (name, n, f, b) VALUES (:name, :n, :f, :b)');
var_dump($st->paramCount(), $st->readOnly());
foreach ([['a', 1, 1.5], ['b', 2, 2.5], ['c', 3, null]] as [$name, $n, $f]) {
    $st->bindValue(':name', $name, SQLITE3_TEXT);
    $st->bindParam(':n', $n, SQLITE3_INTEGER);
    $st->bindValue(':f', $f, $f === null ? SQLITE3_NULL : SQLITE3_FLOAT);
    $st->bindValue(':b', "x\0y", SQLITE3_BLOB);
    $r = $st->execute();
    var_dump($r instanceof SQLite3Result, $db->changes(), $db->lastInsertRowID());
    $st->reset();
}
var_dump($st->getSQL(), $st->close());

$r = $db->query('SELECT id, name, twice(n) AS t2, f, b FROM t ORDER BY name COLLATE rev');
var_dump($r->numColumns(), $r->columnName(2), $r->columnName(9));
while ($row = $r->fetchArray(SQLITE3_ASSOC)) {
    $row['b'] = bin2hex($row['b']);
    var_dump($row);
}
$r->reset();
$row = $r->fetchArray(SQLITE3_NUM);
$row[4] = bin2hex($row[4]);
var_dump($row);
var_dump($r->columnType(3), $r->finalize());

var_dump($db->querySingle('SELECT COUNT(*) FROM t'));
$row = $db->querySingle('SELECT * FROM t WHERE id = 2', true);
$row['b'] = bin2hex($row['b']);
var_dump($row);
var_dump($db->querySingle('SELECT * FROM t WHERE id = 99'));
var_dump(SQLite3::escapeString("it's"));

// Errors: warnings by default, exceptions once enabled.
var_dump(@$db->exec('SELECT * FROM missing'), $db->lastErrorCode(), $db->lastErrorMsg());
$db->enableExceptions(true);
try {
    $db->prepare('SELECT * FROM missing');
} catch (SQLite3Exception $e) {
    echo get_class($e), ': ', $e->getMessage(), ' (', $e->getCode(), ")\n";
}
var_dump($db->close());
try {
    $db->exec('SELECT 1');
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
