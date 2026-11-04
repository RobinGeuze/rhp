<?php
$x = "a";
var_dump(in_array($x, ["a"], null));
var_dump(in_array($x, ["a"], NAN));
$n = null;
var_dump(in_array($x, ["a"], $n));
$n = NAN;
var_dump(in_array($x, ["a"], $n));
var_dump(in_array($x, ["1"], null));
