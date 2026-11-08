<?php
function f() { static $n = 0; return ++$n; }
class C { public function m() { static $k = 0; return ++$k; } }
$o = new C;
$t = 0;
for ($i = 0; $i < 3000000; $i++) { $t += f() + $o->m(); }
echo $t, "\n";
