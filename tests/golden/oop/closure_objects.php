<?php
// Closures are objects: handles, statics per Closure object, debug info.
function d(...$v) {
    ob_start();
    var_dump(...$v);
    // The path and its length depend on the checkout.
    echo str_replace('string(' . strlen(__FILE__) . ') "' . __FILE__, 'string(N) "FILE', ob_get_clean());
}
class P { public $q; }
$a = new P; $a->q = new P;
echo spl_object_id($a), spl_object_id($a->q), "\n";
unset($a); // the inner object is freed first, so the outer handle comes back first
$x = new stdClass; $y = new stdClass;
echo spl_object_id($x), spl_object_id($y), "\n";
echo spl_object_hash($x), "\n";
unset($x, $y);

function counter() {
    return function () { static $n = 0; return ++$n; };
}
$c1 = counter(); $c2 = counter();
echo $c1(), $c1(), $c2(), "\n";
$c3 = $c1->bindTo(null); // copies the current statics
echo $c3(), $c1(), "\n";
d($c1);

class K {
    private $v = 'k';
    function get() { return function (&$r, $o = 1) { return $this->v; }; }
    static function sget() { return static fn () => 1; }
}
$f = (new K)->get();
echo str_replace(__FILE__, 'FILE', print_r($f, true)), "\n";
d(K::sget(), strlen(...), (new K)->get(...));
var_dump(Closure::fromCallable([$f, '__invoke']) === $f);
var_dump(method_exists($f, '__invoke'), is_callable([$f, '__invoke'], false, $name), $name);

$g = function () { return Closure::getCurrent(); };
var_dump($g() === $g);

// Closure::bind refusals
set_error_handler(function ($no, $msg) { echo "Warning: $msg\n"; return true; });
$s = static function () {};
var_dump(Closure::bind($s, new K));
var_dump(Closure::bind(function () {}, null, 'NoSuchClass'));
var_dump(Closure::bind(function () {}, null, 'stdClass'));
var_dump(Closure::bind(strlen(...), null, 'K'));
var_dump(Closure::bind((new K)->get(...), null));
try { echo $f; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $f->p = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $f->q[] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $f->r .= 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { new Closure; } catch (Error $e) { echo $e->getMessage(), "\n"; }

// Closure::call shares the closure's static variables, even ones it
// initialises first
$cnt = function () { static $n = 0; return ++$n; };
var_dump($cnt->call(new K), $cnt->call(new K), $cnt());

// Unbinding a closure whose arrow function uses $this drops that $this too
class T { public $v = 1; function mk() { return function () { return fn() => $this->v; }; } }
$u = (new T)->mk()->bindTo(null);
d($u);

// An internal function's first-class callable keeps no $this
var_dump(strlen(...)->bindTo(new K));
