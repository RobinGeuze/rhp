<?php
ob_start(fn ($s) => str_replace(__DIR__, 'D', $s));
// PDO's driver methods (dbh_method_get): pdo_sqlite's sqliteCreate*,
// deprecated since 8.5, exist on a sqlite connection only.
$pdo = new PDO('sqlite::memory:');
var_dump(method_exists($pdo, 'sqliteCreateFunction'), method_exists($pdo, 'SQLITECREATECOLLATION'));
var_dump(method_exists('PDO', 'sqliteCreateFunction'), method_exists($pdo, 'sqliteNope'));
var_dump(in_array('sqliteCreateFunction', get_class_methods($pdo)));
var_dump((new ReflectionClass('PDO'))->hasMethod('sqliteCreateFunction'));

var_dump($pdo->sqliteCreateFunction('add1', fn($x) => $x + 1, 1));
var_dump($pdo->sqliteCreateAggregate('cat', function ($ctx, $n, $v) { return $ctx . $v; }, fn($ctx) => "[$ctx]"));
var_dump($pdo->SqliteCreateCollation('nocase2', fn($a, $b) => strcasecmp($a, $b)));
var_dump($pdo->query("SELECT add1(41), cat(x) FROM (SELECT 'a' AS x UNION SELECT 'b')")->fetch(PDO::FETCH_NUM));

// Argument checks under the method's own name.
foreach ([
    fn() => $pdo->sqliteCreateFunction('f'),
    fn() => $pdo->sqliteCreateFunction('f', 'no_such_function'),
    fn() => $pdo->sqliteCreateAggregate('f', 'strlen', 'strlen', 'x'),
    fn() => $pdo->sqliteCreateCollation('f', 'strcmp', 3),
    fn() => $pdo->sqliteNope(),
] as $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

// A subclass and Pdo\Sqlite reach them too.
class MyPdo extends PDO {}
$my = new MyPdo('sqlite::memory:');
var_dump($my->sqliteCreateFunction('one', fn() => 1));
try {
    $my->nothing();
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
$s = new Pdo\Sqlite('sqlite::memory:');
var_dump($s->sqliteCreateFunction('two', fn() => 2), $s->query('SELECT two()')->fetchColumn());

// The method's frame shows in a trace.
function boom() { throw new Exception('in callback'); }
$pdo->sqliteCreateFunction('boom', 'boom');
try {
    $pdo->query('SELECT boom()');
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}
try {
    (function () use ($pdo) { $pdo->sqliteCreateFunction([], 'strlen'); })();
} catch (TypeError $e) {
    echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n";
}
