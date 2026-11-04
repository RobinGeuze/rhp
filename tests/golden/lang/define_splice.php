<?php
define("GREETING", "hello");
define("MAX_ITEMS", 100);
define("PI_ISH", 3.14);
echo GREETING, " ", MAX_ITEMS, " ", PI_ISH, "\n";
var_dump(defined("GREETING"));
var_dump(defined("UNDEFINED_THING"));
echo constant("MAX_ITEMS"), "\n";

function usesDefine() { return defined("GREETING") ? GREETING : "no"; }
echo usesDefine(), "\n";

$a = [1, 2, 3, 4, 5];
$removed = array_splice($a, 1, 2, ["x", "y", "z"]);
echo implode(",", $a), " | ", implode(",", $removed), "\n";

$b = [1, 2, 3, 4, 5];
array_splice($b, 2);
echo implode(",", $b), "\n";

$c = [1, 2, 3];
array_splice($c, 1, 0, ["ins"]);
echo implode(",", $c), "\n";

$d = ['a', 'b', 'c', 'd'];
$r = array_splice($d, -2, 1);
echo implode(",", $d), " | ", implode(",", $r), "\n";

$e = [10, 20, 30];
array_splice($e, 1, 1, 99);
echo implode(",", $e), "\n";
