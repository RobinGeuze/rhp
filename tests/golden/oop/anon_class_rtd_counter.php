<?php
namespace N;
// CG(rtd_key_counter): anonymous classes and the class-likes php can't bind
// at compile time share one counter, counted in source order. Printed
// relative to the first (the counter lives as long as the thread).
function show($o) {
    static $first = null;
    $n = hexdec(substr(get_class($o), strrpos(get_class($o), '$') + 1));
    $first ??= $n;
    echo $n - $first, "\n";
}
show(new class {});
class Plain {}
class Child extends Plain {}
class Ex extends \Exception {}
class Late extends Later {}
class Later {}
interface I {}
interface J extends I {}
class Impl implements I {}
trait T {}
class UsesT { use T; }
enum E { case A; }
show(new class {});
function mk() { return new class { function f() { return new class {}; } }; }
$x = mk(); show($x); show($x->f());
if (true) { class Cond { function g() { return new class {}; } } }
show((new Cond)->g());
show(new class extends Plain {});
show(eval('return new class {};'));
show(new class {});
