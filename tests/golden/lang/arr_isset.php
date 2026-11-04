<?php
$a = ["x" => 1, "y" => null, "z" => 0];
var_dump(isset($a["x"]));
var_dump(isset($a["y"]));
var_dump(isset($a["missing"]));
var_dump(empty($a["z"]));
var_dump(empty($a["x"]));
unset($a["x"]);
var_dump(isset($a["x"]));
var_dump($a);
