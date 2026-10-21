<?php
$a = [];
$a[0] = "zero";
$a[1] = "one";
$a[] = "two";
$a["key"] = "val";
var_dump($a);
$b = [1, 2, 3];
$b[1] = 99;
var_dump($b);
$g = [];
$g[2][3] = "deep";
var_dump($g);
