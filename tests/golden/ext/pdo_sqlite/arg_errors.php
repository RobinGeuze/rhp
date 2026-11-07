<?php
ob_start(fn ($s) => str_replace(__DIR__, 'D', $s));
// Argument names in PDO's errors, uri: DSN failures, convert_to_long warnings
// of int binds.
function t($f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
t(fn () => new PDO('sqlite::memory:', null, null, [PDO::ATTR_ERRMODE => 99]));
t(fn () => new PDO('sqlite::memory:', null, null, [PDO::ATTR_CASE => 99]));
$db = new PDO('sqlite::memory:');
t(fn () => $db->setAttribute(PDO::ATTR_ERRMODE, 99));
t(fn () => $db->query('select 1', PDO::FETCH_CLASS, 'stdClass', [1]));
t(fn () => $db->query('select 1')->setFetchMode(PDO::FETCH_CLASS, 'stdClass', [1]));
t(fn () => $db->query('select 1')->fetchAll(PDO::FETCH_CLASS, 'stdClass', [1]));
t(fn () => new PDO('uri:file://' . __DIR__ . '/no-such-dsn-file'));
t(fn () => new PDO('uri:'));
$s = $db->prepare('select ?, typeof(?)');
foreach ([1e300, NAN, new ArrayObject(), '12abc', 2.5] as $v) {
    $x = $v;
    $s->bindParam(1, $x, PDO::PARAM_INT);
    $s->bindValue(2, $v, PDO::PARAM_BOOL);
    $s->execute();
    echo json_encode($s->fetch(PDO::FETCH_NUM)), ' ';
    var_dump($x);
}
