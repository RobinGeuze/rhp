<?php
// A typed variadic parameter's errors carry no parameter name; named extras
// are numbered after the positional arguments.
function vt(int ...$n) { var_dump($n); }
function vu($a, int ...$n) { var_dump($n); }
function t($f) {
    try { $f(); } catch (TypeError $e) { echo preg_replace('/, called in.*/', '', $e->getMessage()), "\n"; }
}
t(fn () => vt(1, "z"));
t(fn () => vt(1, n: "z"));
t(fn () => vt(1, x: 1, y: "z"));
t(fn () => vt(x: 1, y: "z"));
t(fn () => vu(1, 2, 3, x: 1, y: "z"));
t(fn () => vu(a: 1, y: "z"));
vu(1, 2, x: "3");
// a Stringable into a typed by-reference parameter is converted in place
function st(string &$x) {}
$o = new class { function __toString(): string { return "S"; } };
st($o);
var_dump($o);
