<?php
// new class(...$args) / new class(name: $v): the arguments go to the constructor.
$p = ['a' => 1, 'b' => 2]; $x = new class(...$p) { function __construct(public $a, public $b) {} }; var_dump($x->a, $x->b);
$x = new class(b: 5, a: 4) { function __construct(public $a, public $b) {} }; var_dump($x->a, $x->b);
$x = new class(1, ...[2, 3]) { public $r; function __construct(...$r) { $this->r = $r; } }; var_dump($x->r);
function g() { return 'G'; }
$s = ''; $x = new class($s, g(), ...[]) { function __construct(&$o, $x) { $o = "anon$x"; } }; var_dump($s);
// Laravel's Castable::castUsing
interface CastsAttributes { function get($v); }
class Money { static function castUsing(array $arguments) { return new class(...$arguments) implements CastsAttributes {
  function __construct(protected string $currency = 'EUR') {} function get($v) { return "$v {$this->currency}"; } }; } }
var_dump(Money::castUsing([])->get(5), Money::castUsing(['USD'])->get(6));
try { new class(...[1], x: 2) { function __construct($a) {} }; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// A constructor that throws: the object is never destructed.
class D { function __destruct() { echo "dtor\n"; } }
class E extends D { function __construct($x) { throw new Exception("boomE"); } }
try { new class(...[1]) extends D { function __construct($x) { throw new Exception("boom"); } }; } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { new E(...[1]); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { new E(x: 1); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
$c = 'E'; try { new $c(...[1]); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
echo "end\n";
