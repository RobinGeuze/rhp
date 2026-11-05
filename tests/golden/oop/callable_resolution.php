<?php
// Callable checks (zend_is_callable_ex) versus dynamic calls, method
// lookup from a scope, and the by-reference warning of internal callers.

function show(callable $f) { try { $f(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
function tryit(callable|Closure $f) {}

class P {
    private function secret() { echo "P::secret\n"; }
    protected function prot() { echo "P::prot\n"; }
    public function callSecret() { $this->secret(); [$this, 'secret'](); call_user_func([$this, 'secret']); }
    public function __call($n, $a) { echo "P::__call($n)\n"; }
    public static function __callStatic($n, $a) { echo "P::__callStatic($n)\n"; }
    public function inst() { echo "P::inst ", static::class, "\n"; }
    public static function stat() { echo "P::stat ", static::class, "\n"; }
}

class C extends P {
    public function secret() { echo "C::secret\n"; }
    protected function prot() { echo "C::prot\n"; }
    public function run() {
        // A missing method through static syntax from an instance: __call.
        P::missing();
        self::missing2();
        static::missing3();
        // Dynamic calls never borrow $this.
        foreach (['P::inst', ['P', 'inst'], 'P::stat', [$this, 'inst']] as $f) {
            try { $f(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
        }
        // A callable check does.
        call_user_func('P::inst');
        call_user_func([$this, 'parent::inst']);
        var_dump(is_callable('parent::inst'));
    }
}

class D extends P {
    public function viaSibling(P $o) { $o->prot(); }
}

(new C)->callSecret();
(new C)->run();
(new D)->viaSibling(new C);

try { (new C)->nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
P::nope();
$m = "a\0b";
P::$m();

class Q { private function hidden() {} protected static function hs() {} }
foreach (['Q::hidden', 'Q::hs', ['Q', 'hidden'], [new Q, 'hidden'], 'Q::none', 'NoClass::x', '::x', [1, 2], ['Q'], [1 => 'Q', 2 => 'x']] as $c) {
    try { array_map($c, [1]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
}
foreach (['Q::hidden', [new Q, 'hidden'], 'Q::none', ['Q'], [1 => 'Q', 2 => 'x'], '\strlen'] as $c) {
    try { $c(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}

abstract class Ab { abstract function f(); static function g() { static::f(); } }
try { Ab::g(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
trait T { static function ts() { echo "T::ts\n"; } }
T::ts();

class R {
    function byRef(&$x, &...$rest) { $x = 1; }
}
function refd(&$a, $b) { $a = 'changed'; }
call_user_func('refd', 1, 2);
call_user_func([new R, 'byRef'], 1, 2, 3);
call_user_func('sort', [3, 1]);
$v = 'orig';
call_user_func_array('refd', [&$v, 2]);
var_dump($v);
function vr(&...$xs) { foreach ($xs as &$x) { $x = 'set'; } }
vr($p, $q);
var_dump($p, $q);

$arr = ['b' => 2, 'a' => 1, 'c' => 3];
uasort($arr, fn($x, $y) => $x > $y);
var_dump($arr);
uksort($arr, function ($x, $y) { echo "$x<>$y\n"; return strcmp($x, $y); });
var_dump($arr);

class S {
    function reg() { register_shutdown_function('S::down'); }
    function down() { echo "down: ", get_class($this), "\n"; }
    function fc() { return Closure::fromCallable('self::down'); }
}
(new S)->reg();
$c = (new S)->fc();
$c();

class Base { function who() { return 'Base'; } }
class Kid extends Base { function test() { try { tryit('parent::who'); } catch (TypeError $e) { echo "TypeError\n"; } tryit([$this, 'who']); return eval('return Base::who();'); } }
var_dump((new Kid)->test());
$o = new stdClass;
var_dump($o instanceof NotLoaded);
class FcP { function m() { echo "FcP::m ", get_class($this), "\n"; } }
class FcA extends FcP { function m() { echo "FcA::m\n"; } function f() { return [parent::m(...), self::m(...)]; } }
foreach ((new FcA)->f() as $fc) { $fc(); }
echo "end\n";
