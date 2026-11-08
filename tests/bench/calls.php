<?php
function add($a, $b) { return $a + $b; }
class C { public function m($x) { return $x + 1; } public static function s($x) { return $x * 2; } }
$c = new C; $t = 0;
for ($i = 0; $i < 3000000; $i++) { $t = add($t, $i); $t = $c->m($t); $t = C::s($t) & 0xffff; }
echo $t, "\n";
