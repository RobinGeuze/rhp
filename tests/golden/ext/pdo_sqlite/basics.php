<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// PDO on sqlite: exec, prepare, binds, fetch modes, iteration, errors, quoting.
$db = new PDO('sqlite::memory:');
var_dump($db->getAttribute(PDO::ATTR_ERRMODE), $db->getAttribute(PDO::ATTR_DRIVER_NAME));
$db->exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL, data BLOB)');
var_dump($db->exec("INSERT INTO t (name, score) VALUES ('a', 1.5), ('b', 2.25)"));
var_dump($db->lastInsertId());
$st = $db->prepare('INSERT INTO t (name, score) VALUES (?, ?)');
var_dump($st->execute(['c', 3]));
$st = $db->prepare('SELECT * FROM t WHERE id > :id');
$st->bindValue(':id', 0, PDO::PARAM_INT);
$st->execute();
var_dump($st->fetch(PDO::FETCH_ASSOC));
var_dump($st->fetch(PDO::FETCH_NUM));
var_dump($st->fetch(PDO::FETCH_OBJ));
var_dump($st->fetch());
var_dump($st->getColumnMeta(0), $st->columnCount());
foreach ($db->query('SELECT id, name FROM t') as $k => $row) { var_dump($k, $row); }
var_dump($db->query('SELECT id, name FROM t')->fetchAll(PDO::FETCH_KEY_PAIR));
var_dump($db->query('SELECT name FROM t')->fetchAll(PDO::FETCH_COLUMN));
try { $db->exec('SELECT * FROM nope'); } catch (PDOException $e) { var_dump($e->getMessage(), $e->getCode(), $e->errorInfo); echo $e, "\n"; }
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_WARNING);
var_dump($db->query('SELECT * FROM nope'));
var_dump($db->errorCode(), $db->errorInfo());
var_dump($db->quote("it's"));
$db->beginTransaction(); var_dump($db->inTransaction()); $db->rollBack(); var_dump($db->inTransaction());
$st = $db->prepare('select ?'); $x = 5; $st->bindParam(1, $x); $x = 7; $st->execute(); var_dump($st->fetchColumn(), $x);
var_dump($st);
