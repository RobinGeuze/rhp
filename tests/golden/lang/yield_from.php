<?php
function inner() { yield 1; yield 2; yield 3; }
function outer() { yield 0; yield from inner(); yield 4; }
foreach (outer() as $v) echo $v;
echo "\n";

function withKeys() { yield 'a' => 1; yield from ['b' => 2, 'c' => 3]; yield 'd' => 4; }
foreach (withKeys() as $k => $v) echo "$k=$v ";
echo "\n";

function inner2() { yield 1; yield 2; return 100; }
function outer2() { $r = yield from inner2(); echo "[inner=$r]"; yield $r; }
foreach (outer2() as $v) echo "got$v ";
echo "\n";

function nest() { yield from [10, 20]; yield from inner(); }
foreach (nest() as $v) echo $v, " ";
echo "\n";

function manual() { foreach (inner() as $v) { yield $v * 10; } echo "[done]"; }
foreach (manual() as $v) echo "$v ";
echo "\n";
