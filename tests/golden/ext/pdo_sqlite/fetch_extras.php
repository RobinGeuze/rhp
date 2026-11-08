<?php
// Fetch paths beyond the common modes: PDORow dimensions, CLASSTYPE's
// default class, SERIALIZE, statement iterators and FETCH_CLASS into a
// built-in class with readonly properties.
$db = new PDO('sqlite::memory:');
$db->exec("CREATE TABLE t (cls TEXT, a INTEGER, b TEXT)");
$db->exec("INSERT INTO t VALUES ('Known', 1, 'x'), (NULL, 2, 'y'), ('Unknown', 3, 'z')");

$row = $db->query('SELECT a, b FROM t')->fetch(PDO::FETCH_LAZY);
var_dump($row[0], $row['b'], $row[7], isset($row[1]), isset($row['nope']), empty($row[0]));
try {
    $row[0] = 5;
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    new PDORow();
} catch (PDOException $e) {
    echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n";
}

class Known { public $a; public $b; }
class Fallback { public $a; public $b; }
// A NULL class column keeps the default class; an unknown name is stdClass.
foreach ($db->query('SELECT cls, a, b FROM t')->fetchAll(PDO::FETCH_CLASS | PDO::FETCH_CLASSTYPE, 'Fallback') as $o) {
    echo get_class($o), ' ', $o->a, $o->b, "\n";
}

class Ser implements Serializable
{
    public $from;
    public $b;
    public function serialize() { return ''; }
    public function unserialize($data) { echo "unserialize(", var_export($data, true), ")\n"; $this->from = $data; }
    public function __serialize(): array { return []; }
    public function __unserialize(array $data): void {}
}
$st = $db->query("SELECT 'payload', b FROM t");
var_dump(@$st->fetchAll(PDO::FETCH_CLASS | PDO::FETCH_SERIALIZE, 'Ser')[0]);

$st = $db->query('SELECT b AS path FROM t');
var_dump($st->fetchAll(PDO::FETCH_CLASS, 'Directory')[0]);

// A user getIterator() replaces the statement's own iterator; the class
// argument of IteratorIterator picks whose getIterator() runs.
class Rows extends PDOStatement
{
    public function getIterator(): Iterator
    {
        echo "Rows::getIterator\n";
        return new IteratorIterator($this, 'PDOStatement');
    }
}
$st = $db->prepare('SELECT a FROM t', [PDO::ATTR_STATEMENT_CLASS => ['Rows']]);
$st->execute();
foreach ($st as $k => $r) {
    echo $k, ':', $r['a'], "\n";
}
try {
    new IteratorIterator($st, 'Countable');
} catch (LogicException $e) {
    echo $e->getMessage(), "\n";
}
