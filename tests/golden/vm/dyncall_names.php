<?php
// Calls through a name in a variable: case, a leading backslash, a name
// longer than the lookup buffer, an undefined one.
function Twice($x) { return $x * 2; }
foreach (['twice', 'TWICE', '\twice', '\\\\twice', str_repeat('a', 70)] as $n) {
    try { echo $n(2), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { echo call_user_func($n, 3), "\n"; } catch (Error $e) { echo $e->getMessage(), "\n"; }
    echo is_callable($n) ? "callable\n" : "not\n";
}
$f = 'twice'; echo $f(f2()), "\n";
function f2() { return 5; }
