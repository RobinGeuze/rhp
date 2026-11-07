<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// Fetch modes, typed results and the error modes, as WordPress and Laravel use them.
$db = new PDO('sqlite::memory:', null, null, [PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_OBJ]);
$db->exec('CREATE TABLE u (id INTEGER PRIMARY KEY AUTOINCREMENT, name VARCHAR(20) NOT NULL UNIQUE, n BIGINT, f DOUBLE, b BLOB)');
$ins = $db->prepare('INSERT INTO u (name, n, f, b) VALUES (:name, :n, :f, :b)');
foreach ([['ann', 9223372036854775807, 0.1, "\0x"], ['bob', -3, 1e100, null], ['cy', null, 2.5, 'z']] as [$name, $n, $f, $b]) {
    $ins->bindValue(':name', $name);
    $ins->bindValue(':n', $n, $n === null ? PDO::PARAM_NULL : PDO::PARAM_INT);
    $ins->bindValue(':f', $f);
    $ins->bindValue(':b', $b, PDO::PARAM_LOB);
    var_dump($ins->execute(), $ins->rowCount(), $db->lastInsertId());
}
var_dump($db->query('SELECT * FROM u')->fetchAll());
$st = $db->query('SELECT id, name, n FROM u ORDER BY id');
var_dump($st->fetchAll(PDO::FETCH_NUM));
var_dump($db->query('SELECT name, id FROM u')->fetchAll(PDO::FETCH_UNIQUE | PDO::FETCH_ASSOC));
var_dump($db->query('SELECT f > 1 AS big, name FROM u')->fetchAll(PDO::FETCH_GROUP | PDO::FETCH_COLUMN));
var_dump($db->query('SELECT name, name AS name FROM u LIMIT 1')->fetchAll(PDO::FETCH_NAMED));
var_dump($db->query('SELECT id, name FROM u')->fetchAll(PDO::FETCH_FUNC, fn($id, $name) => "$id:$name"));
$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, true);
var_dump($db->query('SELECT id, f FROM u LIMIT 1')->fetch(PDO::FETCH_ASSOC));
$db->setAttribute(PDO::ATTR_STRINGIFY_FETCHES, false);
$st = $db->prepare('SELECT id, name FROM u WHERE id = ?');
$st->execute([2]);
$st->bindColumn(2, $name);
$st->fetch(PDO::FETCH_BOUND);
var_dump($name);
$st = $db->query('SELECT * FROM u');
for ($i = 0; $i < $st->columnCount(); $i++) {
    var_dump($st->getColumnMeta($i));
}
// Errors: a constraint violation in each error mode.
foreach ([PDO::ERRMODE_SILENT, PDO::ERRMODE_WARNING, PDO::ERRMODE_EXCEPTION] as $mode) {
    $db->setAttribute(PDO::ATTR_ERRMODE, $mode);
    try {
        var_dump($db->exec("INSERT INTO u (name) VALUES ('ann')"));
    } catch (PDOException $e) {
        var_dump(get_class($e), $e->getMessage(), $e->getCode(), $e->errorInfo);
    }
    var_dump($db->errorCode(), $db->errorInfo());
}
try {
    $db->prepare('SELECT nope FROM u');
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    $db->query('SELECT 1')->fetchAll(PDO::FETCH_KEY_PAIR);
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
try {
    $db->query('SELECT 1')->fetch(99);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
// Transactions and savepoints (Laravel's nested transactions).
$db->beginTransaction();
$db->exec('SAVEPOINT trans2');
$db->exec("INSERT INTO u (name) VALUES ('dee')");
$db->exec('ROLLBACK TO SAVEPOINT trans2');
$db->commit();
var_dump($db->query('SELECT COUNT(*) FROM u')->fetchColumn());
try {
    $db->commit();
} catch (PDOException $e) {
    echo $e->getMessage(), "\n";
}
var_dump($db->query('PRAGMA foreign_keys')->fetchColumn());
$db->exec('PRAGMA foreign_keys = ON');
var_dump($db->query('PRAGMA foreign_keys')->fetchColumn());
