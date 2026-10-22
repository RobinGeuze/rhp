<?php
$counter = 0;
function bump() { global $counter; $counter++; }
bump(); bump(); bump();
echo $counter, "\n";

function tick() { static $n = 0; return ++$n; }
echo tick(), tick(), tick(), "\n";

function acc($x) { static $t = 2 * 5; $t += $x; return $t; }
echo acc(5), " ", acc(3), "\n";

$a = 1; $b = 2;
function swap() { global $a, $b; [$a, $b] = [$b, $a]; }
swap();
echo "$a $b\n";

function init() { global $fresh; $fresh = "hi"; }
init();
echo $fresh, "\n";

$log = [];
function record($msg) { global $log; $log[] = $msg; }
record("first"); record("second");
echo implode(",", $log), "\n";
