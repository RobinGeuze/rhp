<?php
$n = 0;
register_tick_function(function () use (&$n) { $n++; });
function f() { $a = 1; $b = 2; return $a + $b; }
declare(ticks=2) {
    f();
    function g() { $x = 1; if ($x) { $x++; } return $x; }
    declare(ticks=1);
    g();
    for ($i = 0; $i < 3; $i++) { $i; }
}
echo "after block: $n\n";
g(); f();
echo "end: $n\n";
class C { function m() { declare(ticks=1); return 1; } }
(new C)->m(); $y = 1;
echo "last: $n\n";
