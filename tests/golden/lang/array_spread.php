<?php
$a = [1, 2, 3];
$b = [0, ...$a, 4];
var_dump($b);
$c = ["x" => 1];
$d = [...$c, "y" => 2];
var_dump($d);
