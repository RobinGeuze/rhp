<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
// PDOStatement::$queryString is written once; uninitialized objects; PDO
// subclasses with tentative return types.
class S extends PDOStatement { public function __construct($q) { $this->queryString = $q; } }
$s = new S('x'); var_dump($s->queryString);
try { $s->queryString = 'y'; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { unset($s->queryString); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $s->execute(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$db = new PDO('sqlite::memory:');
$st = $db->query('select 1'); try { $st->queryString = 'z'; } catch (Error $e) { echo $e->getMessage(), "\n"; }
class P extends PDO { public function __construct() {} }
$p = new P; try { $p->exec('x'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($p instanceof PDO);
class Q extends PDO { #[\ReturnTypeWillChange] public function exec($q) { return 1; } }
if (true) { class Q2 extends PDO { public function query($q, $m = null, ...$a) { return 1; } } }
echo "ok\n";
