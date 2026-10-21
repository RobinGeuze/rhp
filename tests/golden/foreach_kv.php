<?php
$m = ["a" => 1, "b" => 2, "c" => 3];
foreach ($m as $k => $v) {
    echo "$k => $v\n";
}
$nested = [[1, 2], [3, 4]];
foreach ($nested as $row) {
    foreach ($row as $x) {
        echo $x, " ";
    }
    echo "\n";
}
