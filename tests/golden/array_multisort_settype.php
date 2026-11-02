<?php
// array_change_key_case, array_multisort, settype, json_validate.
$k = ['a' => 1, 'B' => 2, 5 => 3, 'Ä' => 4, 'mIx' => 5];
var_dump(array_change_key_case($k), array_change_key_case($k, CASE_UPPER), array_change_key_case(['a' => 1, 'A' => 2]));

$d1 = [3, 1, 2, 1];
$d2 = ['c', 'a', 'b', 'z'];
var_dump(array_multisort($d1, $d2), $d1, $d2);
$d1 = [3, 1, 2];
$d2 = ['x' => 'c', 'y' => 'a', 'z' => 'b'];
array_multisort($d1, SORT_DESC, $d2);
var_dump($d1, $d2);
$data = [10, 100, 100, 0];
$data2 = [1, 3, 2, 4];
array_multisort($data, SORT_ASC, SORT_STRING, $data2, SORT_DESC);
var_dump($data, $data2);
$ar = ['10', 9, 2, '1'];
array_multisort($ar);
var_dump($ar);
$names = ['b' => 'Bob', 'a' => 'alice', 7 => 'Carol'];
array_multisort($names, SORT_STRING | SORT_FLAG_CASE);
var_dump($names);
$e = [];
var_dump(array_multisort($e));
foreach ([
    function () { $x = [1, 2]; $y = [1]; array_multisort($x, $y); },
    function () { $x = [1]; array_multisort($x, SORT_ASC, SORT_DESC); },
    function () { $x = [1]; array_multisort($x, 99); },
    function () { $x = [1]; array_multisort($x, 'nope'); },
] as $f) {
    try { $f(); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}

$v = "12abc"; var_dump(settype($v, "integer"), $v);
$v = "1.5"; settype($v, "float"); var_dump($v);
$v = 1; settype($v, "array"); var_dump($v);
$v = null; settype($v, "string"); var_dump($v);
$v = 5; settype($v, "bool"); var_dump($v);
$v = [1, "k" => 2]; settype($v, "object"); print_r($v); echo "\n";
$v = 3; settype($v, "null"); var_dump($v);
$v = 3.7; settype($v, "INT"); var_dump($v);
foreach (['foo', 'resource'] as $t) {
    $v = 1;
    try { settype($v, $t); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}

var_dump(json_validate('{"a":1}'), json_validate('{'), json_last_error(), json_last_error_msg());
var_dump(json_validate('[[1]]', 1), json_last_error_msg(), json_validate(''), json_last_error(), json_validate(' [1, 2] '), json_last_error());
var_dump(json_validate("\"\xff\""), json_last_error_msg(), json_validate("\"\xff\"", 512, JSON_INVALID_UTF8_IGNORE));
foreach ([fn() => json_validate('1', 0), fn() => json_validate('1', 512, JSON_BIGINT_AS_STRING)] as $f) {
    try { $f(); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
