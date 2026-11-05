<?php
$s = [1, 2, 3];
echo array_push($s, 4, 5), "\n";
echo implode(",", $s), "\n";
echo array_pop($s), "\n";
echo implode(",", $s), "\n";
echo array_shift($s), "\n";
echo implode(",", $s), "\n";
echo array_unshift($s, 0), "\n";
echo implode(",", $s), "\n";
$s[] = 99;
echo implode(",", $s), "\n";
echo implode(",", array_keys($s)), "\n";

$q = ["a"=>1, "b"=>2];
array_push($q, 3);
echo implode(",", array_keys($q)), "\n";

$r = [0, 1, 2];
array_pop($r);
$r[] = 9;
echo implode(",", $r), "|", implode(",", array_keys($r)), "\n";
