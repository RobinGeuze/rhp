<?php
// Property names starting with "\0" can never reach a (mangled) private slot.
class A { private $secret = 's3cret'; protected $prot = 'p'; public $pub = 1; }
$a = new A;
foreach (["\0A\0secret", "\0*\0prot", "\0x", "\0"] as $name) {
    try { var_dump($a->$name); } catch (Error $e) { echo "read: ", $e->getMessage(), "\n"; }
    try { $a->$name = 'pwn'; echo "wrote\n"; } catch (Error $e) { echo "write: ", $e->getMessage(), "\n"; }
    try { unset($a->$name); echo "unset ok\n"; } catch (Error $e) { echo "unset: ", $e->getMessage(), "\n"; }
    var_dump(isset($a->$name), empty($a->$name), $a->$name ?? 'coalesced', property_exists($a, $name));
}
try { var_dump($a->{"\0A\0secret"}); } catch (Error $e) { echo $e->getMessage(), "\n"; }
print_r($a);
echo "\n";
var_dump(count((array)$a));

// With magic accessors, the name is handed to __get / __set / __isset.
class M {
    function __get($n) { echo "get ", strlen($n), "\n"; return 'g'; }
    function __set($n, $v) { echo "set ", strlen($n), "\n"; }
    function __isset($n) { echo "isset ", strlen($n), "\n"; return true; }
}
$m = new M;
var_dump($m->{"\0M\0x"});
$m->{"\0y"} = 1;
var_dump(isset($m->{"\0z"}));
