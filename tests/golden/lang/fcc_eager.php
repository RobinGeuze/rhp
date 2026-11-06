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
