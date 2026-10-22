<?php
$a = [3, 1, 4, 1, 5, 9, 2, 6];
sort($a); echo implode(",", $a), "\n";
rsort($a); echo implode(",", $a), "\n";

$s = ["banana", "apple", "cherry", "date"];
sort($s); echo implode(",", $s), "\n";

$n = ["10", "9", "2", "1", "20"];
sort($n); echo implode(",", $n), "\n";
sort($n, SORT_STRING); echo implode(",", $n), "\n";

$assoc = ["d" => 4, "a" => 1, "c" => 3, "b" => 2];
asort($assoc); echo implode(",", array_keys($assoc)), "|", implode(",", $assoc), "\n";
arsort($assoc); echo implode(",", array_keys($assoc)), "\n";
ksort($assoc); echo implode(",", array_keys($assoc)), "\n";
krsort($assoc); echo implode(",", array_keys($assoc)), "\n";

$people = [["n"=>"Bob","a"=>30], ["n"=>"Al","a"=>25], ["n"=>"Cy","a"=>35]];
usort($people, fn($x, $y) => $x["a"] <=> $y["a"]);
echo implode(",", array_column($people, "n")), "\n";

$m = ["banana"=>3, "apple"=>1, "cherry"=>2];
uasort($m, fn($x, $y) => $x <=> $y);
echo implode(",", array_keys($m)), "\n";
uksort($m, fn($x, $y) => strcmp($x, $y));
echo implode(",", array_keys($m)), "\n";
