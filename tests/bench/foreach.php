<?php
$a = range(1, 1000); $o = new ArrayIterator($a); $s = 0;
for ($i = 0; $i < 3000; $i++) { foreach ($a as $k => $v) { $s += $v; } }
for ($i = 0; $i < 1000; $i++) { foreach ($a as &$v) { $s += $v; } unset($v); }
for ($i = 0; $i < 100; $i++) { foreach ($o as $v) { $s += $v; } }
function g() { for ($i = 0; $i < 1000; $i++) yield $i; }
for ($i = 0; $i < 300; $i++) { foreach (g() as $v) { $s += $v; } }
echo $s, "\n";
