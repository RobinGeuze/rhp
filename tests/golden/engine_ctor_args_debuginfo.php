<?php
// Engine semantics behind the tooling builtins: constructor visibility,
// left-to-right argument evaluation, offsetUnset exceptions, __debugInfo in
// var_dump/print_r, and built-in (prelude) functions reporting argument
// errors like internal ones.

class DI { private $p = 1; public int $typed; function __debugInfo() { return ['q' => 2, 5 => 'five', "\0DI\0priv" => 3, "\0*\0prot" => [new DN]]; } }
class DN { public $x = 1; function __debugInfo() { return []; } }
class DS { function __debugInfo() { return ['me' => $this]; } }
$di = new DI; $ds = new DS;
var_dump($di, $ds);
print_r($di); print_r($ds); echo "\n";

set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });

class Priv { private function __construct() {} static function make() { return new Priv; } }
class Prot { protected function __construct() {} }
class ProtChild extends Prot { static function make() { return new ProtChild; } }
function mk() { return new Prot; }
try { new Priv; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { mk(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { new HashContext; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(Priv::make() instanceof Priv, ProtChild::make() instanceof Prot);

function setr(&$x, $v) { $x = $v; return 0; }
function show(...$a) { echo json_encode($a), "\n"; }
$m = 1;
show($m, setr($m, 2), $m);
$n = 1; $r = &$n;
show($n, setr($n, 5), $n);
var_dump(preg_match('/a/', 'a', $q), $q, preg_match('/b/', 'b', $q), $q);

class AA implements ArrayAccess {
    function offsetExists($o): bool { return true; }
    function offsetGet($o): mixed { return 1; }
    function offsetSet($o, $v): void {}
    function offsetUnset($o): void { throw new Exception("unset $o refused"); }
}
$aa = new AA;
try { unset($aa[5]); echo "not reached\n"; } catch (Exception $e) { echo $e->getMessage(), "\n"; }


try { hash_update(hash_init('md5'), []); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { hash_final(); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
$sfa = new SplFixedArray(null);
var_dump($sfa->getSize());
mt_srand(3, MT_RAND_PHP);
var_dump(mt_rand(1, 100));
var_dump(is_float(lcg_value()));
