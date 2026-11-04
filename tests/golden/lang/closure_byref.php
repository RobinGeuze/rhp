<?php
$count = 0;
$inc = function() use (&$count) { $count++; };
$inc(); $inc(); $inc();
echo $count, "\n";

$total = 0;
foreach ([1, 2, 3, 4] as $x) (function($v) use (&$total) { $total += $v; })($x);
echo $total, "\n";

$log = [];
$label = "e";
$rec = function($m) use (&$log, $label) { $log[] = "$label:$m"; };
$rec("a"); $label = "changed"; $rec("b");
echo implode(",", $log), "\n";

function makeCounter() { $n = 0; return function() use (&$n) { return ++$n; }; }
$c = makeCounter(); $d = makeCounter();
echo $c(), $c(), $d(), $c(), "\n";

$acc = [];
$p = function($v) use (&$acc) { $acc[] = $v; };
$p(10); $p(20);
echo implode(",", $acc), "\n";
