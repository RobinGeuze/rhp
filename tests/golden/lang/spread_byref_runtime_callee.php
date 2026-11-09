<?php
// SEND_UNPACK asks a callee known only at run time for its by-reference
// parameters; `new N(f())` with a by-reference constructor parameter is
// SEND_VAR_NO_REF_EX (a notice).
set_error_handler(function ($no, $msg, $file, $line) { echo "E$no: $msg (line $line)\n"; return true; });
function f() { return 1; }
class N {
    function __construct(&$a, $b = 0) { $a = 9; }
    function m(&$a, $b = 0) { $a = 'm'; }
    static function s($x, &...$rest) { foreach ($rest as &$r) { $r = 's'; } }
}
$o = new N($z);
$args = [1, 2]; $o->m(...$args); var_dump($args);
$args = [1, 2, 3]; N::s(...$args); var_dump($args);
$c = 'N'; $args = [1, 2]; new $c(...$args); var_dump($args);
$args = [1, 2]; new N(...$args); var_dump($args);
$args = [1, 2]; new class(...$args) { function __construct(&$a, $b) { $a = 'anon'; } }; var_dump($args);
$m = 'm'; $args = [1]; $o->$m(...$args); var_dump($args);
$fn = [$o, 'm']; $args = [1]; $fn(...$args); var_dump($args);
$args = ['a' => 1]; $o->m(...$args); var_dump($args);
$copy = [1, 2]; $args = $copy; $o->m(...$args); var_dump($copy, $args);
new N(f());
// A class named by a variable: a string or an object.
class S { static function s(&$a) { $a = 's'; } static function t($a) { return $a; } }
$c = 'S'; $args = [1]; $c::s(...$args); var_dump($args);
$args = [1]; var_dump($c::t(...$args));
$c = new S; $args = [1]; $c::s(...$args); var_dump($args);
echo "done\n";
