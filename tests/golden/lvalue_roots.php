<?php
// Write targets rooted at a call result or a variable-variable, by-ref
// arguments from call results, and dynamic static-property names.
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });

class A {
    public $p = [];
    public $q;
    static function g() { static $a; return $a ??= new A; }
}
A::g()->p[] = 1;
A::g()->p[] = 2;
A::g()->q['x'] ??= 5;
A::g()->p[0] += 10;
var_dump(A::g()->p, A::g()->q);

function wm() { static $m; return $m ??= new WeakMap; }
$k = new stdClass;
wm()[$k] = 'v';
var_dump(count(wm()), wm()[$k]);

function arr() { return [1]; }
arr()[0] = 5;
var_dump(arr()[0] ??= 7);

$a = [[1, 2]];
var_dump(array_pop(array_values($a)[0]));
var_dump(array_pop(explode(',', 'a,b')));
var_dump(end(str_split('abc')));
var_dump(sort(arr()));
var_dump(current(arr()), key(arr()));

// variable-variables
$n = 'x';
$$n = 1;
${$n . 'y'}[] = 2;
var_dump($x, $xy);
$$n += 5;
var_dump($x, $$n, ${'x'});
$m = 'n';
var_dump($$$m);
${'zz'} ??= 'z1';
${'zz'} ??= 'z2';
var_dump(${'zz'}, isset($$n), isset(${'nope'}));
function f() {
    $n = 'q';
    $$n = 5;
    echo $q, "\n";
    $k = 'dyn';
    $$k = [1];
    ${$k}[] = 2;
    var_dump($$k, isset($$k), ${'undefined_here'} ?? 'dflt');
}
f();

// dynamic static-property names
class C {
    public static $a = 1;
    protected static $b = [];
    static function t() { $p = 'b'; static::${$p}['k'] = 'v'; return self::$$p; }
}
$p = 'a';
var_dump(C::$$p, C::${'a'});
C::$$p = 10;
C::${$p} += 1;
var_dump(C::$a, C::t(), isset(C::$$p), isset(C::${'zz'}));
$cls = 'C';
var_dump($cls::${'a'});
$fn = Closure::bind(fn(string $c, string $pr) => $c::${$pr}, null, C::class);
var_dump($fn('C', 'b'));
try {
    var_dump(C::${'nope'});
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
