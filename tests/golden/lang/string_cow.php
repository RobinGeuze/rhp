<?php
// Copy-on-write strings: writes through one variable never show in another.
$a = "hello";
$b = $a;
$a .= " world";
$a[0] = 'H';
var_dump($a, $b);

// Appends in a loop, with a copy taken midway.
$s = '';
for ($i = 0; $i < 3000; $i++) {
    $s .= chr(65 + $i % 26);
    if ($i == 1500) $mid = $s;
}
var_dump(strlen($s), strlen($mid), md5($s), md5($mid));
$s .= $s;
var_dump(strlen($s), substr($s, 2998, 6));

// Offset writes past the end pad with spaces.
$p = "ab";
$q = $p;
$p[5] = 'z';
var_dump($p, $q);

// A string used as an array key, then changed.
$k = "key";
$arr = [$k => 1];
$k .= "2";
$arr[$k] = 2;
var_dump($arr, array_key_exists("key", $arr), isset($arr["key2"]));
$num = "12";
$arr2 = [$num => 'int key', "012" => 'string key'];
var_dump($arr2);

// Casts, case changes and implode of shared strings.
$c = "MiXeD";
$d = (string)$c;
$u = strtoupper($c);
$l = strtolower($c);
var_dump($c, $d, $u, $l, (string)-42, (string)PHP_INT_MIN, "n" . PHP_INT_MIN . "/" . -7 . 0);
var_dump(implode(", ", [$c, $u, 3, 1.5, null, true]), implode([]), implode("-", ["only"]));

// Long strings built by builtins (owned buffers taken over).
$big = str_repeat("0123456789", 1000);
$big2 = $big;
$big2 .= "!";
var_dump(strlen($big), strlen($big2), md5(strrev($big)), substr($big2, -3));
$parts = explode("5", $big);
var_dump(count($parts), $parts[1], strlen(implode("5", $parts)) === strlen($big));
