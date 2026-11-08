<?php
// Method and class-name sites reached with several classes, cases and
// scopes: the lookup caches must give what a fresh lookup gives.

trait Greets {
    function hello() { return 'hello from ' . static::class; }
}
class A {
    use Greets { hello as protected hi; }
    private function secret() { return 'A secret'; }
    function callSecret($o) { return $o->secret(); }
    function __call($n, $a) { return "__call($n)"; }
    static function make() { return new static; }
}
class B extends A {
    function secret() { return 'B public secret'; }
}
class C {
    function secret() { return 'C secret'; }
    function HELLO() { return 'C HELLO'; }
}

$a = new A; $b = new B; $c = new C;
for ($i = 0; $i < 2; $i++) {
    // From A's scope, A's private secret() runs even on a B.
    echo $a->callSecret($a), ' | ', $a->callSecret($b), ' | ', $a->callSecret($c), "\n";
    foreach ([$a, $b, $c] as $o) {
        echo $o->hello(), ' / ', $o->Hello(), ' / ';
        // Outside the class: A's private goes to __call.
        echo $o->secret(), " / ", $o instanceof A ? $o->hi() : "-", "\n";
    }
}

// Static calls and class names in other cases and through aliases.
class_alias('B', 'BeeAlias');
for ($i = 0; $i < 2; $i++) {
    echo get_class(a::make()), ' ', get_class(B::MAKE()), ' ', get_class(beealias::make()), "\n";
    echo get_class(new a), ' ', get_class(new BEEALIAS), ' ', (new b) instanceof a ? 'yes' : 'no', "\n";
}

// A class that does not exist yet, then is declared.
function probe() {
    try {
        return get_class(new Later);
    } catch (Error $e) {
        return $e->getMessage();
    }
}
echo probe(), "\n";
eval('class Later {}');
echo probe(), "\n";
echo probe(), "\n";

// The same call site, receivers of alternating classes.
interface Shape { function area(); }
class Sq implements Shape { function __construct(public $s) {} function area() { return $this->s ** 2; } }
class Ci implements Shape { function __construct(public $r) {} function area() { return round(3.14 * $this->r ** 2, 2); } }
$t = 0;
foreach ([new Sq(2), new Ci(1), new Sq(3), new Ci(2)] as $s) {
    $t += $s->area();
}
echo $t, "\n";
