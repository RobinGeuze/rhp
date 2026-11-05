<?php
// VM helpers that mutation testing (cargo-mutants) found under-covered;
// each block names the helper it guards.

function caught(Throwable $e) {
    echo get_class($e), ': ', str_replace(__FILE__, basename(__FILE__), $e->getMessage()), "\n";
}

// is_callable_value: every callable shape
function sq($x) { return $x; }
class Shapes { public function m() {} public function __invoke() {} public static function s() { return 's'; } }
$o = new Shapes;
var_dump(is_callable('sq'), is_callable('strlen'), is_callable('nope'), is_callable(fn() => 1));
var_dump(is_callable([$o, 'm']), is_callable([$o, 'nope']), is_callable($o), is_callable(42));
// a 1- or 3-element array is not a callable (the `len() == 2` guard)
var_dump(is_callable([$o]), is_callable([$o, 'm', 1]));
// ['ClassName', 'staticMethod'] (the string class name arm)
var_dump(is_callable(['Shapes', 's']));

// call_callable_value: string and array callables
function sq2($x) { return $x * $x; }
echo call_user_func('sq2', 6), "\n";
class Inc { public function m($x) { return $x + 1; } public static function s($x) { return $x * 2; } }
$i = new Inc;
$cb = [$i, 'm'];
echo $cb(3), ' ', call_user_func([$i, 'm'], 10), ' ', call_user_func(['Inc', 's'], 21), "\n";
// resolve_callable: calling a three-element array fails
try {
    $cb = [$i, 'm', 1];
    $cb();
} catch (Throwable $e) {
    caught($e);
}

// define_const / get_const: a redefinition warns and returns false
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; return true; });
var_dump(define('X', 1), define('X', 2));
restore_error_handler();
echo X, "\n";
const PI = 3;
define('E', 2);
echo constant('PI'), ' ', constant('E'), ' ', PI, ' ', E, "\n";

// enum_builtin_static: a string-backed enum's from / tryFrom
enum SB: string { case A = 'a'; case B = 'b'; }
echo SB::from('b')->name, "\n";
var_dump(SB::tryFrom('a'), SB::tryFrom('z'));

// is_enum_class: a plain object dumps as an object
class Plain { public $x = 1; public $y = 'hi'; }
var_dump(new Plain());

// class_parent_name: the inheritance chain
class A {}
class B extends A {}
var_dump(new B() instanceof A, get_parent_class(new B()), get_parent_class(new A()));

// type_lc: the member-call-on-non-object error names the type
foreach ([5, null] as $x) {
    try {
        $x->foo();
    } catch (Error $e) {
        caught($e);
    }
}

// dim_fetch: negative and out-of-bounds string offsets
$s = 'abc';
echo $s[-1], $s[-3], "\n";
var_dump(@$s[3], @$s[-5]);

// autovivify: null vivifies, a string (even "") is a container
$x = '';
try {
    $x[] = 5;
} catch (Error $e) {
    caught($e);
}
$x = null;
$x[] = 5;
print_r($x);
$x = '';
$x[2] = 'a';
var_dump($x);
$s = 'hi';
try {
    $s[] = 'x';
} catch (Error $e) {
    caught($e);
}

// bind_params / required_params: the argument count error
function req($a, $b, $c = 1) { return $a; }
try {
    req(1);
} catch (ArgumentCountError $e) {
    caught($e);
}

// eval_cmp NotIdentical: the `!==` operator
var_dump(1 !== 1, 1 !== 2, 1 !== '1', 'a' !== 'a');
