<?php
[$a, $b, $c] = [1, 2, 3];
echo "$a $b $c\n";
list($x, , $z) = [10, 20, 30];
echo "$x $z\n";
["name" => $n, "age" => $ag] = ["name" => "Bob", "age" => 42];
echo "$n is $ag\n";
[[$p, $q], [$r, $s]] = [[1, 2], [3, 4]];
echo "$p$q$r$s\n";
foreach ([[1, "one"], [2, "two"]] as [$num, $word]) {
    echo "$num=$word ";
}
echo "\n";
