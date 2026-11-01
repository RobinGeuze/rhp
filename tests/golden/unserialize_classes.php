<?php
// unserialize(): allowed_classes, __PHP_Incomplete_Class, enums, and php's
// mapping of (un)mangled names onto declared properties.
class H { public $x; function __unserialize($d) { echo "H::__unserialize\n"; $this->x = $d; } }
class Wk { public $a = 0; function __wakeup() { echo "Wk::__wakeup\n"; } }
print_r(unserialize('O:1:"H":1:{i:0;i:1;}', ['allowed_classes' => false]));
echo "\n";
print_r(unserialize('O:1:"H":1:{i:0;i:1;}', ['allowed_classes' => ['h']]));
echo "\n";
print_r(unserialize('O:2:"Wk":1:{s:1:"a";i:5;}', ['allowed_classes' => ['Other']]));
echo "\n";
print_r(unserialize('a:2:{i:0;O:2:"Wk":0:{}i:1;O:1:"H":0:{}}', ['allowed_classes' => ['WK']]));
echo "\n";
print_r(unserialize('O:2:"Wk":1:{s:1:"a";i:5;}', ['allowed_classes' => true]));
echo "\n";
$inc = unserialize("O:7:\"Missing\":3:{s:1:\"a\";i:1;s:4:\"\0*\0b\";i:2;s:10:\"\0Missing\0c\";i:3;}");
var_dump(get_class($inc));
print_r($inc);
echo "\n";
echo str_replace("\0", '~', serialize($inc)), "\n";
var_dump(serialize(unserialize('O:7:"Missing":0:{}')));
foreach ([1, 'x', null, 1.5] as $bad) {
    try { unserialize('i:1;', ['allowed_classes' => $bad]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
try { unserialize('i:1;', ['allowed_classes' => [1]]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { unserialize('i:1;', ['allowed_classes' => ['bad name']]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

enum E: string { case A = 'a'; const C = 'x'; }
enum P { case One; }
var_dump(unserialize('E:3:"E:A";'), unserialize('E:5:"P:One";'));
var_dump(@unserialize('E:3:"E:C";'), @unserialize('E:3:"E:Z";'), @unserialize('E:3:"H:x";'), @unserialize('E:3:"Q:x";'), @unserialize('E:2:"EA";'));
var_dump(unserialize(serialize([E::A, P::One])) === [E::A, P::One]);
foreach (['O:1:"E":0:{}', 'O:1:"I":0:{}', 'O:2:"Ab":0:{}'] as $s) {
    try { var_dump(unserialize($s)); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
interface I {}
abstract class Ab {}

class A { private $a = 1; protected $b = 2; public $c = 3; }
class B extends A { private $a = 'b'; }
print_r(unserialize("O:1:\"A\":3:{s:1:\"a\";i:7;s:1:\"b\";i:8;s:4:\"\0A\0c\";i:9;}"));
print_r(unserialize("O:1:\"B\":3:{s:1:\"a\";i:7;s:4:\"\0A\0a\";i:8;s:4:\"\0*\0c\";i:9;}"));
print_r(@unserialize("O:1:\"A\":1:{s:4:\"\0X\0z\";i:1;}"));
var_dump(@unserialize("O:1:\"A\":1:{s:3:\"\0ab\";i:1;}"));
var_dump(unserialize(serialize(new B)) == new B);
