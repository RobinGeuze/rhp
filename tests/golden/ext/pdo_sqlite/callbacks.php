<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// Pdo\Sqlite user functions, aggregates and collations (called from inside
// sqlite3_step), PDO::connect, ATTR_STATEMENT_CLASS, FETCH_CLASS/FETCH_INTO.
$db = new Pdo\Sqlite('sqlite::memory:');
var_dump(get_class($db));
$db->createFunction('dbl', fn($x) => $x * 2);
var_dump($db->query('select dbl(21)')->fetchColumn());
$db->createAggregate('mysum', function ($ctx, $n, $v) { return ($ctx ?? 0) + $v; }, function ($ctx, $n) { return $ctx; });
$db->exec('create table n(v int)'); $db->exec('insert into n values (1),(2),(3)');
var_dump($db->query('select mysum(v) from n')->fetchColumn());
$db->createCollation('rev', fn($a, $b) => -strcmp($a, $b));
var_dump($db->query("select v from n order by cast(v as text) collate rev")->fetchAll(PDO::FETCH_COLUMN));
$db->createFunction('boom', function () { throw new RuntimeException('boom!'); });
try { $db->query('select boom()'); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$p = PDO::connect('sqlite::memory:'); var_dump(get_class($p));
echo PDO::SQLITE_ATTR_OPEN_FLAGS, "\n";
class S extends PDOStatement { public $extra; protected function __construct($a) { $this->extra = $a; } }
$db->setAttribute(PDO::ATTR_STATEMENT_CLASS, ['S', ['hi']]);
$s = $db->query('select 1 as one'); var_dump($s);
class R { public $one; public function __construct() { echo "ctor\n"; } }
var_dump($db->query('select 1 as one')->fetchAll(PDO::FETCH_CLASS, 'R'));
$q = $db->query('select 1 as one, 2 as two'); $q->setFetchMode(PDO::FETCH_INTO, $o = new stdClass); var_dump($q->fetch());
