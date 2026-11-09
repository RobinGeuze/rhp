<?php
// PDOStatement's iterator has no rewind: InternalIterator::rewind() after
// the first step is an Error.
$pdo = new PDO('sqlite::memory:');
$pdo->exec('create table t(a)'); $pdo->exec('insert into t values (1),(2)');
$it = $pdo->query('select a from t')->getIterator();
$it->rewind();
foreach ($it as $k => $r) echo $k, ' ', $r['a'], "\n";
try { $it->rewind(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { foreach ($it as $r) {} } catch (Error $e) { echo $e->getMessage(), "\n"; }
