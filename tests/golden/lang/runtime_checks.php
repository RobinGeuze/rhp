<?php
// Runtime errors of forms php compiles but rejects when they run.

function attempt(callable $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

class A {
    public $p = 1;
    public static $s = 2;
    const K = 'k';
    function m() { return function () { return $this->p; }; }
    static function sm() { return function () { return isset($this); }; }
    static function st() { return $this; }
}
class B extends A {
    function pc() { return parent::class; }
    static function sc() { return static::class; }
}

// $this where no object is bound.
attempt(fn() => (new A)->m()());
attempt(fn() => A::sm()());
attempt(fn() => A::st());
attempt(function () { return $this; });
attempt(fn() => isset($this));
attempt(fn() => empty($this));

// Nullsafe short-circuits the whole chain.
$n = null;
attempt(fn() => $n?->a->b()['c']);
attempt(fn() => $n?->a(print("not evaluated\n"))->b);
attempt(fn() => (new A)?->p);

// X::class at run time.
attempt(fn() => B::sc());
attempt(fn() => (new B)->pc());
attempt(fn() => $n::class);
attempt(fn() => (new B)::class);
$s = 'A';
attempt(fn() => $s::class);

// Dynamic class constant fetch.
$k = 'K';
attempt(fn() => A::{$k});
attempt(fn() => A::{'K'} . B::{strtolower('k') === 'k' ? 'K' : 'X'});
attempt(fn() => A::{1});
attempt(fn() => A::{'NOPE'});

// unset of a static property.
attempt(function () { unset(A::$s); });

// self/parent/static without a class scope.
$c = function () { return new self; };
attempt($c);
$c = function () { return static::K; };
attempt($c);
$c = function () { return new stdClass instanceof parent; };
attempt($c);

// [] read in an argument of a function called by name.
function takes($x) { return $x; }
function takes_ref(&$x) { $x = 'set'; return $x; }
function call_by_name() {
    $a = [];
    attempt(fn() => later_ref($a[]));
    var_dump($a);
    attempt(fn() => later_val($a[]));
}
function later_ref(&$x) { $x = 'appended'; return $x; }
function later_val($x) { return $x; }
call_by_name();

// An optional parameter before a required one is required.
set_error_handler(function ($no, $msg) { echo "handler: $msg\n"; return true; });
eval('function opt($a = 1, $b) { return [$a, $b]; }');
restore_error_handler();
attempt(fn() => opt(b: 2));
attempt(fn() => opt(5, 6));
echo (new ReflectionFunction('opt'))->getParameters()[0]->isOptional() ? "optional\n" : "required\n";

var_dump(__COMPILER_HALT_OFFSET__ > 0);
__halt_compiler();
data after the halt
