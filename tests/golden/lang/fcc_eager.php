<?php
// A first-class callable resolves its callee when it is created.
namespace N;
class A {
    private function priv() {}
    public function inst() { return "inst"; }
    public static function stat() { return "stat"; }
    public static function __callStatic($n, $a) { return "magic $n"; }
    public function own() { return [self::stat(...), static::stat(...), $this->priv(...)]; }
}
class B extends A {}
$tries = [
    fn() => undefined_fn(...),
    fn() => \undefined_fn(...),
    fn() => Missing::m(...),
    fn() => A::inst(...),
    fn() => (new A)->priv(...),
    fn() => (new A)->nope(...),
    fn() => [new A, 'nope'](...),
    fn() => 'A::stat'(...),
    fn() => 'N\A::stat'(...),
    fn() => (42)(...),
    fn() => ([])(...),
];
foreach ($tries as $i => $t) {
    try { $t(); echo "$i ok\n"; } catch (\Error $e) { echo "$i ", get_class($e), ": ", $e->getMessage(), "\n"; }
}
var_dump(A::anything(...)());
var_dump(count((new B)->own()));
var_dump(strlen(...)("abc"));
$c = function () { return 1; };
var_dump($c(...) === $c);
$f = strlen(...);
var_dump($f(...) === $f);
// static-call syntax on $this's class: an instance method of $this
class P2 { function m() { return "P2::m " . get_class($this); } private function pv() { return "pv"; } function getpv() { return self::pv(...); } }
class C2 extends P2 {
    function m() { return "C2"; }
    function t() { return [parent::m(...), self::m(...), P2::m(...), static::m(...), $this->getpv()]; }
}
foreach ((new C2)->t() as $f) { var_dump($f()); }
class Q { function f() { return P2::m(...); } }
try { (new Q)->f(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
// An array callable has no scope or $this; a 'C::m' literal is static-call syntax.
class R2 extends P2 {
    function t() {
        foreach ([fn() => 'N\P2::m'(...), fn() => ['N\P2', 'm'](...), fn() => ['self', 'm'](...), fn() => 'self::m'(...)] as $i => $t) {
            try { echo $i, " ", $t()(), "\n"; } catch (\Error $e) { echo $i, " ", $e->getMessage(), "\n"; }
        }
    }
}
(new R2)->t();
